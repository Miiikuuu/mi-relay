package io.mirelay.android

import android.content.Context
import android.graphics.Bitmap
import android.net.Uri
import android.util.LruCache
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withContext
import java.io.File
import java.util.UUID

internal data class AlbumPhoto(
    val key: String,
    val name: String,
    val path: String,
    val size: Long?,
    val modified: Long?,
    val source: SourceFile? = null,
    val tree: String? = null,
    val generation: String = "",
    val transfer: Transfer? = null,
)

/** Presentation only. No database writes, hashing, enqueueing or sync filtering. */
internal object AlbumPhotos {
    fun source(tree: String, files: List<SourceFile>, generation: String): List<AlbumPhoto> = files
        .filter { FileFilter.isImageName(it.name) }
        .map { AlbumPhoto(it.documentId, it.name, it.relativePath, it.size, it.modified, it, tree, generation) }
        .sortedWith(compareByDescending<AlbumPhoto> { it.modified?.takeIf { date -> date > 0 } ?: Long.MIN_VALUE }
            .thenBy { it.path }.thenBy { it.key })

    fun transfers(files: List<Transfer>): List<AlbumPhoto> = files
        .filter { !it.superseded && FileFilter.isImageName(it.name) }
        .sortedWith(compareByDescending<Transfer> { it.createdAt }.thenBy { it.id })
        .map { AlbumPhoto(it.id, it.name, it.relativePath ?: it.name, it.size, it.createdAt, transfer = it) }

}

internal class AlbumLibrary(private val context: Context) {
    companion object { const val MAX_PREVIEW_BYTES = 32L * 1024 * 1024 }
    private val permits = Semaphore(2)
    private val source = DirectorySource(context.contentResolver)
    private val scratch = File(context.cacheDir, "album-scratch")
    private val scratchLock = Any()
    private var cleaned = false
    private val cache = object : LruCache<String, Bitmap>(12 * 1024 * 1024) {
        override fun sizeOf(key: String, value: Bitmap) = value.byteCount
        // Visible composables may still reference evicted bitmaps: never recycle.
    }

    private suspend fun <T> read(block: (AutoSession) -> T): T = coroutineScope {
        val session = AutoSession(15_000)
        // Cancel a blocked provider query/stream immediately when the screen or
        // tile leaves composition, not only after the deadline has elapsed.
        val cancellation = launch(start = CoroutineStart.UNDISPATCHED) {
            try { awaitCancellation() }
            finally { withContext(NonCancellable + Dispatchers.IO) { session.cancel() } }
        }
        try { withContext(Dispatchers.IO) { block(session) } }
        finally {
            cancellation.cancel()
            withContext(NonCancellable + Dispatchers.IO) { session.close() }
        }
    }

    suspend fun scan(tree: String): List<AlbumPhoto> = read { session ->
        val files = source.snapshot(Uri.parse(tree), session, strict = true).files
        AlbumPhotos.source(tree, files, UUID.randomUUID().toString())
    }

    suspend fun load(photo: AlbumPhoto, store: RelayStore, target: Int): Bitmap? {
        if (photo.source == null) return PhotoThumbnails.load(store, photo.key, target)
        require(target in 1..1024)
        return permits.withPermit {
            read { session ->
                val tree = Uri.parse(requireNotNull(photo.tree))
                source.requirePermission(tree) // Includes cache hits after revocation.
                val key = "${photo.tree}:${photo.key}:${photo.generation}:$target"
                cache.get(key) ?: decode(photo.source, tree, target, session)?.also { cache.put(key, it) }
            }
        }
    }

    private fun decode(file: SourceFile, tree: Uri, target: Int, session: AutoSession): Bitmap? {
        if (file.size == null || file.size !in 1..MAX_PREVIEW_BYTES) return null
        check(source.metadata(tree, file, session, true).fingerprint == file.fingerprint) { "Refresh photos to read the changed file." }
        synchronized(scratchLock) {
            check(scratch.isDirectory || scratch.mkdirs())
            if (!cleaned) {
                // Only our disposable scratch copies from a previous process.
                scratch.listFiles()?.filter { it.name.startsWith("preview-") && it.name.endsWith(".tmp") }?.forEach { it.delete() }
                cleaned = true
            }
        }
        val temp = File.createTempFile("preview-", ".tmp", scratch)
        try {
            requireNotNull(context.contentResolver.openAssetFileDescriptor(file.uri, "r", session.signal)).use { descriptor ->
                descriptor.createInputStream().use { input ->
                    session.track(input)
                    try {
                        temp.outputStream().use { output ->
                            val buffer = ByteArray(64 * 1024)
                            var size = 0L
                            while (true) {
                                session.check()
                                val count = input.read(buffer)
                                if (count < 0) break
                                check(count > 0) { "Source stopped making progress." }
                                size += count
                                check(size <= file.size && size <= MAX_PREVIEW_BYTES) { "Image changed while reading." }
                                output.write(buffer, 0, count)
                            }
                            check(size == file.size) { "Image changed while reading." }
                        }
                    } finally { session.untrack(input) }
                }
            }
            session.check()
            source.requirePermission(tree)
            check(source.metadata(tree, file, session, true).fingerprint == file.fingerprint)
            return PhotoThumbnails.decode(temp, target)
        } finally { temp.delete() }
    }
}

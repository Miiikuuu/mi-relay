package io.mirelay.android

import android.graphics.Bitmap
import android.util.AtomicFile
import java.io.ByteArrayOutputStream
import java.io.File
import java.util.UUID

/** Disposable derived images, never originals. Shared lock also covers eviction
 * versus reads and multiple RelayStore instances in the same process. */
internal class PhotoPreviewCache(
    private val root: File,
    private val maxBytes: Long = 32L * 1024 * 1024,
    private val maxEntries: Int = 128,
) {
    companion object { private val lock = Any() }

    private fun file(id: String): File {
        require(UUID.fromString(id).toString() == id) { "Invalid transfer ID." }
        return File(root, "$id.png")
    }

    /** Best effort: storage/codec failures must never fail a successful upload. */
    fun prepare(id: String, payload: File): Boolean = synchronized(lock) {
        try {
            val destination = file(id)
            val bitmap = PhotoThumbnails.decode(payload, 1024) ?: return false
            val bytes = try {
                ByteArrayOutputStream().use { output ->
                    if (!bitmap.compress(Bitmap.CompressFormat.PNG, 100, output)) return false
                    output.toByteArray()
                }
            } finally { bitmap.recycle() }
            if (bytes.size > 5 * 1024 * 1024 || bytes.size > maxBytes || maxEntries < 1) return false
            if (!root.isDirectory && !root.mkdirs()) return false
            // AtomicFile's unfinished .new files are disposable after process death.
            root.listFiles()?.filter { it.name.endsWith(".new") }?.forEach { it.delete() }
            val entries = root.listFiles()?.filter { it.isFile && it != destination }
                ?.sortedBy { it.lastModified() } ?: return false
            var total = entries.sumOf { it.length() }
            var count = entries.size
            for (entry in entries) {
                if (total + bytes.size <= maxBytes && count < maxEntries) break
                val length = entry.length()
                if (entry.delete()) { total -= length; count-- }
            }
            if (total + bytes.size > maxBytes || count >= maxEntries) return false
            val atomic = AtomicFile(destination)
            val output = atomic.startWrite()
            try {
                output.write(bytes)
                atomic.finishWrite(output)
            } catch (error: Exception) {
                atomic.failWrite(output)
                throw error
            }
            true
        } catch (_: Exception) { false }
    }

    fun load(id: String, target: Int): Bitmap? = synchronized(lock) {
        try {
            val source = file(id)
            PhotoThumbnails.decode(source, target)?.also {
                source.setLastModified(System.currentTimeMillis())
            }
        } catch (_: Exception) { null }
    }
}

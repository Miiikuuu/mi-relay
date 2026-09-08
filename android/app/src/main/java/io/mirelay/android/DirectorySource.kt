package io.mirelay.android

import android.content.ContentResolver
import android.database.Cursor
import android.net.Uri
import android.os.CancellationSignal
import android.os.OperationCanceledException
import android.provider.DocumentsContract
import android.provider.DocumentsContract.Document
import java.io.Closeable
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

/** One bounded operation, with no surviving polling thread after close(). */
internal class AutoSession(timeoutMillis: Long = 90_000, private val stopped: () -> Boolean = { false }) : Closeable {
    var directoryProgress: Boolean = false
    val signal = CancellationSignal()
    private val stream = AtomicReference<Closeable?>()
    private val timer = Executors.newSingleThreadScheduledExecutor { task -> Thread(task, "auto-deadline").apply { isDaemon = true } }
    private val deadline = timer.schedule({ cancel() }, timeoutMillis, TimeUnit.MILLISECONDS)
    fun check() { if (stopped()) cancel(); signal.throwIfCanceled() }
    fun track(value: Closeable) { stream.set(value); check() }
    fun untrack(value: Closeable) { stream.compareAndSet(value, null) }
    fun cancel() {
        try { stream.getAndSet(null)?.close() } catch (_: Exception) { }
        signal.cancel()
    }
    override fun close() { deadline.cancel(false); cancel(); timer.shutdownNow() }
}

internal data class DirectorySnapshot(val name: String, val files: List<SourceFile>)

internal class DirectorySource(private val resolver: ContentResolver) {
    companion object {
        private val columns = arrayOf(Document.COLUMN_DOCUMENT_ID, Document.COLUMN_DISPLAY_NAME, Document.COLUMN_MIME_TYPE, Document.COLUMN_SIZE, Document.COLUMN_LAST_MODIFIED, Document.COLUMN_FLAGS)
        fun validate(tree: Uri) {
            require(tree.scheme == "content" && !tree.authority.isNullOrBlank() && DocumentsContract.isTreeUri(tree)) { "Choose a source directory using the system picker." }
        }
    }
    fun requirePermission(tree: Uri) {
        validate(tree)
        if (resolver.persistedUriPermissions.none { it.uri == tree && it.isReadPermission }) throw SecurityException("Source permission is no longer available.")
    }
    fun snapshot(tree: Uri, session: AutoSession, strict: Boolean = false): DirectorySnapshot {
        requirePermission(tree)
        session.check()
        val root = DocumentsContract.getTreeDocumentId(tree)
        val rootUri = DocumentsContract.buildDocumentUriUsingTree(tree, root)
        val name = query(rootUri, session).use {
            check(it.moveToFirst() && it.text(Document.COLUMN_MIME_TYPE) == Document.MIME_TYPE_DIR) { "Source directory is unavailable." }
            InputRules.filename(it.text(Document.COLUMN_DISPLAY_NAME))
        }
        val directories = ArrayDeque<Triple<String, Int, String>>()
        directories.add(Triple(root, 0, ""))
        val seen = hashSetOf(root)
        val files = mutableListOf<SourceFile>()
        while (directories.isNotEmpty()) {
            session.check()
            val (parent, depth, prefix) = directories.removeFirst()
            require(depth <= 16) { "Source is too deeply nested. Choose a smaller directory." }
            val children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, parent)
            query(children, session).use { cursor ->
                check(!cursor.extras.getBoolean(DocumentsContract.EXTRA_LOADING, false)) { "Source is still loading. Try again when it is available." }
                while (cursor.moveToNext()) {
                    session.check()
                    val id = requireNotNull(cursor.text(Document.COLUMN_DOCUMENT_ID)) { "Source returned an invalid document." }
                    require(id.isNotEmpty() && id.length <= 4096 && seen.add(id)) { "Source returned duplicate or cyclic documents." }
                    require(seen.size <= AutoStore.MAX_ENTRIES + 1) { "Source exceeds 5,000 entries. Choose a smaller directory." }
                    val rawName = cursor.text(Document.COLUMN_DISPLAY_NAME)
                    val name = if (strict) requireNotNull(rawName) { "Source omitted a filename." } else InputRules.filename(rawName)
                    if (strict) { require('/' !in name) { "A filename contains a path separator." }; DirectoryPaths.validate(prefix + name) }
                    if (cursor.text(Document.COLUMN_MIME_TYPE) == Document.MIME_TYPE_DIR) directories.add(Triple(id, depth + 1, "$prefix$name/"))
                    else files.add(cursor.file(tree, id, prefix, strict))
                }
            }
        }
        if (strict) require(files.map { it.relativePath }.distinct().size == files.size) { "Source returned duplicate relative paths." }
        return DirectorySnapshot(name, files.sortedBy { it.relativePath })
    }
    fun metadata(tree: Uri, file: SourceFile, session: AutoSession, strict: Boolean = false): SourceFile {
        session.check()
        // Always use the granted tree, never a provider-supplied absolute URI or path.
        return query(DocumentsContract.buildDocumentUriUsingTree(tree, file.documentId), session).use {
            check(it.moveToFirst() && it.text(Document.COLUMN_DOCUMENT_ID) == file.documentId && it.text(Document.COLUMN_MIME_TYPE) != Document.MIME_TYPE_DIR) { "Source file changed or disappeared." }
            it.file(tree, file.documentId, file.relativePath.substringBeforeLast('/', "").let { prefix -> if (prefix.isEmpty()) "" else "$prefix/" }, strict)
        }
    }
    private fun query(uri: Uri, session: AutoSession): Cursor {
        session.check()
        val cursor = checkNotNull(resolver.query(uri, columns, null, null, null, session.signal)) { "Source did not return a complete listing." }
        try {
            session.check()
            check(!cursor.extras.getBoolean(DocumentsContract.EXTRA_LOADING, false) && cursor.extras.getString(DocumentsContract.EXTRA_ERROR).isNullOrEmpty()) { "Source is still loading or reported an error." }
            return cursor
        } catch (error: Exception) { cursor.close(); throw error }
    }
    private fun Cursor.file(tree: Uri, id: String, prefix: String = "", strict: Boolean = false): SourceFile {
        val virtual = (number(Document.COLUMN_FLAGS) ?: 0L) and Document.FLAG_VIRTUAL_DOCUMENT.toLong() != 0L
        val name = if (strict) requireNotNull(text(Document.COLUMN_DISPLAY_NAME)) else InputRules.filename(text(Document.COLUMN_DISPLAY_NAME))
        return SourceFile(id, DocumentsContract.buildDocumentUriUsingTree(tree, id), name,
            if (virtual) null else number(Document.COLUMN_SIZE), number(Document.COLUMN_LAST_MODIFIED), prefix + name)
    }

    /** Hash every file, including already uploaded files whose provider timestamp
     * may be unchanged. Never treat a partial scan as an initialization baseline. */
    fun hashes(tree: Uri, files: List<SourceFile>, session: AutoSession): List<HashedSource> {
        session.check()
        DirectoryScanException.validate(files)
        return files.map { file ->
            DirectoryPaths.validate(file.relativePath)
            check(metadata(tree, file, session, true).fingerprint == file.fingerprint) { "Source changed; refresh the preview." }
            val digest = java.security.MessageDigest.getInstance("SHA-256")
            var size = 0L
            requireNotNull(resolver.openAssetFileDescriptor(file.uri, "r", session.signal)).use { descriptor ->
                descriptor.createInputStream().use { input ->
                    session.track(input)
                    try {
                        val buffer = ByteArray(64 * 1024)
                        while (true) {
                            session.check(); val count = input.read(buffer)
                            if (count < 0) break
                            check(count > 0) { "Source stopped making progress." }
                            size += count; require(size <= requireNotNull(file.size)) { "Source grew while scanning." }
                            digest.update(buffer, 0, count)
                        }
                    } finally { session.untrack(input) }
                }
            }
            check(size == file.size && metadata(tree, file, session, true).fingerprint == file.fingerprint) { "Source changed while hashing; refresh the preview." }
            HashedSource(file, digest.digest().joinToString("") { "%02x".format(it.toInt() and 255) })
        }
    }
    private fun Cursor.text(column: String): String? = getColumnIndex(column).let { if (it < 0 || isNull(it)) null else getString(it) }
    private fun Cursor.number(column: String): Long? = getColumnIndex(column).let { if (it < 0 || isNull(it)) null else getLong(it) }
}

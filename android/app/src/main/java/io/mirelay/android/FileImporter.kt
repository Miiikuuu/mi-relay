package io.mirelay.android

import android.content.ContentResolver
import android.net.Uri
import android.provider.OpenableColumns
import android.system.Os
import android.system.OsConstants
import java.io.File
import java.io.FileOutputStream
import java.util.UUID
import java.security.MessageDigest

class FileImporter(private val resolver: ContentResolver, private val store: RelayStore) {
    fun import(uri: Uri, folderId: String): String {
        return FolderWorkGate.work {
            store.requireConnectionOpen(folderId)
            checkNotNull(stage(uri, folderId) { file -> store.addTransfer(file.id, folderId, file.name, file.size); true })
        }
    }
    internal fun stage(uri: Uri, folderId: String, session: AutoSession? = null, exactName: String? = null, commit: (StagedFile) -> Boolean): String? {
        return FolderWorkGate.work { stageLocked(uri, folderId, session, exactName, commit) }
    }
    private fun stageLocked(uri: Uri, folderId: String, session: AutoSession?, exactName: String?, commit: (StagedFile) -> Boolean): String? {
        require(UUID.fromString(folderId).toString() == folderId)
        store.requireConnectionOpen(folderId)
        exactName?.let { DirectoryPaths.validate(it); require('/' !in it) }
        require(uri.scheme == "content") { "Choose or share a file using Android's file provider." }
        var name: String? = null
        session?.check()
        resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null, session?.signal)?.use {
            if (it.moveToFirst()) {
                val nameColumn = it.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                if (nameColumn >= 0) name = it.getString(nameColumn)
                val sizeColumn = it.getColumnIndex(OpenableColumns.SIZE)
                if (sizeColumn >= 0 && !it.isNull(sizeColumn)) require(it.getLong(sizeColumn) <= MAX_FILE_BYTES) { "Each file must be at most 100 MiB." }
            }
        }
        val id = UUID.randomUUID().toString()
        val dir = store.directory(id)
        check(dir.mkdirs()) { "Cannot create private staging directory." }
        val partial = File(dir, "payload.part")
        val payload = File(dir, "payload")
        var recorded = false
        try {
            // Ownership must survive a process death before the queue commit.
            // No payload bytes are written until this marker is durable.
            FileOutputStream(File(dir, "folder-owner")).use { it.write(folderId.toByteArray(Charsets.US_ASCII)); it.fd.sync() }
            for (directory in listOfNotNull(dir, dir.parentFile, dir.parentFile?.parentFile)) {
                val fd = Os.open(directory.path, OsConstants.O_RDONLY, 0)
                try { Os.fsync(fd) } finally { Os.close(fd) }
            }
            var size = 0L
            val digest = MessageDigest.getInstance("SHA-256")
            requireNotNull(resolver.openAssetFileDescriptor(uri, "r", session?.signal)) { "Cannot open shared file. Share it again." }.use { descriptor ->
                descriptor.createInputStream().use { input ->
                    session?.track(input)
                    try {
                        FileOutputStream(partial).use { output ->
                            val buffer = ByteArray(64 * 1024)
                            while (true) {
                                if (Thread.currentThread().isInterrupted) throw InterruptedException("Import interrupted")
                                session?.check()
                                val count = input.read(buffer)
                                if (count < 0) break
                                check(count > 0) { "File provider stopped making progress." }
                                size += count
                                require(size <= MAX_FILE_BYTES) { "Each file must be at most 100 MiB." }
                                output.write(buffer, 0, count)
                                digest.update(buffer, 0, count)
                                session?.progress()
                            }
                            require(size > 0) { "Empty files are not supported by this server." }
                            output.fd.sync()
                        }
                    } finally { session?.untrack(input) }
                }
            }
            check(partial.renameTo(payload)) { "Cannot commit staged file." }
            for (directory in listOfNotNull(dir, dir.parentFile, dir.parentFile?.parentFile)) {
                check(directory.isDirectory) { "Staging directory is unavailable." }
                val fd = Os.open(directory.path, OsConstants.O_RDONLY, 0)
                try { Os.fsync(fd) } finally { Os.close(fd) }
            }
            session?.check()
            val hash = digest.digest().joinToString("") { "%02x".format(it.toInt() and 255) }
            recorded = commit(StagedFile(id, exactName ?: InputRules.filename(name), size, hash))
            if (recorded) store.refresh()
            return if (recorded) id else null
        } finally {
            if (!recorded) {
                partial.delete(); payload.delete()
                if (!partial.exists() && !payload.exists()) File(dir, "folder-owner").delete()
                dir.delete()
            }
        }
    }
}

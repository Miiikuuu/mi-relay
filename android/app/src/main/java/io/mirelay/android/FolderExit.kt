package io.mirelay.android

import android.content.ContentValues
import android.content.Intent
import android.net.Uri
import android.system.Os
import android.system.OsConstants
import java.io.File
import java.nio.file.Files

/** Only app-owned copies are deleted. Keep a minimal Folder receipt and its
 * encrypted, now transfer-disabled token until both devices acknowledge. */
internal class FolderExit(private val app: RelayApplication) {
    private val store get() = app.store
    private val connection = FolderConnection(app)

    @Synchronized fun run(id: String, discoverOnly: Boolean = false) {
        val folder = requireNotNull(store.folder(id))
        require(folder.scoped) { "Legacy Folders support local removal only." }
        if (folder.exitPhase == "complete") return
        val token = store.token(id)
        if (discoverOnly && folder.exitPhase == null && !connection.exit(folder, token, "exit_status").getBoolean("requested")) return
        store.writableDatabase.execSQL("UPDATE folders SET exit_phase=COALESCE(exit_phase,'pending'),pairing_state='disconnect_pending' WHERE id=?", arrayOf(id))
        app.auto.stopFolder(id)
        val server = connection.exit(folder, token, "clean_exit")
        check(server.getBoolean("server_cleaned")) { "Server cleanup is still pending." }
        if (store.folder(id)?.exitPhase != "local_cleaned") cleanLocal(id)
        val receipt = connection.exit(folder, token, "ack_exit")
        check(receipt.getBoolean("sender_cleaned")) { "Local cleanup receipt was not confirmed. Retry clean exit." }
        if (receipt.getBoolean("receiver_cleaned")) {
            store.writableDatabase.update("folders", ContentValues().apply {
                put("exit_phase", "complete"); put("pairing_state", "disconnected"); put("token", ""); putNull("verification")
            }, "id=?", arrayOf(id))
        }
        store.refresh()
    }

    // Serialize grant ownership checks with Auto's directory selection/enable
    // operations. A concurrently added Folder must not lose its new grant.
    private fun cleanLocal(id: String) = synchronized(app.auto) { FolderWorkGate.clean {
        val source = store.automatic.source(id)
        val preview = store.directorySync.preview(id)
        val trees = listOfNotNull(source?.treeUri, preview?.tree).distinct()
        // Ownership metadata is kept until all deletes and permission releases
        // succeed, making retries safe after partial filesystem failure.
        for (transfer in store.cleanupTransfers(id)) {
            val directory = store.cleanupDirectory(transfer)
            if (directory.exists()) {
                check(directory.canonicalFile == directory.absoluteFile && directory.isDirectory) { "Unsafe private staging path." }
                // Delete ownership last so interruption cannot orphan payloads.
                for (file in checkNotNull(directory.listFiles()).sortedBy { it.name == "folder-owner" }) removeRegular(file)
                sync(directory)
                check(directory.delete()) { "Private staging directory is still busy." }
                directory.parentFile?.let(::sync)
            }
            store.photoPreviews.remove(transfer)
        }
        for (tree in trees) {
            val shared = store.readableDatabase.rawQuery(
                "SELECT 1 FROM auto_sources WHERE folder_id<>? AND tree_uri=? UNION ALL SELECT 1 FROM directory_previews WHERE folder_id<>? AND tree_uri=?",
                arrayOf(id, tree, id, tree)).use { it.moveToFirst() }
            if (!shared && app.contentResolver.persistedUriPermissions.any { it.uri.toString() == tree }) {
                app.contentResolver.releasePersistableUriPermission(Uri.parse(tree), Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
        }
        val db = store.writableDatabase
        db.beginTransaction()
        try {
            db.delete("transfers", "folder_id=?", arrayOf(id))
            db.delete("auto_sources", "folder_id=?", arrayOf(id))
            db.delete("directory_previews", "folder_id=?", arrayOf(id))
            db.execSQL("UPDATE folders SET exit_phase='local_cleaned' WHERE id=?", arrayOf(id))
            db.setTransactionSuccessful()
        } finally { db.endTransaction() }
        store.refresh()
    } }

    companion object {
        internal fun removeRegular(file: File) {
            check(!Files.isSymbolicLink(file.toPath())) { "Refusing a symbolic-link cleanup target." }
            if (!file.exists()) return
            check(file.canonicalFile == file.absoluteFile && file.isFile) { "Refusing an unexpected cleanup target." }
            check(file.delete()) { "Could not remove an app-owned temporary file. Retry clean exit." }
            file.parentFile?.let(::sync)
        }
        private fun sync(directory: File) {
            val fd = Os.open(directory.path, OsConstants.O_RDONLY or OsConstants.O_NOFOLLOW, 0)
            try { check(OsConstants.S_ISDIR(Os.fstat(fd).st_mode)); Os.fsync(fd) } finally { Os.close(fd) }
        }
    }
}

package io.mirelay.android

import android.content.ContentValues
import android.database.Cursor
import android.database.sqlite.SQLiteDatabase
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.util.UUID

/** Consent, discovery and enqueue receipts live in the SAME database as transfers. */
class AutoStore(private val relay: RelayStore) {
    private val db get() = relay.writableDatabase
    private val mutableSources = MutableStateFlow<List<AutoSource>>(emptyList())
    val sources = mutableSources.asStateFlow()

    companion object {
        internal const val STABLE_MILLIS = 10_000L
        internal const val MAX_ENTRIES = 5_000
        internal const val MAX_HISTORY = 50_000
        internal const val ALLOWED_TRANSFER = "(auto_revision IS NULL OR EXISTS (SELECT 1 FROM auto_sources a WHERE a.folder_id = transfers.folder_id AND a.revision = transfers.auto_revision AND a.enabled = 1))"
        internal fun createTables(db: SQLiteDatabase) {
            db.execSQL("CREATE TABLE auto_sources (folder_id TEXT PRIMARY KEY REFERENCES folders(id) ON DELETE CASCADE, tree_uri TEXT NOT NULL, name TEXT NOT NULL, revision TEXT NOT NULL, enabled INTEGER NOT NULL, unmetered INTEGER NOT NULL, last_scan INTEGER, error TEXT, prepared INTEGER NOT NULL DEFAULT 0)")
            db.execSQL("CREATE TABLE auto_files (folder_id TEXT NOT NULL REFERENCES auto_sources(folder_id) ON DELETE CASCADE, document_id TEXT NOT NULL, fingerprint TEXT NOT NULL, stable_since INTEGER NOT NULL, state TEXT NOT NULL, transfer_id TEXT, error TEXT, PRIMARY KEY(folder_id, document_id))")
            db.execSQL("CREATE TABLE auto_hashes (folder_id TEXT NOT NULL REFERENCES folders(id) ON DELETE CASCADE, sha256 TEXT NOT NULL, transfer_id TEXT NOT NULL REFERENCES transfers(id) ON DELETE CASCADE, PRIMARY KEY(folder_id, sha256))")
            db.execSQL("CREATE INDEX auto_transfers ON transfers(folder_id, auto_revision, status)")
        }
    }

    internal fun refresh() {
        mutableSources.value = db.rawQuery("SELECT a.*, CASE WHEN a.directory_sync=1 THEN (SELECT COUNT(*) FROM directory_files d WHERE d.folder_id=a.folder_id AND d.state='PENDING') ELSE (SELECT COUNT(*) FROM auto_files f WHERE f.folder_id=a.folder_id AND f.state='PENDING') END AS waiting, CASE WHEN a.directory_sync=1 THEN (SELECT COUNT(*) FROM directory_files d WHERE d.folder_id=a.folder_id AND d.error IS NOT NULL) ELSE (SELECT COUNT(*) FROM auto_files f WHERE f.folder_id=a.folder_id AND f.error IS NOT NULL) END AS skipped FROM auto_sources a ORDER BY folder_id", null).use { cursor ->
            buildList { while (cursor.moveToNext()) add(cursor.source()) }
        }
    }
    fun source(folderId: String): AutoSource? {
        refresh()
        return sources.value.find { it.folderId == folderId }
    }
    internal fun active(folder: String, revision: String): Boolean = db.rawQuery(
        "SELECT 1 FROM auto_sources WHERE folder_id=? AND revision=? AND enabled=1", arrayOf(folder, revision)
    ).use { it.moveToFirst() }
    internal fun permits(transfer: Transfer) = transfer.autoRevision?.let { active(transfer.folderId, it) } ?: true

    /** Baseline and enabling commit together. Failure never leaves a partially enabled source. */
    internal fun enable(folder: String, tree: String, name: String, unmetered: Boolean, files: List<SourceFile>, now: Long,
        includeExisting: Boolean = false, startImmediately: Boolean = true): AutoSource {
        require(files.size <= MAX_ENTRIES && files.map { it.documentId }.distinct().size == files.size)
        val revision = UUID.randomUUID().toString()
        transaction {
            require(source(folder)?.directorySync != true) { "This Folder is initialized for directory sync. Resume it without resetting its baseline." }
            check(!db.rawQuery("SELECT 1 FROM auto_sources WHERE folder_id=? AND enabled=1", arrayOf(folder)).use { it.moveToFirst() }) { "Pause Auto before changing its source." }
            db.delete("auto_files", "folder_id=?", arrayOf(folder))
            // UPDATE instead of REPLACE preserves foreign-key semantics.
            val values = ContentValues().apply {
                put("tree_uri", tree); put("name", name); put("revision", revision); put("enabled", if (startImmediately) 1 else 0)
                put("prepared", if (startImmediately) 0 else 1)
                put("unmetered", if (unmetered) 1 else 0); put("last_scan", now); putNull("error")
            }
            if (db.update("auto_sources", values, "folder_id=?", arrayOf(folder)) == 0) {
                values.put("folder_id", folder); db.insertOrThrow("auto_sources", null, values)
            }
            for (file in files) insertSeen(folder, file, now, if (includeExisting) "PENDING" else "BASELINE")
        }
        relay.refresh()
        return requireNotNull(source(folder))
    }

    internal fun startPrepared(folder: String, unmetered: Boolean): AutoSource {
        check(db.update("auto_sources", ContentValues().apply { put("enabled", 1); put("prepared", 0); put("unmetered", if (unmetered) 1 else 0) },
            "folder_id=? AND enabled=0 AND prepared=1", arrayOf(folder)) == 1) { "Source is no longer prepared." }
        relay.refresh()
        return requireNotNull(source(folder))
    }

    /** Pausing invalidates both in-flight scans and upload ownership before cancellation. */
    internal fun disable(folder: String, reason: String? = null) {
        transaction {
            db.update("auto_sources", ContentValues().apply { put("enabled", 0); put("error", reason) }, "folder_id=?", arrayOf(folder))
            db.update("transfers", ContentValues().apply { put("status", TransferStatus.PAUSED.name); put("work_id", "") },
                "folder_id=? AND auto_revision IS NOT NULL AND status IN (?,?)", arrayOf(folder, TransferStatus.QUEUED.name, TransferStatus.UPLOADING.name))
        }
        relay.refresh()
    }

    internal fun makeManual(id: String) {
        if (relay.transfer(id)?.relativePath != null) {
            require(relay.transfer(id)?.let(::permits) == true) { "Resume directory sync before retrying this file." }
            return
        }
        db.update("transfers", ContentValues().apply { putNull("auto_revision"); put("auto_unmetered", 0) },
            "id=? AND status IN (?,?)", arrayOf(id, TransferStatus.PAUSED.name, TransferStatus.FAILED.name))
    }

    /** Only identities absent from the complete baseline can become candidates. */
    internal fun observe(source: AutoSource, files: List<SourceFile>, now: Long): List<SourceFile> {
        val ready = mutableListOf<SourceFile>()
        transaction {
            check(active(source.folderId, source.revision)) { "Auto is no longer enabled." }
            val count = db.rawQuery("SELECT COUNT(*) FROM auto_files WHERE folder_id=?", arrayOf(source.folderId)).use { it.moveToFirst(); it.getInt(0) }
            var added = 0
            for (file in files) {
                db.rawQuery("SELECT fingerprint, stable_since, state, error FROM auto_files WHERE folder_id=? AND document_id=?", arrayOf(source.folderId, file.documentId)).use { cursor ->
                    if (!cursor.moveToFirst()) {
                        check(count + ++added <= MAX_HISTORY) { "Auto history is full. Pause and re-enable with a fresh baseline." }
                        insertSeen(source.folderId, file, now, "PENDING")
                    } else if (cursor.getString(2) == "PENDING") {
                        if (cursor.getString(0) != file.fingerprint || now < cursor.getLong(1) || cursor.getString(3) == "Source file is unavailable.") {
                            db.update("auto_files", ContentValues().apply {
                                put("fingerprint", file.fingerprint); put("stable_since", now); putNull("error")
                            }, "folder_id=? AND document_id=?", arrayOf(source.folderId, file.documentId))
                        } else if (file.eligible && now - cursor.getLong(1) >= STABLE_MILLIS) ready.add(file)
                    }
                    if (!file.eligible) db.update("auto_files", ContentValues().apply {
                        put("error", "Empty, over 100 MiB, or missing size/modification metadata. Use manual sending if appropriate.")
                    }, "folder_id=? AND document_id=? AND state='PENDING'", arrayOf(source.folderId, file.documentId))
                }
            }
            // A disappeared pending identity must be observed stably again if it returns.
            val present = files.map { it.documentId }.toHashSet()
            db.rawQuery("SELECT document_id FROM auto_files WHERE folder_id=? AND state='PENDING'", arrayOf(source.folderId)).use { cursor ->
                while (cursor.moveToNext()) if (cursor.getString(0) !in present) {
                    db.update("auto_files", ContentValues().apply { put("stable_since", now); put("error", "Source file is unavailable.") },
                        "folder_id=? AND document_id=?", arrayOf(source.folderId, cursor.getString(0)))
                }
            }
        }
        return ready
    }

    /** File ledger, hash de-duplication and queue insertion are one atomic commit. */
    internal fun commit(source: AutoSource, file: SourceFile, staged: StagedFile): Boolean {
        var created = false
        transaction {
            if (!active(source.folderId, source.revision)) return@transaction
            val eligible = db.rawQuery("SELECT 1 FROM auto_files WHERE folder_id=? AND document_id=? AND fingerprint=? AND state='PENDING'",
                arrayOf(source.folderId, file.documentId, file.fingerprint)).use { it.moveToFirst() }
            if (!eligible) return@transaction
            val duplicate = db.rawQuery("SELECT transfer_id FROM auto_hashes WHERE folder_id=? AND sha256=?", arrayOf(source.folderId, staged.sha256)).use { if (it.moveToFirst()) it.getString(0) else null }
            if (duplicate == null) {
                relay.addTransfer(staged.id, source.folderId, staged.name, staged.size)
                db.update("transfers", ContentValues().apply { put("auto_revision", source.revision); put("auto_unmetered", if (source.unmetered) 1 else 0) }, "id=?", arrayOf(staged.id))
                db.insertOrThrow("auto_hashes", null, ContentValues().apply { put("folder_id", source.folderId); put("sha256", staged.sha256); put("transfer_id", staged.id) })
                created = true
            }
            db.update("auto_files", ContentValues().apply {
                put("state", if (created) "STAGED" else "DUPLICATE"); put("transfer_id", duplicate ?: staged.id); putNull("error")
            }, "folder_id=? AND document_id=?", arrayOf(source.folderId, file.documentId))
        }
        // The caller must learn that the payload is durable BEFORE publishing UI state.
        // A refresh failure here must never cause FileImporter to delete a queued payload.
        return created
    }

    internal fun scanResult(source: AutoSource, now: Long, error: String? = null) {
        db.update("auto_sources", ContentValues().apply { put("last_scan", now); put("error", error) },
            "folder_id=? AND revision=? AND enabled=1", arrayOf(source.folderId, source.revision))
        relay.refresh()
    }
    internal fun fileError(source: AutoSource, file: SourceFile, message: String, now: Long) {
        if (!active(source.folderId, source.revision)) return
        db.update("auto_files", ContentValues().apply { put("error", message); put("stable_since", now) },
            "folder_id=? AND document_id=? AND fingerprint=? AND state='PENDING' AND EXISTS (SELECT 1 FROM auto_sources a WHERE a.folder_id=auto_files.folder_id AND a.revision=? AND a.enabled=1)", arrayOf(source.folderId, file.documentId, file.fingerprint, source.revision))
    }
    internal fun needsSettle(source: AutoSource): Boolean = active(source.folderId, source.revision) && db.rawQuery(
        "SELECT 1 FROM auto_files WHERE folder_id=? AND state='PENDING' AND error IS NULL LIMIT 1", arrayOf(source.folderId)
    ).use { it.moveToFirst() }
    private fun insertSeen(folder: String, file: SourceFile, now: Long, state: String) {
        db.insertOrThrow("auto_files", null, ContentValues().apply {
            put("folder_id", folder); put("document_id", file.documentId); put("fingerprint", file.fingerprint)
            put("stable_since", now); put("state", state)
        })
    }
    private inline fun transaction(block: () -> Unit) {
        db.beginTransaction()
        try { block(); db.setTransactionSuccessful() } finally { db.endTransaction() }
    }
    private fun Cursor.source() = AutoSource(
        getString(getColumnIndexOrThrow("folder_id")), getString(getColumnIndexOrThrow("tree_uri")),
        getString(getColumnIndexOrThrow("name")), getString(getColumnIndexOrThrow("revision")),
        getInt(getColumnIndexOrThrow("enabled")) != 0, getInt(getColumnIndexOrThrow("unmetered")) != 0,
        getColumnIndexOrThrow("last_scan").let { if (isNull(it)) null else getLong(it) },
        getString(getColumnIndexOrThrow("error")), getInt(getColumnIndexOrThrow("waiting")), getInt(getColumnIndexOrThrow("skipped")),
        getInt(getColumnIndexOrThrow("prepared")) != 0,
        getInt(getColumnIndexOrThrow("directory_sync")) != 0,
    )
}

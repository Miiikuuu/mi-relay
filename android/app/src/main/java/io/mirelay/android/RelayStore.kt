package io.mirelay.android

import android.content.ContentValues
import android.content.Context
import android.database.Cursor
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.io.File
import java.util.UUID

/** All database/filesystem methods run on an IO or WorkManager thread. */
class RelayStore(context: Context, private val vault: TokenCipher = TokenVault()) : SQLiteOpenHelper(context, "relay.db", null, 7) {
    private val root = File(context.noBackupFilesDir, "outgoing")
    private val cleanupRoot = File(context.noBackupFilesDir.canonicalFile, "outgoing")
    internal val photoPreviews = PhotoPreviewCache(File(context.cacheDir.canonicalFile, "photo-previews"))
    private val mutableFolders = MutableStateFlow<List<Folder>>(emptyList())
    private val mutableTransfers = MutableStateFlow<List<Transfer>>(emptyList())
    private val refreshLock = Any()
    val folders = mutableFolders.asStateFlow()
    val transfers = mutableTransfers.asStateFlow()
    val automatic by lazy { AutoStore(this) }
    val directorySync by lazy { DirectorySyncStore(this) }

    override fun onConfigure(db: SQLiteDatabase) { db.setForeignKeyConstraintsEnabled(true) }
    override fun onCreate(db: SQLiteDatabase) {
        db.execSQL("CREATE TABLE folders (id TEXT PRIMARY KEY, name TEXT NOT NULL, server TEXT NOT NULL, insecure INTEGER NOT NULL, token TEXT NOT NULL, pairing_state TEXT NOT NULL DEFAULT 'legacy', verification TEXT)")
        db.execSQL("CREATE TABLE transfers (id TEXT PRIMARY KEY, folder_id TEXT NOT NULL REFERENCES folders(id), name TEXT NOT NULL, size INTEGER NOT NULL, status TEXT NOT NULL, uploaded INTEGER NOT NULL DEFAULT 0, error TEXT, delivery_id TEXT, work_id TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL, auto_revision TEXT, auto_unmetered INTEGER NOT NULL DEFAULT 0)")
        db.execSQL("CREATE INDEX transfer_folder ON transfers(folder_id, created_at DESC)")
        AutoStore.createTables(db)
        DirectorySyncStore.createTables(db)
        createCategoryColumns(db)
        db.execSQL("ALTER TABLE folders ADD COLUMN exit_phase TEXT")
    }
    override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) {
        check(oldVersion in 1..6 && newVersion == 7) { "Unsupported database version; existing data was not changed." }
        if (oldVersion == 1) {
            db.execSQL("ALTER TABLE transfers ADD COLUMN auto_revision TEXT")
            db.execSQL("ALTER TABLE transfers ADD COLUMN auto_unmetered INTEGER NOT NULL DEFAULT 0")
            AutoStore.createTables(db)
        } else if (oldVersion == 2) db.execSQL("ALTER TABLE auto_sources ADD COLUMN prepared INTEGER NOT NULL DEFAULT 0")
        if (oldVersion < 3) {
            db.execSQL("ALTER TABLE folders ADD COLUMN pairing_state TEXT NOT NULL DEFAULT 'legacy'")
            db.execSQL("ALTER TABLE folders ADD COLUMN verification TEXT")
        }
        if (oldVersion < 4) DirectorySyncStore.createTables(db)
        if (oldVersion < 5) createCategoryColumns(db)
        if (oldVersion < 7) db.execSQL("ALTER TABLE folders ADD COLUMN exit_phase TEXT")
        // Version 6 is a semantic migration: older apps must not reopen this
        // database and turn disconnect_pending back into ready via handshake.
    }

    private fun createCategoryColumns(db: SQLiteDatabase) {
        db.execSQL("ALTER TABLE folders ADD COLUMN kind TEXT NOT NULL DEFAULT 'general'")
        db.execSQL("ALTER TABLE auto_sources ADD COLUMN file_filter TEXT NOT NULL DEFAULT 'all'")
        db.execSQL("ALTER TABLE auto_sources ADD COLUMN filtered INTEGER NOT NULL DEFAULT 0")
        db.execSQL("ALTER TABLE directory_previews ADD COLUMN file_filter TEXT NOT NULL DEFAULT 'all'")
        db.execSQL("ALTER TABLE directory_previews ADD COLUMN skipped TEXT NOT NULL DEFAULT '[]'")
    }

    fun setFolderKind(id: String, kind: FolderKind) {
        check(writableDatabase.update("folders", ContentValues().apply { put("kind", kind.key) }, "id=?", arrayOf(id)) == 1) { "Folder no longer exists." }
        refresh()
    }

    // Never hold SQLiteOpenHelper's monitor while waiting for a connection:
    // a transaction on another thread needs that monitor to obtain the database
    // handle and finish. Serialize UI snapshots on an independent lock instead.
    fun refresh() = synchronized(refreshLock) {
        mutableFolders.value = readableDatabase.rawQuery("SELECT * FROM folders ORDER BY name COLLATE NOCASE, id", null).use { cursor ->
            buildList { while (cursor.moveToNext()) add(cursor.folder()) }
        }
        mutableTransfers.value = readableDatabase.rawQuery("SELECT * FROM transfers ORDER BY created_at DESC, id", null).use { cursor ->
            buildList { while (cursor.moveToNext()) add(cursor.transfer()) }
        }
        automatic.refresh()
    }

    fun saveFolder(id: String?, name: String, server: String, token: String, insecure: Boolean, pairingState: String? = null): String {
        if (id != null) requireConnectionOpen(id)
        require(name.trim().isNotEmpty() && name.toByteArray(Charsets.UTF_8).size <= 128 && name.none { it.isISOControl() }) { "Enter a Folder name (up to 128 bytes)." }
        val url = InputRules.server(server, insecure, BuildConfig.DEBUG)
        val key = id ?: UUID.randomUUID().toString()
        if (id != null) require(folder(id)?.server == url) { "Create a new Folder to change the server." }
        val values = ContentValues().apply {
            put("name", name.trim()); put("server", url); put("insecure", if (insecure && BuildConfig.DEBUG) 1 else 0)
            if (id == null || token.isNotBlank()) put("token", vault.seal(key, InputRules.token(token)))
            if (pairingState != null) put("pairing_state", pairingState)
        }
        if (id == null) {
            values.put("id", key)
            writableDatabase.insertOrThrow("folders", null, values)
        } else {
            require(writableDatabase.update("folders", values, "id=? AND pairing_state NOT IN ('disconnect_pending','disconnected')", arrayOf(id)) == 1) { "Folder was removed or disconnected." }
        }
        refresh()
        return key
    }

    fun pairingResult(id: String, state: String, verification: String?) {
        if (state == "disconnected") { beginDisconnect(id); finishDisconnect(id); return }
        require(state in listOf("awaiting_peer", "awaiting_confirmation", "ready"))
        writableDatabase.update("folders", ContentValues().apply {
            put("pairing_state", state); put("verification", verification)
        }, "id=? AND pairing_state NOT IN ('disconnect_pending','disconnected')", arrayOf(id))
        refresh()
    }

    internal fun connectionOpen(id: String): Boolean = readableDatabase.rawQuery(
        "SELECT 1 FROM folders WHERE id=? AND pairing_state NOT IN ('disconnect_pending','disconnected')", arrayOf(id)
    ).use { it.moveToFirst() }

    internal fun requireConnectionOpen(id: String) {
        require(connectionOpen(id)) { "Folder is disconnected or awaiting disconnection. Create a new Folder to reconnect." }
    }

    /** Durable local barrier BEFORE contacting the server or cancelling workers.
     * Keep the credential on failure so a lost response can be retried safely. */
    internal fun beginDisconnect(id: String) {
        val db = writableDatabase
        db.beginTransaction()
        try {
            requireNotNull(folder(id)) { "Folder no longer exists." }
            db.execSQL("UPDATE folders SET pairing_state='disconnect_pending',verification=NULL WHERE id=? AND pairing_state!='disconnected'", arrayOf(id))
            db.execSQL("UPDATE auto_sources SET enabled=0,prepared=0 WHERE folder_id=?", arrayOf(id))
            db.execSQL("UPDATE transfers SET work_id='',status=CASE WHEN status IN ('QUEUED','UPLOADING') THEN 'PAUSED' ELSE status END WHERE folder_id=?", arrayOf(id))
            db.setTransactionSuccessful()
        } finally { db.endTransaction() }
        refresh()
    }

    internal fun finishDisconnect(id: String) {
        require(folder(id)?.exitPhase == null) { "Finish clean exit before forgetting its receipt credential." }
        check(writableDatabase.update("folders", ContentValues().apply {
            put("pairing_state", "disconnected"); put("token", ""); putNull("verification")
        }, "id=? AND exit_phase IS NULL AND pairing_state IN ('disconnect_pending','disconnected')", arrayOf(id)) == 1)
        refresh()
    }

    /** Local metadata only. Originals, received files and private staged bytes
     * are never recursively removed, even if an old worker still owns a file. */
    internal fun removeFolder(id: String) {
        val db = writableDatabase
        db.beginTransaction()
        try {
            val current = requireNotNull(folder(id)) { "Folder no longer exists." }
            require(current.exitPhase == null || current.exitPhase == "complete") { "Both devices must finish clean exit before removing its receipt." }
            require(current.pairingState == "disconnected" || (!current.scoped && current.pairingState == "disconnect_pending")) {
                "Wait for server disconnection confirmation before removing this Folder."
            }
            db.delete("transfers", "folder_id=?", arrayOf(id))
            check(db.delete("folders", "id=?", arrayOf(id)) == 1)
            db.setTransactionSuccessful()
        } finally { db.endTransaction() }
        refresh()
    }

    fun folder(id: String): Folder? = readableDatabase.rawQuery("SELECT * FROM folders WHERE id = ?", arrayOf(id)).use {
        if (it.moveToFirst()) it.folder() else null
    }
    fun token(id: String): String = readableDatabase.rawQuery("SELECT token FROM folders WHERE id = ?", arrayOf(id)).use {
        check(it.moveToFirst()) { "Folder no longer exists." }
        vault.open(id, it.getString(0))
    }
    fun transfer(id: String): Transfer? = readableDatabase.rawQuery("SELECT * FROM transfers WHERE id = ?", arrayOf(id)).use {
        if (it.moveToFirst()) it.transfer() else null
    }
    fun directory(id: String): File {
        require(UUID.fromString(id).toString() == id) { "Invalid transfer ID." }
        return File(root, id)
    }
    internal fun cleanupDirectory(id: String): File {
        val candidate = directory(id).canonicalFile
        require(candidate == File(cleanupRoot, id)) { "Private staging path has been redirected." }
        return candidate
    }
    internal fun cleanupTransfers(folder: String): Set<String> {
        refresh()
        val known = transfers.value.associate { it.id to it.folderId }
        val owned = known.filterValues { it == folder }.keys.toMutableSet()
        require(root.canonicalFile == cleanupRoot) { "Private staging root has been redirected." }
        if (!cleanupRoot.exists()) return owned
        for (entry in checkNotNull(cleanupRoot.listFiles())) {
            val directory = cleanupDirectory(entry.name)
            require(directory.isDirectory) { "Unexpected private staging entry." }
            val marker = File(directory, "folder-owner")
            if (marker.exists()) {
                require(marker.canonicalFile == marker.absoluteFile && marker.isFile && marker.length() == 36L) { "Invalid staging ownership marker; review required." }
                val owner = marker.readText(Charsets.US_ASCII)
                require(UUID.fromString(owner).toString() == owner && (known[entry.name] == null || known[entry.name] == owner)) { "Inconsistent staging ownership." }
                if (owner == folder) owned.add(entry.name)
            } else if (known[entry.name] == null) {
                require(directory.listFiles()!!.none {
                    // Legacy local removal intentionally kept lock inodes.
                    // An empty regular lock contains no transfer data and must
                    // stay in place, not block unrelated Folder retirement.
                    val emptyLock = it.name == "resume.json.lock" &&
                        !java.nio.file.Files.isSymbolicLink(it.toPath()) && it.isFile && it.length() == 0L
                    !emptyLock && (it.name == "payload" || it.name == "payload.part" || it.name.startsWith("resume.json"))
                }) { "Old unowned staging data requires review before clean exit can finish." }
            }
        }
        return owned
    }
    fun addTransfer(id: String, folder: String, name: String, size: Long) {
        require(size in 1..MAX_FILE_BYTES)
        val db = writableDatabase
        db.beginTransaction()
        try {
            requireConnectionOpen(folder)
            db.insertOrThrow("transfers", null, ContentValues().apply {
                put("id", id); put("folder_id", folder); put("name", name); put("size", size)
                put("status", TransferStatus.QUEUED.name); put("created_at", System.currentTimeMillis())
            })
            db.setTransactionSuccessful()
        } finally { db.endTransaction() }
    }
    fun assign(id: String, workId: String) {
        check(writableDatabase.update("transfers", ContentValues().apply {
            put("work_id", workId); put("status", TransferStatus.QUEUED.name); putNull("error")
        }, "id = ? AND status != ? AND ${AutoStore.ALLOWED_TRANSFER}", arrayOf(id, TransferStatus.UPLOADED.name)) == 1)
        refresh()
    }
    fun pause(id: String) {
        writableDatabase.update("transfers", ContentValues().apply {
            put("work_id", ""); put("status", TransferStatus.PAUSED.name)
        }, "id = ? AND status IN (?, ?)", arrayOf(id, TransferStatus.QUEUED.name, TransferStatus.UPLOADING.name))
        refresh()
    }
    fun updateOwned(id: String, workId: String, status: TransferStatus, uploaded: Long? = null, error: String? = null, delivery: String? = null): Boolean {
        val updated = writableDatabase.update("transfers", ContentValues().apply {
            put("status", status.name); if (uploaded != null) put("uploaded", uploaded)
            if (error == null) putNull("error") else put("error", error.take(1000))
            if (delivery != null) put("delivery_id", delivery)
        }, "id = ? AND work_id = ? AND status != ? AND ${AutoStore.ALLOWED_TRANSFER}", arrayOf(id, workId, TransferStatus.UPLOADED.name)) == 1
        if (updated) refresh()
        return updated
    }
    private fun Cursor.str(column: String) = getString(getColumnIndexOrThrow(column))
    private fun Cursor.long(column: String) = getLong(getColumnIndexOrThrow(column))
    private fun Cursor.folder() = Folder(str("id"), str("name"), str("server"), long("insecure") != 0L, str("pairing_state"), str("verification"), FolderKind.fromKey(str("kind")), str("exit_phase"))
    private fun Cursor.transfer() = Transfer(str("id"), str("folder_id"), str("name"), long("size"), TransferStatus.valueOf(str("status")), long("uploaded"), str("error"), str("delivery_id"), str("work_id"), long("created_at"), str("auto_revision"), long("auto_unmetered") != 0L,
        str("relative_path"), getColumnIndexOrThrow("source_version").let { if (isNull(it)) null else getLong(it) }, str("source_sha256"), long("received") != 0L, long("conflict") != 0L, long("superseded") != 0L)
}

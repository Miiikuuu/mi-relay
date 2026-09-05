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
class RelayStore(context: Context, private val vault: TokenCipher = TokenVault()) : SQLiteOpenHelper(context, "relay.db", null, 4) {
    private val root = File(context.noBackupFilesDir, "outgoing")
    private val mutableFolders = MutableStateFlow<List<Folder>>(emptyList())
    private val mutableTransfers = MutableStateFlow<List<Transfer>>(emptyList())
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
    }
    override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) {
        check(oldVersion in 1..3 && newVersion == 4) { "Unsupported database version; existing data was not changed." }
        if (oldVersion == 1) {
            db.execSQL("ALTER TABLE transfers ADD COLUMN auto_revision TEXT")
            db.execSQL("ALTER TABLE transfers ADD COLUMN auto_unmetered INTEGER NOT NULL DEFAULT 0")
            AutoStore.createTables(db)
        } else if (oldVersion == 2) db.execSQL("ALTER TABLE auto_sources ADD COLUMN prepared INTEGER NOT NULL DEFAULT 0")
        if (oldVersion < 3) {
            db.execSQL("ALTER TABLE folders ADD COLUMN pairing_state TEXT NOT NULL DEFAULT 'legacy'")
            db.execSQL("ALTER TABLE folders ADD COLUMN verification TEXT")
        }
        DirectorySyncStore.createTables(db)
    }

    @Synchronized fun refresh() {
        mutableFolders.value = readableDatabase.rawQuery("SELECT * FROM folders ORDER BY name COLLATE NOCASE, id", null).use { cursor ->
            buildList { while (cursor.moveToNext()) add(cursor.folder()) }
        }
        mutableTransfers.value = readableDatabase.rawQuery("SELECT * FROM transfers ORDER BY created_at DESC, id", null).use { cursor ->
            buildList { while (cursor.moveToNext()) add(cursor.transfer()) }
        }
        automatic.refresh()
    }

    fun saveFolder(id: String?, name: String, server: String, token: String, insecure: Boolean, pairingState: String? = null): String {
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
            require(writableDatabase.update("folders", values, "id = ?", arrayOf(id)) == 1) { "Folder no longer exists." }
        }
        refresh()
        return key
    }

    fun pairingResult(id: String, state: String, verification: String?) {
        require(state in listOf("awaiting_peer", "awaiting_confirmation", "ready"))
        writableDatabase.update("folders", ContentValues().apply {
            put("pairing_state", state); put("verification", verification)
        }, "id=?", arrayOf(id))
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
    fun addTransfer(id: String, folder: String, name: String, size: Long) {
        require(size in 1..MAX_FILE_BYTES)
        writableDatabase.insertOrThrow("transfers", null, ContentValues().apply {
            put("id", id); put("folder_id", folder); put("name", name); put("size", size)
            put("status", TransferStatus.QUEUED.name); put("created_at", System.currentTimeMillis())
        })
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
    private fun Cursor.folder() = Folder(str("id"), str("name"), str("server"), long("insecure") != 0L, str("pairing_state"), str("verification"))
    private fun Cursor.transfer() = Transfer(str("id"), str("folder_id"), str("name"), long("size"), TransferStatus.valueOf(str("status")), long("uploaded"), str("error"), str("delivery_id"), str("work_id"), long("created_at"), str("auto_revision"), long("auto_unmetered") != 0L,
        str("relative_path"), getColumnIndexOrThrow("source_version").let { if (isNull(it)) null else getLong(it) }, str("source_sha256"), long("received") != 0L, long("conflict") != 0L, long("superseded") != 0L)
}

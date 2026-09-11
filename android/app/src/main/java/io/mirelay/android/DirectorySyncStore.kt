package io.mirelay.android

import android.content.ContentValues
import android.database.sqlite.SQLiteDatabase
import org.json.JSONObject
import java.util.UUID

/** Path identity, version allocation and transfer insertion share one transaction.
 * No source file is deleted; no version/baseline is reset when Auto is paused. */
class DirectorySyncStore(private val relay: RelayStore) {
    private val db get() = relay.writableDatabase
    companion object {
        internal fun createTables(db: SQLiteDatabase) {
            db.execSQL("ALTER TABLE auto_sources ADD COLUMN directory_sync INTEGER NOT NULL DEFAULT 0")
            db.execSQL("ALTER TABLE auto_sources ADD COLUMN next_version INTEGER NOT NULL DEFAULT 1")
            db.execSQL("ALTER TABLE transfers ADD COLUMN relative_path TEXT")
            db.execSQL("ALTER TABLE transfers ADD COLUMN source_version INTEGER")
            db.execSQL("ALTER TABLE transfers ADD COLUMN source_sha256 TEXT")
            db.execSQL("ALTER TABLE transfers ADD COLUMN received INTEGER NOT NULL DEFAULT 0")
            db.execSQL("ALTER TABLE transfers ADD COLUMN conflict INTEGER NOT NULL DEFAULT 0")
            db.execSQL("ALTER TABLE transfers ADD COLUMN superseded INTEGER NOT NULL DEFAULT 0")
            db.execSQL("CREATE UNIQUE INDEX directory_transfer_version ON transfers(folder_id,relative_path,source_version) WHERE relative_path IS NOT NULL")
            db.execSQL("CREATE TABLE directory_files (folder_id TEXT NOT NULL REFERENCES auto_sources(folder_id) ON DELETE CASCADE, path TEXT NOT NULL, document_id TEXT NOT NULL, fingerprint TEXT NOT NULL, sha256 TEXT, observed_sha TEXT NOT NULL, stable_since INTEGER NOT NULL, state TEXT NOT NULL, error TEXT, PRIMARY KEY(folder_id,path))")
            db.execSQL("CREATE TABLE directory_previews (folder_id TEXT PRIMARY KEY REFERENCES folders(id) ON DELETE CASCADE, id TEXT NOT NULL, tree_uri TEXT NOT NULL, name TEXT NOT NULL, scope TEXT NOT NULL, source TEXT NOT NULL, remote TEXT NOT NULL, comparison TEXT NOT NULL)")
        }
    }
    fun preview(folder: String): DirectoryPreview? = db.rawQuery("SELECT id,tree_uri,name,comparison,file_filter,skipped FROM directory_previews WHERE folder_id=?", arrayOf(folder)).use {
        if (!it.moveToFirst()) null else JSONObject(it.getString(3)).let { diff -> DirectoryPreview(it.getString(0),it.getString(1),it.getString(2),diff.getInt("identical"),diff.strings("missing"),diff.strings("different"),diff.getInt("destination_only"),FileFilter.fromKey(it.getString(4)),skippedFromJson(it.getString(5))) }
    }
    internal fun requirePreviewAllowed(folder: String, tree: String) {
        val source = relay.automatic.source(folder)
        require(source?.enabled != true) { "Pause Auto before previewing changes." }
        require(source?.directorySync != true || source.treeUri == tree) { "An initialized directory cannot be remapped. Choose its original source." }
    }
    internal fun savePreview(folder: Folder, tree: String, name: String, files: List<HashedSource>, remote: JSONObject, diff: JSONObject,
        filter: FileFilter = FileFilter.ALL, skipped: List<SkippedFile> = emptyList()): DirectoryPreview {
        requirePreviewAllowed(folder.id, tree)
        requireFilterChange(folder.id, filter)
        val source = files.snapshotJson()
        require(source.toByteArray().size + remote.toString().toByteArray().size + diff.toString().toByteArray().size + skipped.toJson().toByteArray().size <= 8 * 1024 * 1024) { "Directory preview exceeds its safety limit." }
        db.insertWithOnConflict("directory_previews", null, ContentValues().apply {
            put("folder_id",folder.id); put("id",UUID.randomUUID().toString()); put("tree_uri",tree); put("name",name)
            put("scope",folder.server); put("source",source); put("remote",remote.toString()); put("comparison",diff.toString())
            put("file_filter",filter.key); put("skipped",skipped.toJson())
        }, SQLiteDatabase.CONFLICT_REPLACE).also { check(it != -1L) }
        return requireNotNull(preview(folder.id))
    }
    internal fun confirm(folder: Folder, previewId: String, tree: String, files: List<HashedSource>, remote: JSONObject, unmetered: Boolean, now: Long,
        filter: FileFilter = FileFilter.ALL, skipped: List<SkippedFile> = emptyList()): AutoSource {
        transaction {
            requirePreviewAllowed(folder.id, tree)
            requireFilterChange(folder.id, filter)
            require(!db.rawQuery("SELECT 1 FROM transfers WHERE folder_id=? AND status!='UPLOADED' LIMIT 1",arrayOf(folder.id)).use { it.moveToFirst() }) { "Finish existing transfers before initializing this Folder." }
            val diff = db.rawQuery("SELECT source,remote,comparison,file_filter,skipped FROM directory_previews WHERE folder_id=? AND id=? AND tree_uri=? AND scope=?", arrayOf(folder.id,previewId,tree,folder.server)).use {
                require(it.moveToFirst() && it.getString(0) == files.snapshotJson() && it.getString(1) == remote.toString() && it.getString(3) == filter.key && it.getString(4) == skipped.toJson()) { "Source, filter or Linux changed since preview. Preview again before confirming." }
                JSONObject(it.getString(2))
            }
            val changed = (diff.strings("missing") + diff.strings("different")).toSet()
            val name = requireNotNull(preview(folder.id)).name
            val previousVersion = db.rawQuery("SELECT next_version FROM auto_sources WHERE folder_id=?", arrayOf(folder.id)).use { if(it.moveToFirst()) it.getLong(0) else 1L }
            val historical = db.rawQuery("SELECT path FROM directory_files WHERE folder_id=?", arrayOf(folder.id)).use { rows -> buildSet { while(rows.moveToNext()) add(rows.getString(0)) } }
            require((historical + files.map { it.file.relativePath }).size <= AutoStore.MAX_ENTRIES) { "Directory path history reached 5,000 entries. No files were removed." }
            val values = ContentValues().apply {
                put("tree_uri",tree); put("name",name); put("revision",UUID.randomUUID().toString()); put("enabled",1)
                put("prepared",0); put("directory_sync",1); put("next_version",maxOf(previousVersion,floor(remote))); put("unmetered",if(unmetered)1 else 0); put("last_scan",now); putNull("error")
                put("file_filter",filter.key); put("filtered",skipped.size)
            }
            if(db.update("auto_sources",values,"folder_id=?",arrayOf(folder.id)) == 0) { values.put("folder_id",folder.id); db.insertOrThrow("auto_sources",null,values) }
            db.delete("auto_files","folder_id=?",arrayOf(folder.id))
            // Keep excluded path history and monotonically increasing versions.
            // Exclusion never removes a source, destination, or transfer record.
            db.update("directory_files",ContentValues().apply {put("state","EXCLUDED");putNull("error")},"folder_id=?",arrayOf(folder.id))
            for(value in files) {
                DirectoryPaths.validate(value.file.relativePath)
                val fileValues = ContentValues().apply {
                    put("folder_id",folder.id); put("path",value.file.relativePath); put("document_id",value.file.documentId); put("fingerprint",value.file.fingerprint)
                    if(value.file.relativePath !in changed) put("sha256",value.sha256) else putNull("sha256")
                    put("observed_sha",value.sha256); put("stable_since",now - AutoStore.STABLE_MILLIS)
                    put("state",if(value.file.relativePath in changed) "PENDING" else "CLEAN")
                    putNull("error")
                }
                if(db.update("directory_files",fileValues,"folder_id=? AND path=?",arrayOf(folder.id,value.file.relativePath)) == 0) db.insertOrThrow("directory_files",null,fileValues)
            }
            db.delete("directory_previews","folder_id=?",arrayOf(folder.id))
        }
        relay.refresh(); return requireNotNull(relay.automatic.source(folder.id))
    }
    internal fun filteredResult(source: AutoSource, count: Int) {
        db.update("auto_sources", ContentValues().apply { put("filtered", count) }, "folder_id=? AND revision=? AND enabled=1", arrayOf(source.folderId,source.revision))
    }
    private fun requireFilterChange(folder: String, filter: FileFilter) {
        val source = relay.automatic.source(folder)
        require(source?.directorySync != true || source.fileFilter != filter) { "This directory is already initialized with this filter. Resume sync without resetting its baseline." }
    }
    internal fun resume(folder: String, tree: String, unmetered: Boolean): AutoSource {
        transaction {
            val source = requireNotNull(relay.automatic.source(folder))
            require(source.directorySync && !source.enabled && source.treeUri == tree) { "Resume the initialized directory. Use a new Folder to change its source." }
            val revision = UUID.randomUUID().toString()
            db.update("auto_sources",ContentValues().apply { put("enabled",1);put("revision",revision);put("unmetered",if(unmetered)1 else 0);putNull("error") },"folder_id=?",arrayOf(folder))
            db.update("transfers",ContentValues().apply {put("auto_revision",revision);put("auto_unmetered",if(unmetered)1 else 0);put("status","QUEUED");put("work_id","")},
                "folder_id=? AND relative_path IS NOT NULL AND status IN ('PAUSED','QUEUED','UPLOADING')",arrayOf(folder))
            // Failed versions remain visible/retryable without becoming manual
            // transfers that could bypass the directory's network/consent rules.
            db.update("transfers",ContentValues().apply {put("auto_revision",revision);put("auto_unmetered",if(unmetered)1 else 0)},"folder_id=? AND relative_path IS NOT NULL AND status='FAILED'",arrayOf(folder))
        }
        relay.refresh(); return requireNotNull(relay.automatic.source(folder))
    }
    internal fun observe(source: AutoSource, files: List<HashedSource>, now: Long): List<HashedSource> {
        val ready = mutableListOf<HashedSource>()
        require(files.size <= AutoStore.MAX_ENTRIES && files.map { it.file.relativePath }.distinct().size == files.size)
        transaction {
            check(source.directorySync && relay.automatic.active(source.folderId,source.revision))
            var count = db.rawQuery("SELECT COUNT(*) FROM directory_files WHERE folder_id=?",arrayOf(source.folderId)).use {it.moveToFirst();it.getInt(0)}
            for(value in files) {
                val file=value.file;DirectoryPaths.validate(file.relativePath)
                db.rawQuery("SELECT sha256,observed_sha,stable_since,error FROM directory_files WHERE folder_id=? AND path=?",arrayOf(source.folderId,file.relativePath)).use {
                    if(!it.moveToFirst()) {
                        require(++count <= AutoStore.MAX_ENTRIES) { "Directory path history reached 5,000 entries. No files were removed." }
                        db.insertOrThrow("directory_files",null,ContentValues().apply {put("folder_id",source.folderId);put("path",file.relativePath);put("document_id",file.documentId);put("fingerprint",file.fingerprint);put("observed_sha",value.sha256);put("stable_since",now);put("state","PENDING")})
                    } else {
                        val clean=it.getString(0)==value.sha256
                        val stable=it.getString(1)==value.sha256 && now>=it.getLong(2) && it.getString(3)!="Source file is unavailable."
                        db.update("directory_files",ContentValues().apply {
                            put("document_id",file.documentId);put("fingerprint",file.fingerprint);put("observed_sha",value.sha256)
                            if(!stable) put("stable_since",now)
                            put("state",if(clean) "CLEAN" else "PENDING");putNull("error")
                        },"folder_id=? AND path=?",arrayOf(source.folderId,file.relativePath))
                        if(!clean && stable && now-it.getLong(2)>=AutoStore.STABLE_MILLIS) ready.add(value)
                    }
                }
            }
            val paths=files.map {it.file.relativePath}.toSet()
            db.rawQuery("SELECT path FROM directory_files WHERE folder_id=? AND state='PENDING'",arrayOf(source.folderId)).use { while(it.moveToNext()) if(it.getString(0) !in paths) {
                db.update("directory_files",ContentValues().apply {put("stable_since",now);put("error","Source file is unavailable.")},"folder_id=? AND path=?",arrayOf(source.folderId,it.getString(0)))
            } }
        }
        return ready
    }
    internal fun commit(source: AutoSource, value: HashedSource, staged: StagedFile): Boolean {
        var created=false
        transaction {
            if(!relay.automatic.active(source.folderId,source.revision)) return@transaction
            require(staged.sha256==value.sha256 && staged.size==value.file.size && staged.name==value.file.name) { "Source changed while copying." }
            val matches=db.rawQuery("SELECT 1 FROM directory_files WHERE folder_id=? AND path=? AND fingerprint=? AND observed_sha=? AND state='PENDING'",arrayOf(source.folderId,value.file.relativePath,value.file.fingerprint,value.sha256)).use {it.moveToFirst()}
            if(!matches) return@transaction
            val version=db.rawQuery("SELECT next_version FROM auto_sources WHERE folder_id=?",arrayOf(source.folderId)).use {it.moveToFirst();it.getLong(0)}
            require(version in 1 until Long.MAX_VALUE) { "Directory version exhausted." }
            relay.addTransfer(staged.id,source.folderId,staged.name,staged.size)
            db.update("transfers",ContentValues().apply {put("auto_revision",source.revision);put("auto_unmetered",if(source.unmetered)1 else 0);put("relative_path",value.file.relativePath);put("source_version",version);put("source_sha256",value.sha256)},"id=?",arrayOf(staged.id))
            db.update("auto_sources",ContentValues().apply {put("next_version",version+1)},"folder_id=?",arrayOf(source.folderId))
            db.update("directory_files",ContentValues().apply {put("sha256",value.sha256);put("state","CLEAN");putNull("error")},"folder_id=? AND path=?",arrayOf(source.folderId,value.file.relativePath))
            created=true
        }
        return created
    }
    internal fun receipts(folder: String, remote: JSONObject) {
        transaction {
            val entries=remote.getJSONArray("entries")
            for(i in 0 until entries.length()) {
                val entry=entries.getJSONObject(i);val path=entry.getString("path");DirectoryPaths.validate(path)
                val version=entry.getLong("version");val hash=entry.getString("sha256")
                db.update("transfers",ContentValues().apply {put("superseded",1)},"folder_id=? AND relative_path=? AND source_version<? AND status='UPLOADED'",arrayOf(folder,path,version.toString()))
                if(entry.getBoolean("acknowledged")) db.update("transfers",ContentValues().apply {put("received",1);put("conflict",if(entry.getBoolean("conflict"))1 else 0)},
                    "folder_id=? AND relative_path=? AND source_version=? AND source_sha256=? AND status='UPLOADED'",arrayOf(folder,path,version.toString(),hash))
            }
            db.execSQL("UPDATE auto_sources SET next_version=MAX(next_version,?) WHERE folder_id=? AND directory_sync=1",arrayOf<Any>(floor(remote),folder))
        }
        relay.refresh()
    }
    internal fun fileError(source: AutoSource, value: HashedSource, now: Long) {
        if(!relay.automatic.active(source.folderId,source.revision)) return
        db.update("directory_files",ContentValues().apply {put("error","Source changed or could not be staged. Will retry on the next check.");put("stable_since",now)},"folder_id=? AND path=? AND observed_sha=? AND state='PENDING'",arrayOf(source.folderId,value.file.relativePath,value.sha256))
    }
    internal fun needsSettle(source: AutoSource): Boolean = relay.automatic.active(source.folderId,source.revision) && db.rawQuery("SELECT 1 FROM directory_files WHERE folder_id=? AND state='PENDING' AND error IS NULL LIMIT 1",arrayOf(source.folderId)).use {it.moveToFirst()}
    private fun floor(remote: JSONObject): Long {
        val entries=remote.getJSONArray("entries");var maximum=0L
        for(i in 0 until entries.length()) maximum=maxOf(maximum,entries.getJSONObject(i).getLong("version"))
        require(maximum in 0 until Long.MAX_VALUE-1) { "Directory version exhausted." };return maximum+1
    }
    private inline fun transaction(block: () -> Unit) {db.beginTransaction();try {block();db.setTransactionSuccessful()} finally {db.endTransaction()}}
}

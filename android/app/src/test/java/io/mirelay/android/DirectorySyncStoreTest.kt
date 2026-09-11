package io.mirelay.android

import android.app.Application
import android.net.Uri
import org.json.JSONArray
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import java.util.UUID

@RunWith(RobolectricTestRunner::class)
@Config(sdk=[35],application=Application::class)
class DirectorySyncStoreTest {
    private lateinit var store:RelayStore
    private lateinit var folder:Folder
    private val tree="content://source/tree/root"
    private val cipher=object:TokenCipher {override fun seal(id:String,token:String)="cipher:$token";override fun open(id:String,value:String)=value.removePrefix("cipher:")}
    @Before fun setup() {
        val context=RuntimeEnvironment.getApplication();context.deleteDatabase("relay.db");store=RelayStore(context,cipher)
        val id=store.saveFolder(null,"Directory","https://example.com/f/${UUID.randomUUID()}","secret",false,"ready");folder=store.folder(id)!!
    }
    @After fun close(){store.close()}
    private fun file(path:String="Pixiv/a.txt",hash:Char='a',id:String=path)=HashedSource(SourceFile(id,Uri.parse("content://source/document/$id"),path.substringAfterLast('/'),42,1,path),hash.toString().repeat(64))
    private fun remote(entries:JSONArray=JSONArray())=JSONObject().put("schema_version",1).put("receiver",JSONObject().put("id","00000000-0000-4000-8000-000000000001").put("entries",JSONArray())).put("entries",entries)
    private fun diff(files:List<HashedSource>,identical:Set<String> = emptySet())=JSONObject().put("identical",identical.size).put("missing",JSONArray(files.filter{it.file.relativePath !in identical}.map{it.file.relativePath})).put("different",JSONArray()).put("destination_only",0)
    private fun start(files:List<HashedSource>,identical:Set<String> = emptySet()):AutoSource {
        val state=remote();val preview=store.directorySync.savePreview(folder,tree,"Pixiv",files,state,diff(files,identical))
        return store.directorySync.confirm(folder,preview.id,tree,files,state,true,10000)
    }
    private fun staged(value:HashedSource)=StagedFile(UUID.randomUUID().toString(),value.file.name,value.file.size!!,value.sha256)
    private fun commit(source:AutoSource,value:HashedSource):Transfer {
        val staged=staged(value);assertTrue(store.directorySync.commit(source,value,staged));store.refresh();return store.transfer(staged.id)!!
    }
    private fun uploaded(value:Transfer) {store.assign(value.id,"worker");assertTrue(store.updateOwned(value.id,"worker",TransferStatus.UPLOADED,42,delivery=UUID.randomUUID().toString()))}

    @Test fun previewPersistsWithoutEnablingOrQueuing() {
        val files=listOf(file());val preview=store.directorySync.savePreview(folder,tree,"Pixiv",files,remote(),diff(files))
        assertNull(store.automatic.source(folder.id));assertTrue(store.transfers.value.isEmpty())
        store.close();store=RelayStore(RuntimeEnvironment.getApplication(),cipher)
        assertEquals(preview,store.directorySync.preview(folder.id));assertNull(store.automatic.source(folder.id))
        assertEquals("secret",store.token(folder.id))
    }
    @Test fun previewBindsFilterAndSkippedFilesAcrossRestart() {
        val files=listOf(file("photo.jpg"));val state=remote()
        val skipped=listOf(SkippedFile("notes.txt","Not an included image extension","doc:42:1:notes.txt"))
        val plan=store.directorySync.savePreview(folder,tree,"Photos",files,state,diff(files),FileFilter.IMAGES,skipped)
        store.close();store=RelayStore(RuntimeEnvironment.getApplication(),cipher)
        assertEquals(plan,store.directorySync.preview(folder.id))
        assertThrows(IllegalArgumentException::class.java) {store.directorySync.confirm(folder,plan.id,tree,files,state,true,10000,FileFilter.ALL,skipped)}
        assertThrows(IllegalArgumentException::class.java) {store.directorySync.confirm(folder,plan.id,tree,files,state,true,10000,FileFilter.IMAGES,emptyList())}
        assertNull(store.automatic.source(folder.id));assertTrue(store.transfers.value.isEmpty())
        val source=store.directorySync.confirm(folder,plan.id,tree,files,state,true,10000,FileFilter.IMAGES,skipped)
        assertEquals(FileFilter.IMAGES,source.fileFilter);assertEquals(1,source.filtered)
    }
    @Test fun changingFilterPreservesVersionsHistoryAndUploadedRecords() {
        val text=file("notes.txt");val photo=file("cover.jpg",'b');val original=start(listOf(text,photo))
        val textTransfer=commit(original,text);uploaded(textTransfer)
        val imageTransfer=commit(original,photo);uploaded(imageTransfer)
        store.automatic.disable(folder.id)
        val skipped=listOf(SkippedFile("notes.txt","Not an included image extension","fingerprint"))
        val state=remote();val plan=store.directorySync.savePreview(folder,tree,"Photos",listOf(photo),state,diff(listOf(photo),setOf(photo.file.relativePath)),FileFilter.IMAGES,skipped)
        val changed=store.directorySync.confirm(folder,plan.id,tree,listOf(photo),state,true,30000,FileFilter.IMAGES,skipped)
        assertNotEquals(original.revision,changed.revision);assertEquals(FileFilter.IMAGES,changed.fileFilter)
        assertEquals(2,store.transfers.value.size)
        assertEquals("EXCLUDED",store.readableDatabase.rawQuery("SELECT state FROM directory_files WHERE path='notes.txt'",null).use {it.moveToFirst();it.getString(0)})
        assertFalse(store.directorySync.commit(original,text,staged(text)))
        val next=file("new.jpg",'c');store.directorySync.observe(changed,listOf(photo,next),50000)
        store.directorySync.observe(changed,listOf(photo,next),60000)
        assertEquals(3L,commit(changed,next).sourceVersion)
        // A delayed old scan must not change the new policy's visible count.
        store.directorySync.filteredResult(original,999)
        assertEquals(1,store.automatic.source(folder.id)!!.filtered)
    }
    @Test fun pendingTransfersBlockChangingTheFilterWithoutDeletingAnything() {
        val value=file("note.txt");val source=start(listOf(value));val transfer=commit(source,value)
        store.automatic.disable(folder.id);val state=remote()
        val plan=store.directorySync.savePreview(folder,tree,"Photos",emptyList(),state,diff(emptyList()),FileFilter.IMAGES)
        assertThrows(IllegalArgumentException::class.java) {store.directorySync.confirm(folder,plan.id,tree,emptyList(),state,true,20000,FileFilter.IMAGES)}
        assertNotNull(store.transfer(transfer.id));assertEquals(FileFilter.ALL,store.automatic.source(folder.id)!!.fileFilter)
        assertFalse(store.automatic.source(folder.id)!!.enabled)
    }
    @Test fun initializationRequiresMatchingPreviewSourceRemoteTreeAndIdentity() {
        val files=listOf(file());val state=remote();val plan=store.directorySync.savePreview(folder,tree,"Pixiv",files,state,diff(files))
        for(action in listOf<()->Unit>(
            {store.directorySync.confirm(folder,"wrong",tree,files,state,true,10000)},
            {store.directorySync.confirm(folder,plan.id,"content://other/tree/root",files,state,true,10000)},
            {store.directorySync.confirm(folder,plan.id,tree,listOf(file(hash='b')),state,true,10000)},
            {store.directorySync.confirm(folder,plan.id,tree,files,remote().put("changed",true),true,10000)}
        )) {assertThrows(IllegalArgumentException::class.java){action()};assertNull(store.automatic.source(folder.id));assertTrue(store.transfers.value.isEmpty())}
        store.directorySync.confirm(folder,plan.id,tree,files,state,true,10000)
        assertTrue(store.automatic.source(folder.id)!!.directorySync);assertNull(store.directorySync.preview(folder.id));assertTrue(store.transfers.value.isEmpty())
    }
    @Test fun equalBytesAtDifferentPathsAreBothQueuedAndIdenticalDestinationIsSkipped() {
        val files=listOf(file("A/same.txt"),file("B/same.txt"),file("already.txt"))
        val source=start(files,setOf("already.txt"));val ready=store.directorySync.observe(source,files,10001)
        assertEquals(2,ready.size);val first=commit(source,ready[0]);val second=commit(source,ready[1])
        assertNotEquals(first.relativePath,second.relativePath);assertEquals(first.sourceSha256,second.sourceSha256)
        assertEquals(1L,first.sourceVersion);assertEquals(2L,second.sourceVersion)
        assertTrue(store.directorySync.observe(source,files,30000).isEmpty())
    }
    @Test fun sameSizeAndTimestampModificationGetsNewVersionAndReversionGetsAnother() {
        val old=file();val source=start(listOf(old));commit(source,old)
        val changed=file(hash='b');assertEquals(old.file.fingerprint,changed.file.fingerprint)
        assertTrue(store.directorySync.observe(source,listOf(changed),20000).isEmpty())
        assertEquals(listOf(changed),store.directorySync.observe(source,listOf(changed),30000));assertEquals(2L,commit(source,changed).sourceVersion)
        assertTrue(store.directorySync.observe(source,listOf(old),40000).isEmpty())
        assertEquals(listOf(old),store.directorySync.observe(source,listOf(old),50000));assertEquals(3L,commit(source,old).sourceVersion)
    }
    @Test fun pauseResumeRotatesWorkerOwnershipButKeepsBaselineVersionsAndOfflineChanges() {
        val original=file();val source=start(listOf(original));val pending=commit(source,original);store.assign(pending.id,"old-worker")
        store.automatic.disable(folder.id)
        assertFalse(store.directorySync.commit(source,original,staged(original)))
        assertThrows(IllegalArgumentException::class.java){store.automatic.makeManual(pending.id)}
        val resumed=store.directorySync.resume(folder.id,tree,false)
        assertNotEquals(source.revision,resumed.revision);assertFalse(store.automatic.active(folder.id,source.revision))
        assertFalse(store.updateOwned(pending.id,"old-worker",TransferStatus.UPLOADED))
        assertEquals(1L,store.transfer(pending.id)!!.sourceVersion);assertEquals(resumed.revision,store.transfer(pending.id)!!.autoRevision)
        val added=file("offline.txt",'b');val current=listOf(original,added)
        assertTrue(store.directorySync.observe(resumed,current,20000).isEmpty())
        assertEquals(listOf(added),store.directorySync.observe(resumed,current,30000));assertEquals(2L,commit(resumed,added).sourceVersion)
        store.automatic.makeManual(pending.id);assertEquals(resumed.revision,store.transfer(pending.id)!!.autoRevision)
    }
    @Test fun cannotRemapOrResetAnInitializedDirectory() {
        val source=start(listOf(file()));store.automatic.disable(folder.id)
        assertThrows(IllegalArgumentException::class.java){store.directorySync.resume(folder.id,"content://other/tree/root",true)}
        assertThrows(IllegalArgumentException::class.java){store.automatic.enable(folder.id,tree,"Reset",true,emptyList(),0)}
        assertThrows(IllegalArgumentException::class.java){store.directorySync.savePreview(folder,tree,"Reset",emptyList(),remote(),diff(emptyList()))}
        assertEquals(source.treeUri,store.automatic.source(folder.id)!!.treeUri)
    }
    @Test fun failedAtomicCommitDoesNotConsumeAVersionOrLeaveATransfer() {
        val value=file();val source=start(listOf(value))
        store.writableDatabase.execSQL("CREATE TRIGGER fail_directory_commit BEFORE UPDATE ON directory_files BEGIN SELECT RAISE(ABORT,'injected'); END")
        assertThrows(Exception::class.java){store.directorySync.commit(source,value,staged(value))}
        store.refresh();assertTrue(store.transfers.value.isEmpty())
        store.writableDatabase.execSQL("DROP TRIGGER fail_directory_commit")
        assertEquals(1L,commit(source,value).sourceVersion)
    }
    @Test fun changedOrMismatchedStagingNeverCreatesAQueueRecord() {
        val value=file();val source=start(listOf(value))
        assertThrows(IllegalArgumentException::class.java){store.directorySync.commit(source,value,staged(file(hash='b')))}
        assertTrue(store.transfers.value.isEmpty());assertEquals(1L,commit(source,value).sourceVersion)
    }
    @Test fun missingSourceFilesAreRetainedAndReappearingPendingFilesMustSettleAgain() {
        val value=file();val source=start(listOf(value))
        assertTrue(store.directorySync.observe(source,emptyList(),10001).isEmpty());assertFalse(store.directorySync.needsSettle(source))
        assertTrue(store.directorySync.observe(source,listOf(value),20000).isEmpty())
        assertEquals(listOf(value),store.directorySync.observe(source,listOf(value),30000));commit(source,value)
        store.directorySync.observe(source,emptyList(),40000);store.refresh();assertEquals(1,store.transfers.value.size)
    }
    @Test fun receiptsRequireExactPathVersionHashAndDistinguishSupersededFromSynced() {
        val value=file();val source=start(listOf(value));val first=commit(source,value);uploaded(first)
        val changed=file(hash='b');store.directorySync.observe(source,listOf(changed),20000);store.directorySync.observe(source,listOf(changed),30000)
        val second=commit(source,changed);uploaded(second)
        val entry=JSONObject().put("path",value.file.relativePath).put("version",2).put("sha256","c".repeat(64)).put("acknowledged",true).put("conflict",true)
        store.directorySync.receipts(folder.id,remote(JSONArray().put(entry)))
        assertTrue(store.transfer(first.id)!!.superseded);assertFalse(store.transfer(first.id)!!.received);assertFalse(store.transfer(second.id)!!.received)
        entry.put("sha256",changed.sha256);store.directorySync.receipts(folder.id,remote(JSONArray().put(entry)))
        assertTrue(store.transfer(second.id)!!.received);assertTrue(store.transfer(second.id)!!.conflict)
    }
    @Test fun directoryHistoryAndMetadataSurviveReopenAndRemoteFloorIsMonotonic() {
        val value=file();val source=start(listOf(value));val pending=commit(source,value)
        store.close();store=RelayStore(RuntimeEnvironment.getApplication(),cipher)
        assertEquals(pending,store.transfer(pending.id));assertTrue(store.directorySync.observe(source,listOf(value),20000).isEmpty())
        store.directorySync.receipts(folder.id,remote(JSONArray().put(JSONObject().put("path","remote.txt").put("version",50).put("sha256","a".repeat(64)).put("acknowledged",false))))
        store.directorySync.receipts(folder.id,remote())
        val added=file("new.txt");store.directorySync.observe(source,listOf(value,added),30000);store.directorySync.observe(source,listOf(value,added),40000)
        assertEquals(51L,commit(source,added).sourceVersion)
    }
    @Test fun directoryPathsArePortableAndNeverSilentlyNormalized() {
        for(path in listOf("../bad","/root","a//b","a\\b","a/.mirelay-history","C:/bad","a/\u0000bad"," ","a/".repeat(18))) assertThrows(IllegalArgumentException::class.java){DirectoryPaths.validate(path)}
        DirectoryPaths.validate("Pixiv/画师/ original.png")
    }
    @Test fun schemaThreeUpgradeKeepsPairingPreparedSourceAndLegacyReceipt() {
        store.close();val context=RuntimeEnvironment.getApplication();context.deleteDatabase("relay.db")
        android.database.sqlite.SQLiteDatabase.openOrCreateDatabase(context.getDatabasePath("relay.db"),null).use {db->
            db.execSQL("CREATE TABLE folders(id TEXT PRIMARY KEY,name TEXT NOT NULL,server TEXT NOT NULL,insecure INTEGER NOT NULL,token TEXT NOT NULL,pairing_state TEXT NOT NULL,verification TEXT)")
            db.execSQL("CREATE TABLE transfers(id TEXT PRIMARY KEY,folder_id TEXT NOT NULL,name TEXT NOT NULL,size INTEGER NOT NULL,status TEXT NOT NULL,uploaded INTEGER NOT NULL DEFAULT 0,error TEXT,delivery_id TEXT,work_id TEXT NOT NULL DEFAULT '',created_at INTEGER NOT NULL,auto_revision TEXT,auto_unmetered INTEGER NOT NULL DEFAULT 0)")
            AutoStore.createTables(db)
            db.execSQL("INSERT INTO folders VALUES(?, 'Existing',?,0,'cipher:old-secret','ready','123456abcdef')",arrayOf(folder.id,folder.server))
            db.execSQL("INSERT INTO auto_sources(folder_id,tree_uri,name,revision,enabled,unmetered,prepared) VALUES(?,?,'Source','old-revision',0,1,1)",arrayOf(folder.id,tree))
            db.execSQL("INSERT INTO transfers(id,folder_id,name,size,status,uploaded,delivery_id,created_at) VALUES('old-transfer',?,'old.bin',42,'UPLOADED',42,'old-receipt',1)",arrayOf(folder.id))
            db.version=3
        }
        store=RelayStore(context,cipher);store.refresh()
        assertEquals(5,store.readableDatabase.version);assertEquals("old-secret",store.token(folder.id));assertEquals("ready",store.folder(folder.id)!!.pairingState)
        assertTrue(store.automatic.source(folder.id)!!.prepared);assertFalse(store.automatic.source(folder.id)!!.directorySync)
        assertEquals("old-receipt",store.transfer("old-transfer")!!.deliveryId);assertNull(store.transfer("old-transfer")!!.relativePath)
    }
    @Test fun changedPrivatePayloadIsRejectedBeforeNativeUpload() {
        val value=file();val source=start(listOf(value));val transfer=commit(source,value)
        val payload=java.io.File.createTempFile("directory-payload-",".bin")
        try {payload.writeBytes(ByteArray(42){1});assertThrows(IllegalArgumentException::class.java){verifyDirectoryPayload(payload,transfer)}} finally {payload.delete()}
    }
}

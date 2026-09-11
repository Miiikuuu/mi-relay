package io.mirelay.android

import android.app.Application
import android.database.sqlite.SQLiteDatabase
import android.net.Uri
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import java.util.UUID
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
class AutoStoreTest {
    private lateinit var store: RelayStore
    private lateinit var folder: String
    private val cipher = object : TokenCipher {
        override fun seal(id: String, token: String) = "cipher:$token"
        override fun open(id: String, value: String) = value.removePrefix("cipher:")
    }
    @Before fun setup() {
        val context = RuntimeEnvironment.getApplication()
        context.deleteDatabase("relay.db")
        store = RelayStore(context, cipher)
        folder = store.saveFolder(null, "Auto test", "https://example.com", "secret", false)
    }
    @After fun close() { store.close() }
    private fun file(id: String = "new", size: Long? = 42, modified: Long? = 1) = SourceFile(id, Uri.parse("content://test/tree/root/document/$id"), "$id.bin", size, modified)
    private fun enable(files: List<SourceFile> = emptyList(), target: String = folder) = store.automatic.enable(target, "content://test/tree/root", "Source", true, files, 0)
    private fun staged(hash: String = "hash") = StagedFile(UUID.randomUUID().toString(), "new.bin", 42, hash)
    @Test fun sourceIssueCountsTowardAttentionPersistsAndClearsAfterSuccessfulScan() {
        val source = enable()
        store.automatic.observe(source, listOf(file("empty", 0)), 1)
        store.automatic.scanResult(source, 2, "Source check failed.")
        store.close()
        store = RelayStore(RuntimeEnvironment.getApplication(), cipher)
        val failed = store.automatic.source(folder)!!
        assertEquals(1, failed.skipped); assertEquals(2, failed.attentionCount)
        assertEquals("1 waiting · 2 need attention (includes a source issue)", failed.attentionSummary)
        store.automatic.scanResult(source, 3)
        val recovered = store.automatic.source(folder)!!
        assertNull(recovered.error); assertEquals(1, recovered.attentionCount)
        assertEquals("1 waiting · 1 need attention", recovered.attentionSummary)
        assertEquals("0 waiting · 0 need attention", recovered.copy(waiting = 0, skipped = 0).attentionSummary)
    }
    @Test fun preparedDirectoryDoesNotSendAndKeepsOriginalBaselineAfterActivation() {
        val old = file("historical")
        val source = store.automatic.enable(folder, "content://test/tree/root", "Pixiv", true, listOf(old), 0,
            startImmediately = false)
        assertFalse(source.enabled); assertTrue(source.prepared)
        assertTrue(store.transfers.value.isEmpty())
        val activated = store.automatic.startPrepared(folder, false)
        assertTrue(activated.enabled); assertFalse(activated.prepared); assertFalse(activated.unmetered)
        val fresh = file("arrived-during-pairing")
        assertTrue(store.automatic.observe(activated, listOf(old, fresh), 100).isEmpty())
        assertEquals(listOf(fresh), store.automatic.observe(activated, listOf(old, fresh), 10100))
        assertEquals(source.revision, activated.revision)
    }
    @Test fun existingFilesRequireConsentAndAStableSecondObservation() {
        val existing = file("historical")
        val source = store.automatic.enable(folder, "content://test/tree/root", "Pixiv", true, listOf(existing), 100,
            includeExisting = true, startImmediately = false)
        assertTrue(store.transfers.value.isEmpty())
        val activated = store.automatic.startPrepared(folder, true)
        assertTrue(store.automatic.observe(activated, listOf(existing), 101).isEmpty())
        assertEquals(listOf(existing), store.automatic.observe(activated, listOf(existing), 10100))
        assertTrue(store.automatic.commit(activated, existing, staged()))
        store.refresh(); assertEquals(1, store.transfers.value.size)
        assertTrue(store.automatic.observe(activated, listOf(existing), 50000).isEmpty())
        assertEquals(source.revision, activated.revision)
    }
    @Test fun initializedDirectoryAndPendingPairingSurviveReopenWithoutStartingAuto() {
        store.pairingResult(folder, "awaiting_confirmation", "123456abcdef")
        store.automatic.enable(folder, "content://test/tree/root", "Pixiv", true, listOf(file("old")), 0,
            includeExisting = true, startImmediately = false)
        store.close()
        store = RelayStore(RuntimeEnvironment.getApplication(), cipher)
        store.refresh()
        assertEquals("awaiting_confirmation", store.folder(folder)!!.pairingState)
        assertEquals("123456abcdef", store.folder(folder)!!.verification)
        assertTrue(store.automatic.source(folder)!!.prepared)
        assertFalse(store.automatic.source(folder)!!.enabled)
        assertTrue(store.transfers.value.isEmpty())
    }
    @Test fun versionTwoMigrationKeepsEnabledSourceBaselineAndCredential() {
        store.close()
        val context = RuntimeEnvironment.getApplication()
        context.deleteDatabase("relay.db")
        SQLiteDatabase.openOrCreateDatabase(context.getDatabasePath("relay.db"), null).use { db ->
            db.execSQL("CREATE TABLE folders (id TEXT PRIMARY KEY,name TEXT NOT NULL,server TEXT NOT NULL,insecure INTEGER NOT NULL,token TEXT NOT NULL)")
            db.execSQL("CREATE TABLE transfers (id TEXT PRIMARY KEY,folder_id TEXT NOT NULL,name TEXT NOT NULL,size INTEGER NOT NULL,status TEXT NOT NULL,uploaded INTEGER NOT NULL DEFAULT 0,error TEXT,delivery_id TEXT,work_id TEXT NOT NULL DEFAULT '',created_at INTEGER NOT NULL,auto_revision TEXT,auto_unmetered INTEGER NOT NULL DEFAULT 0)")
            db.execSQL("CREATE TABLE auto_sources (folder_id TEXT PRIMARY KEY,tree_uri TEXT NOT NULL,name TEXT NOT NULL,revision TEXT NOT NULL,enabled INTEGER NOT NULL,unmetered INTEGER NOT NULL,last_scan INTEGER,error TEXT)")
            db.execSQL("CREATE TABLE auto_files (folder_id TEXT NOT NULL,document_id TEXT NOT NULL,fingerprint TEXT NOT NULL,stable_since INTEGER NOT NULL,state TEXT NOT NULL,transfer_id TEXT,error TEXT,PRIMARY KEY(folder_id,document_id))")
            db.execSQL("CREATE TABLE auto_hashes (folder_id TEXT NOT NULL,sha256 TEXT NOT NULL,transfer_id TEXT NOT NULL,PRIMARY KEY(folder_id,sha256))")
            db.execSQL("INSERT INTO folders VALUES(?, 'Old Folder','https://example.com',0,'cipher:old-secret')", arrayOf(folder))
            db.execSQL("INSERT INTO auto_sources VALUES(?, 'content://test/tree/root','Old source','old-revision',1,1,123,NULL)", arrayOf(folder))
            db.execSQL("INSERT INTO auto_files VALUES(?, 'old','42:1:old.bin',123,'BASELINE',NULL,NULL)", arrayOf(folder))
            db.version = 2
        }
        store = RelayStore(context, cipher); store.refresh()
        assertEquals(5, store.readableDatabase.version)
        assertEquals("old-secret", store.token(folder)); assertEquals("legacy", store.folder(folder)!!.pairingState)
        val source = store.automatic.source(folder)!!
        assertTrue(source.enabled); assertFalse(source.prepared); assertEquals("old-revision", source.revision)
        assertTrue(store.automatic.observe(source, listOf(file("old")), 20000).isEmpty())
    }
    private fun ready(source: AutoSource, file: SourceFile = file()) {
        assertTrue(store.automatic.observe(source, listOf(file), 0).isEmpty())
        assertEquals(listOf(file), store.automatic.observe(source, listOf(file), AutoStore.STABLE_MILLIS))
    }
    @Test fun baselineSkipsHistoryAndEditsButRequiresTwoObservationsForNewFiles() {
        val old = file("old"); val fresh = file()
        val source = enable(listOf(old))
        val entries = listOf(old.copy(modified = 2), fresh)
        assertTrue(store.automatic.observe(source, entries, 1).isEmpty())
        assertTrue(store.automatic.observe(source, entries, 10000).isEmpty())
        assertEquals(listOf(fresh), store.automatic.observe(source, entries, 10001))
    }
    @Test fun changesAndBackwardClockRestartStabilityWindow() {
        val source = enable(); val original = file(); val changed = original.copy(size = 43)
        store.automatic.observe(source, listOf(original), 100)
        assertTrue(store.automatic.observe(source, listOf(changed), 20000).isEmpty())
        assertTrue(store.automatic.observe(source, listOf(changed), 50).isEmpty())
        assertEquals(listOf(changed), store.automatic.observe(source, listOf(changed), 10050))
    }
    @Test fun disappearingAndReturningFileMustSettleAgain() {
        val source = enable(); val file = file()
        store.automatic.observe(source, listOf(file), 0)
        store.automatic.observe(source, emptyList(), 10000)
        assertTrue(store.automatic.observe(source, listOf(file), 30000).isEmpty())
        assertEquals(listOf(file), store.automatic.observe(source, listOf(file), 40000))
    }
    @Test fun unsupportedMetadataNeverBecomesReadyOrTriggersSettleLoop() {
        val source = enable()
        val files = listOf(file("empty", 0), file("large", MAX_FILE_BYTES + 1), file("unknown", null), file("time", modified = null))
        store.automatic.observe(source, files, 0)
        assertTrue(store.automatic.observe(source, files, 99999).isEmpty())
        assertFalse(store.automatic.needsSettle(source))
        assertEquals(4, store.automatic.source(folder)!!.skipped)
    }
    @Test fun concurrentCommitsCreateOneTransferAndOneHashReceipt() {
        val source = enable(); val file = file(); ready(source, file)
        val pool = Executors.newFixedThreadPool(4)
        try {
            val results = (1..8).map { pool.submit<Boolean> { store.automatic.commit(source, file, staged()) } }.map { it.get(10, TimeUnit.SECONDS) }
            assertEquals(1, results.count { it })
            store.refresh(); assertEquals(1, store.transfers.value.size)
        } finally { pool.shutdownNow() }
    }
    @Test fun sameHashAcrossDifferentDocumentsIsDeduplicatedWithinFolder() {
        val source = enable(); val one = file(); val two = file("copy")
        store.automatic.observe(source, listOf(one, two), 0)
        assertTrue(store.automatic.commit(source, one, staged()))
        assertFalse(store.automatic.commit(source, two, staged()))
        val other = store.saveFolder(null, "Other", "https://example.com", "other", false)
        val otherSource = enable(target = other); ready(otherSource)
        assertTrue(store.automatic.commit(otherSource, one, staged()))
        store.refresh(); assertEquals(2, store.transfers.value.size)
    }
    @Test fun pauseInvalidatesQueuedAndRunningAutomaticWorkersButNotManualFiles() {
        val source = enable(); ready(source); val staged = staged()
        store.automatic.commit(source, file(), staged); store.assign(staged.id, "auto-worker")
        store.updateOwned(staged.id, "auto-worker", TransferStatus.UPLOADING, 10)
        val manual = UUID.randomUUID().toString(); store.addTransfer(manual, folder, "manual", 42); store.assign(manual, "manual-worker")
        store.automatic.disable(folder)
        assertEquals(TransferStatus.PAUSED, store.transfer(staged.id)!!.status)
        assertFalse(store.updateOwned(staged.id, "auto-worker", TransferStatus.UPLOADED, 42, delivery = "stale"))
        assertThrows(IllegalStateException::class.java) { store.assign(staged.id, "late") }
        assertTrue(store.updateOwned(manual, "manual-worker", TransferStatus.UPLOADED, 42, delivery = "manual"))
        store.automatic.makeManual(staged.id); store.assign(staged.id, "explicit-resume")
        assertNull(store.transfer(staged.id)!!.autoRevision)
    }
    @Test fun staleRevisionCannotCommitOrSetErrorsOnReenabledSource() {
        val old = enable(); ready(old); store.automatic.disable(folder)
        val current = enable(); ready(current)
        assertFalse(store.automatic.commit(old, file(), staged()))
        store.automatic.scanResult(old, 5, "stale")
        store.automatic.fileError(old, file(), "stale", 5)
        assertNull(store.automatic.source(folder)!!.error)
        assertEquals(0, store.automatic.source(folder)!!.skipped)
        assertTrue(store.automatic.commit(current, file(), staged()))
    }
    @Test fun incompleteInvalidOrDuplicateBaselineNeverEnables() {
        assertThrows(IllegalArgumentException::class.java) { enable(listOf(file(), file())) }
        assertNull(store.automatic.source(folder))
        assertThrows(IllegalArgumentException::class.java) { enable((0..AutoStore.MAX_ENTRIES).map { file("$it") }) }
        assertNull(store.automatic.source(folder))
    }
    @Test fun reenablingSkipsFilesAddedWhilePausedAndRetainsContentDeduplication() {
        val old = enable(); ready(old); store.automatic.commit(old, file(), staged())
        store.automatic.disable(folder)
        val duringPause = file("while-paused"); val current = enable(listOf(duringPause))
        assertTrue(store.automatic.observe(current, listOf(duringPause), 100000).isEmpty())
        val copy = file("copy"); ready(current, copy)
        assertFalse(store.automatic.commit(current, copy, staged()))
    }
    @Test fun versionOneMigrationPreservesCredentialsReceiptAndManualOwnership() {
        store.close()
        val context = RuntimeEnvironment.getApplication()
        context.deleteDatabase("relay.db")
        SQLiteDatabase.openOrCreateDatabase(context.getDatabasePath("relay.db"), null).use { db ->
            db.execSQL("CREATE TABLE folders (id TEXT PRIMARY KEY, name TEXT NOT NULL, server TEXT NOT NULL, insecure INTEGER NOT NULL, token TEXT NOT NULL)")
            db.execSQL("CREATE TABLE transfers (id TEXT PRIMARY KEY, folder_id TEXT NOT NULL REFERENCES folders(id), name TEXT NOT NULL, size INTEGER NOT NULL, status TEXT NOT NULL, uploaded INTEGER NOT NULL DEFAULT 0, error TEXT, delivery_id TEXT, work_id TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL)")
            db.execSQL("INSERT INTO folders VALUES ('legacy', 'Legacy', 'https://example.com', 0, 'cipher:secret')")
            db.execSQL("INSERT INTO transfers VALUES ('receipt', 'legacy', 'old.bin', 42, 'UPLOADED', 42, NULL, 'delivery', 'owner', 123)")
            db.version = 1
        }
        store = RelayStore(context, cipher); store.refresh()
        assertEquals("secret", store.token("legacy")); assertEquals("delivery", store.transfer("receipt")!!.deliveryId)
        assertEquals("owner", store.transfer("receipt")!!.workId); assertNull(store.transfer("receipt")!!.autoRevision)
        assertEquals(5, store.readableDatabase.version); assertTrue(store.automatic.sources.value.isEmpty())
    }
}

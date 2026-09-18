package io.mirelay.android

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.os.OperationCanceledException
import android.provider.DocumentsContract
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import androidx.work.NetworkType
import androidx.work.WorkManager
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.UUID

internal object AutoFixture {
    val tree: Uri = DocumentsContract.buildTreeDocumentUri("io.mirelay.android.test.auto", "root")
    fun control(operation: String, args: Bundle = Bundle()) {
        DeviceSupport.app.contentResolver.call(Uri.parse("content://io.mirelay.android.test.fixtures"), "auto", operation, args)
    }
    fun opens(): Int = checkNotNull(DeviceSupport.app.contentResolver.call(
        Uri.parse("content://io.mirelay.android.test.fixtures"), "auto", "stats", Bundle()
    )).getInt("opens")
    fun put(id: String, size: Long = 12345, parent: String = "root", extra: Bundle.() -> Unit = {}) = control("put", Bundle().apply {
        putString("id", id); putLong("size", size); putString("parent", parent); extra()
    })
    fun mode(value: String) = control("mode", Bundle().apply { putString("mode", value) })
    fun reset() { control("revoke"); control("reset"); control("grant") }
}

@RunWith(AndroidJUnit4::class)
class DeviceAutoTest {
    private val app get() = DeviceSupport.app
    private val manager get() = WorkManager.getInstance(app)
    @Before fun setup() { DeviceSupport.reset(); AutoFixture.reset() }
    @After fun close() {
        app.store.automatic.sources.value.filter { it.enabled }.forEach { app.auto.pause(it.folderId) }
        AutoFixture.control("revoke"); AutoFixture.control("reset")
    }
    private fun enable(): AutoSource {
        val folder = DeviceSupport.folder(); app.auto.enable(folder, AutoFixture.tree, true)
        return app.store.automatic.source(folder)!!
    }
    private fun scan(source: AutoSource, time: Long) = AutoSession().use { app.auto.scan(source, it) { time } }
    @Test fun baselineIsReadOnlyRecursiveAndOnlyOnePeriodicJobIsRegistered() {
        AutoFixture.put("old"); AutoFixture.put("nested", extra = { putBoolean("directory", true) })
        AutoFixture.put("nested-old", parent = "nested")
        val source = enable()
        val permission = app.contentResolver.persistedUriPermissions.single { it.uri == AutoFixture.tree }
        assertTrue(permission.isReadPermission); assertFalse(permission.isWritePermission)
        scan(source, 0); scan(source, 20000)
        assertTrue(app.store.transfers.value.isEmpty())
        repeat(3) { app.auto.recover() }
        val jobs = manager.getWorkInfosForUniqueWork("auto-periodic:${source.folderId}").get().filter { !it.state.isFinished }
        assertEquals(1, jobs.size)
        assertEquals(NetworkType.UNMETERED, jobs.single().constraints.requiredNetworkType)
        assertTrue(jobs.single().constraints.requiresBatteryNotLow()); assertTrue(jobs.single().constraints.requiresStorageNotLow())
        assertEquals(30 * 60 * 1000L, jobs.single().periodicityInfo!!.repeatIntervalMillis)
    }
    @Test fun automaticWorkerFindsNewNestedFileUploadsInBackgroundAndDeduplicatesCopies() {
        AutoFixture.put("historical", 97); AutoFixture.put("nested", extra = { putBoolean("directory", true) })
        val source = enable()
        AutoFixture.put("new-text", 12345, "nested", extra = { putString("name", "Auto notes.txt") })
        // No Activity and no Send action: a real scheduled check plus its bounded
        // stability follow-up must stage and upload while the launcher is visible.
        UiDevice.getInstance(InstrumentationRegistry.getInstrumentation()).pressHome()
        app.auto.checkNow(source.folderId)
        DeviceSupport.await(90000) { app.store.transfers.value.singleOrNull()?.status == TransferStatus.UPLOADED }
        val uploaded = app.store.transfers.value.single()
        assertEquals("Auto notes.txt", uploaded.name); assertNotNull(uploaded.deliveryId)
        assertEquals(source.revision, uploaded.autoRevision)
        // Completion is durably published before the worker deletes staging.
        // Fast persistent connections make that intentional window observable.
        DeviceSupport.await(5000) { !File(app.store.directory(uploaded.id), "payload").exists() }
        AutoFixture.put("same-content-new-name", 12345)
        scan(source, 100000); scan(source, 120000)
        assertEquals(1, app.store.transfers.value.size)
        val uri = DocumentsContract.buildDocumentUriUsingTree(AutoFixture.tree, "new-text")
        assertEquals(12345, app.contentResolver.openInputStream(uri)!!.use { it.readBytes().size })
    }
    @Test fun incompleteLoadingErrorAndDuplicateListingsNeverEnableOrSendHistory() {
        AutoFixture.put("historical")
        val folder = DeviceSupport.folder()
        for (mode in listOf("loading", "error", "duplicate")) {
            AutoFixture.control("grant"); AutoFixture.mode(mode)
            assertThrows(Exception::class.java) { app.auto.enable(folder, AutoFixture.tree, true) }
            assertNull(app.store.automatic.source(folder)); assertTrue(app.store.transfers.value.isEmpty())
            assertFalse(app.contentResolver.persistedUriPermissions.any { it.uri == AutoFixture.tree })
        }
    }
    @Test fun oversizedAndDeepSourcesAreRejectedWithoutPartialBaseline() {
        var parent = "root"
        repeat(18) { i -> val id = "dir-$i"; AutoFixture.put(id, parent = parent, extra = { putBoolean("directory", true) }); parent = id }
        val folder = DeviceSupport.folder()
        assertThrows(IllegalArgumentException::class.java) { app.auto.enable(folder, AutoFixture.tree, true) }
        assertNull(app.store.automatic.source(folder))
        AutoFixture.reset()
        repeat(AutoStore.MAX_ENTRIES + 1) { AutoFixture.put("file-$it", 1) }
        assertThrows(IllegalArgumentException::class.java) { app.auto.enable(folder, AutoFixture.tree, true) }
        assertNull(app.store.automatic.source(folder))
        AutoFixture.control("remove", Bundle().apply { putString("id", "file-${AutoStore.MAX_ENTRIES}") })
        AutoFixture.control("grant")
        val started = android.os.SystemClock.elapsedRealtime()
        app.auto.enable(folder, AutoFixture.tree, true)
        val baselineMillis = android.os.SystemClock.elapsedRealtime() - started
        val source = app.store.automatic.source(folder)!!
        val scanStart = android.os.SystemClock.elapsedRealtime()
        scan(source, 100000)
        android.util.Log.i("AutoQA", "5000 local synthetic entries: baseline_ms=$baselineMillis unchanged_scan_ms=${android.os.SystemClock.elapsedRealtime() - scanStart}")
        assertTrue(app.store.transfers.value.isEmpty())
    }
    @Test fun changingDuringReadNeverQueuesAndCleansTemporaryPayload() {
        val source = enable()
        AutoFixture.put("still-writing", extra = { putBoolean("mutateOnRead", true) })
        val root = File(app.noBackupFilesDir, "outgoing")
        val before = root.list()?.toSet().orEmpty()
        scan(source, 0); scan(source, 20000)
        assertTrue(app.store.transfers.value.isEmpty())
        assertEquals(1, app.store.automatic.source(source.folderId)!!.skipped)
        assertEquals(before, root.list()?.toSet().orEmpty())
    }
    @Test fun virtualUnknownEmptyAndOversizedFilesDoNotLoopOrUpload() {
        val source = enable()
        AutoFixture.put("virtual", extra = { putBoolean("virtual", true) })
        AutoFixture.put("no-time", extra = { putBoolean("unknownTime", true) })
        AutoFixture.put("no-size", extra = { putBoolean("unknownSize", true) })
        AutoFixture.put("empty", 0); AutoFixture.put("too-large", MAX_FILE_BYTES + 1)
        assertFalse(scan(source, 0)); assertFalse(scan(source, 20000))
        assertEquals(5, app.store.automatic.source(source.folderId)!!.skipped)
        assertTrue(app.store.transfers.value.isEmpty())
    }
    @Test fun revokedPermissionPausesAutoThroughRealWorkerAndKeepsManualQueue() {
        val source = enable()
        val manual = DeviceSupport.import(source.folderId, 511)
        AutoFixture.control("revoke")
        app.auto.checkNow(source.folderId)
        DeviceSupport.await(30000) { app.store.automatic.source(source.folderId)?.enabled == false }
        assertTrue(app.store.automatic.source(source.folderId)!!.error!!.contains("revoked"))
        assertEquals(TransferStatus.QUEUED, app.store.transfer(manual)!!.status)
    }
    @Test fun pauseCancelsAllAutoJobsAndStaleFailureCannotDisableNewRevision() {
        val old = enable(); AutoFixture.put("new")
        app.auto.checkNow(old.folderId); app.auto.pause(old.folderId)
        assertTrue(manager.getWorkInfosByTag("auto:${old.folderId}").get().all { it.state.isFinished })
        assertFalse(scan(old, 100000)); assertTrue(app.store.transfers.value.isEmpty())
        app.auto.enable(old.folderId, AutoFixture.tree, true)
        val fresh = app.store.automatic.source(old.folderId)!!
        app.auto.pauseIfCurrent(old, "stale permission failure")
        assertTrue(app.store.automatic.source(old.folderId)!!.enabled)
        assertNotEquals(old.revision, fresh.revision)
        scan(fresh, 0); scan(fresh, 20000)
        assertTrue(app.store.transfers.value.isEmpty()) // added while paused = new baseline
    }
    @Test fun cancelledScanCannotCommitAndGrantDoesNotAllowParentAccess() {
        val source = enable()
        AutoSession(stopped = { true }).use { session ->
            assertThrows(OperationCanceledException::class.java) { app.auto.scan(source, session) }
        }
        assertTrue(app.store.transfers.value.isEmpty())
        val outside = DocumentsContract.buildDocumentUri("io.mirelay.android.test.auto", "outside")
        assertThrows(SecurityException::class.java) { app.contentResolver.query(outside, null, null, null, null)?.close() }
        val writable = DocumentsContract.buildDocumentUriUsingTree(AutoFixture.tree, "root")
        assertThrows(SecurityException::class.java) { app.contentResolver.openOutputStream(writable)?.close() }
    }
    @Test fun committedButUnscheduledAutomaticFileRecoversWithItsNetworkConstraints() {
        val source = enable(); AutoFixture.put("recovery", 12347)
        val file = AutoSession().use { DirectorySource(app.contentResolver).snapshot(AutoFixture.tree, it).files.single() }
        app.store.automatic.observe(source, listOf(file), 0)
        val id = FileImporter(app.contentResolver, app.store).stage(file.uri, source.folderId) { app.store.automatic.commit(source, file, it) }!!
        assertTrue(manager.getWorkInfosForUniqueWork("upload:$id").get().isEmpty())
        app.uploads.recover(source)
        val work = manager.getWorkInfosForUniqueWork("upload:$id").get().single()
        assertEquals(NetworkType.UNMETERED, work.constraints.requiredNetworkType)
        assertTrue(work.constraints.requiresBatteryNotLow()); assertTrue(work.constraints.requiresStorageNotLow())
        DeviceSupport.await(60000) { app.store.transfer(id)?.status == TransferStatus.UPLOADED }
    }
    @Test fun revokedPermissionAlsoPreventsAlreadyStagedAutomaticUpload() {
        val source = enable(); AutoFixture.put("revoke-before-upload", 12348)
        val file = AutoSession().use { DirectorySource(app.contentResolver).snapshot(AutoFixture.tree, it).files.single() }
        app.store.automatic.observe(source, listOf(file), 0)
        val id = FileImporter(app.contentResolver, app.store).stage(file.uri, source.folderId) { app.store.automatic.commit(source, file, it) }!!
        AutoFixture.control("revoke")
        app.uploads.enqueue(id)
        DeviceSupport.await(30000) { app.store.transfer(id)?.status == TransferStatus.PAUSED }
        assertNull(app.store.transfer(id)!!.deliveryId); assertFalse(app.store.automatic.source(source.folderId)!!.enabled)
        assertTrue(File(app.store.directory(id), "payload").exists())
    }
    @Test fun pauseBetweenStagingAndEnqueueWinsUntilExplicitManualResume() {
        val source = enable(); AutoFixture.put("pause-before-enqueue", 12346)
        val file = AutoSession().use { DirectorySource(app.contentResolver).snapshot(AutoFixture.tree, it).files.single() }
        app.store.automatic.observe(source, listOf(file), 0)
        val id = FileImporter(app.contentResolver, app.store).stage(file.uri, source.folderId) { app.store.automatic.commit(source, file, it) }!!
        app.uploads.pause(id)
        app.uploads.enqueue(id); app.uploads.recover(source)
        assertEquals(TransferStatus.PAUSED, app.store.transfer(id)!!.status)
        assertTrue(manager.getWorkInfosForUniqueWork("upload:$id").get().isEmpty())
        app.auto.pause(source.folderId)
        app.uploads.enqueue(id, manual = true)
        assertNull(app.store.transfer(id)!!.autoRevision)
        assertEquals(NetworkType.CONNECTED, manager.getWorkInfosForUniqueWork("upload:$id").get().single().constraints.requiredNetworkType)
        DeviceSupport.await(60000) { app.store.transfer(id)?.status == TransferStatus.UPLOADED }
    }
}

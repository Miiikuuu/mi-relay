package io.mirelay.android

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

/** Host harness kills/reopens the app between these explicitly selected tests. */
@RunWith(AndroidJUnit4::class)
class RecoverySeedTest {
    @Test fun seedInterruptedUpload() {
        DeviceSupport.reset()
        val folder = DeviceSupport.folder(url = DeviceSupport.server + "/slow")
        val id = DeviceSupport.import(folder, 16 * 1024 * 1024, "process-recovery.bin")
        DeviceSupport.app.store.refresh()
        assertEquals(TransferStatus.QUEUED, DeviceSupport.app.store.transfer(id)?.status)
        // Deliberately no WorkManager request: startup recovery must schedule it.
    }
    @Test fun verifyRecoveredUpload() {
        DeviceSupport.guard()
        val files = DeviceSupport.app.store.apply { refresh() }.transfers.value
        assertEquals(1, files.size)
        assertEquals(files.single().error, TransferStatus.UPLOADED, files.single().status)
        assertNotNull(files.single().deliveryId)
        assertFalse(java.io.File(DeviceSupport.app.store.directory(files.single().id), "payload").exists())
    }
    @Test fun seedInterruptedAutomaticUpload() {
        DeviceSupport.reset(); AutoFixture.reset()
        val app = DeviceSupport.app
        val folder = DeviceSupport.folder(url = DeviceSupport.server + "/slow")
        app.auto.enable(folder, AutoFixture.tree, true)
        val source = app.store.automatic.source(folder)!!
        AutoFixture.put("auto-process-recovery", 16 * 1024 * 1024, extra = { putString("name", "auto-process-recovery.bin") })
        val file = AutoSession().use { DirectorySource(app.contentResolver).snapshot(AutoFixture.tree, it).files.single() }
        app.store.automatic.observe(source, listOf(file), 0)
        app.store.automatic.observe(source, listOf(file), 20000)
        val id = FileImporter(app.contentResolver, app.store).stage(file.uri, source.folderId) { app.store.automatic.commit(source, file, it) }!!
        assertEquals(source.revision, app.store.transfer(id)!!.autoRevision)
        assertEquals(TransferStatus.QUEUED, app.store.transfer(id)!!.status)
        // Simulates process death after the atomic queue commit, before enqueue.
    }
    @Test fun verifyRecoveredAutomaticUpload() {
        verifyRecoveredUpload()
        val app = DeviceSupport.app
        val source = app.store.automatic.sources.value.single()
        assertTrue(source.enabled); assertEquals(source.revision, app.store.transfers.value.single().autoRevision)
        assertTrue(app.contentResolver.persistedUriPermissions.any { it.uri == AutoFixture.tree && it.isReadPermission })
        val jobs = androidx.work.WorkManager.getInstance(app).getWorkInfosForUniqueWork("auto-periodic:${source.folderId}").get()
        assertEquals(1, jobs.count { !it.state.isFinished })
        app.auto.pause(source.folderId); AutoFixture.control("revoke")
    }
}

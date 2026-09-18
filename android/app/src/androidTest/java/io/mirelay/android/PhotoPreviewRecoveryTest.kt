package io.mirelay.android

import android.os.Process
import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

/** Host harness force-stops between phases; unlike Activity recreation this
 * proves the bitmap did not survive only in the process-wide memory cache. */
@RunWith(AndroidJUnit4::class)
class PhotoPreviewRecoveryTest {
    @Test fun seedCompletedImage() {
        DeviceSupport.reset()
        val app = DeviceSupport.app
        val folder = DeviceSupport.folder()
        val id = DeviceSupport.image(folder, "Uploaded-original.png")
        val dir = app.store.directory(id)
        val payload = File(dir, "payload")
        app.uploads.enqueue(id, manual = true)
        DeviceSupport.await(60000) { app.store.transfer(id)?.status == TransferStatus.UPLOADED && !payload.exists() }
        assertNotNull(runBlocking { PhotoThumbnails.load(app.store, id, 256) })
        File(app.cacheDir, "photo-recovery-pid").writeText(Process.myPid().toString())
    }

    @Test fun verifyPreviewInNewProcess() {
        DeviceSupport.guard()
        val app = DeviceSupport.app
        assertNotEquals(File(app.cacheDir, "photo-recovery-pid").readText(), Process.myPid().toString())
        val transfer = app.store.apply { refresh() }.transfers.value.single()
        assertEquals(TransferStatus.UPLOADED, transfer.status)
        assertNotNull(transfer.deliveryId)
        assertFalse(File(app.store.directory(transfer.id), "payload").exists())
        assertFalse(File(app.store.directory(transfer.id), "resume.json").exists())
        for (target in listOf(256, 1024)) {
            val expected = app.resources.openRawResource(R.drawable.mirelay_brand_icon).use { input ->
                android.graphics.BitmapFactory.decodeStream(input)
            }
            val actual = runBlocking { PhotoThumbnails.load(app.store, transfer.id, target) }!!
            assertTrue(actual.width <= target && actual.height <= target)
            if (target == 1024) assertTrue("Persistent preview must retain source pixels", expected.sameAs(actual))
            expected.recycle()
        }
    }
}

package io.mirelay.android

import android.Manifest
import android.content.pm.PackageManager
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class NotificationDeniedTest {
    @get:Rule val ui = createAndroidComposeRule<MainActivity>()
    @Test fun deniedNotificationsStillPermitForegroundUpload() {
        DeviceSupport.reset()
        val app = DeviceSupport.app
        assertEquals(PackageManager.PERMISSION_DENIED, app.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS))
        val folder = DeviceSupport.folder()
        val id = DeviceSupport.import(folder, name = "notification-denied.bin")
        app.uploads.enqueue(id)
        DeviceSupport.await(45000) { app.store.transfer(id)?.status in listOf(TransferStatus.UPLOADED, TransferStatus.FAILED) }
        assertEquals(app.store.transfer(id)?.error, TransferStatus.UPLOADED, app.store.transfer(id)?.status)
    }
}

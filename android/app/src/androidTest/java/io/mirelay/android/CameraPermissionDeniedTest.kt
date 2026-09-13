package io.mirelay.android

import android.Manifest
import android.content.pm.PackageManager
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class CameraPermissionDeniedTest {
    @get:Rule val ui = createAndroidComposeRule<MainActivity>()
    @Test fun refusingCameraKeepsManualPairingAndDoesNotCreateFolder() {
        DeviceSupport.reset()
        val app = DeviceSupport.app
        assertEquals(PackageManager.PERMISSION_DENIED, app.checkSelfPermission(Manifest.permission.CAMERA))
        ui.onNode(hasText("Add Folder") and hasAnyAncestor(hasTestTag("welcome"))).performClick()
        ui.onNodeWithTag("scan-invitation").performScrollTo().performClick()
        val device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())
        val deny = device.wait(Until.findObject(By.res(java.util.regex.Pattern.compile(".*:id/permission_deny_button"))), 10000)
        checkNotNull(deny) { "Camera permission prompt did not appear" }.click()
        ui.onNodeWithTag("scan-error").performScrollTo().assertTextContains("Camera permission was not granted", substring=true)
        ui.onNodeWithText("Server URL").performScrollTo().performTextInput("https://manual.example.com")
        assertTrue(app.store.folders.value.isEmpty())
        assertEquals(PackageManager.PERMISSION_DENIED, app.checkSelfPermission(Manifest.permission.CAMERA))
    }
}

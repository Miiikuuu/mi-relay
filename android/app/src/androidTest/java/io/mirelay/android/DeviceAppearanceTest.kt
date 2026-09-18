package io.mirelay.android

import android.Manifest
import android.content.Intent
import android.graphics.Bitmap
import android.os.SystemClock
import android.provider.Settings
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import java.io.File
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

/** Real frame-clock test, deliberately without Compose's automatic idle clock:
 * an explicitly enabled infinite decoration is not an idle UI. QA AVD only. */
@RunWith(AndroidJUnit4::class)
class DeviceAppearanceTest {
    @Test fun backgroundMovesOnlyWhenEnabledAndSystemAnimationsAllowIt() {
        DeviceSupport.guard()
        DeviceSupport.reset()
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val app = DeviceSupport.app
        val device = UiDevice.getInstance(instrumentation)
        val store = AppearancePreferences(app)
        store.preferences.edit().clear().commit()
        val resolver = app.contentResolver
        val originalScale = Settings.Global.getString(resolver, Settings.Global.ANIMATOR_DURATION_SCALE)
        instrumentation.uiAutomation.adoptShellPermissionIdentity(Manifest.permission.WRITE_SECURE_SETTINGS)
        fun scale(value: String?) { Settings.Global.putString(resolver, Settings.Global.ANIMATOR_DURATION_SCALE, value) }
        fun click(text: String) {
            assertTrue("Missing $text", device.wait(Until.hasObject(By.textContains(text)), 10000))
            device.findObject(By.textContains(text)).click()
        }
        fun capture(name: String): Bitmap {
            val bitmap = instrumentation.uiAutomation.takeScreenshot() ?: error("Screenshot unavailable")
            val directory = File(app.filesDir, "visual-qa").apply { mkdirs() }
            File(directory, "$name.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
            return bitmap
        }
        fun changed(a: Bitmap, b: Bitmap): Int {
            assertEquals(a.width, b.width); assertEquals(a.height, b.height)
            var count = 0
            // Exclude system bars and clock. Sparse sampling bounds test work.
            for (y in 180 until a.height - 120 step 8) for (x in 24 until a.width - 24 step 8) {
                if (a.getPixel(x, y) != b.getPixel(x, y)) count++
            }
            return count
        }
        try {
            scale("1")
            instrumentation.startActivitySync(Intent(app, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK))
            assertTrue(device.wait(Until.hasObject(By.desc("Folders")), 10000))
            device.findObject(By.desc("Folders")).click()
            click("Settings")
            click("Background motion")
            click("Done")
            DeviceSupport.await { store.read().motion }
            SystemClock.sleep(1000)
            val moving = capture("ambient-on-first")
            SystemClock.sleep(1300)
            val moved = capture("ambient-on-second")
            try { assertTrue("Enabled background did not move", changed(moving, moved) > 20) }
            finally { moving.recycle(); moved.recycle() }
            // The ContentObserver must stop an already-running animation; a
            // startup-only read of the system setting would fail this check.
            scale("0")
            SystemClock.sleep(1000)
            val stopped = capture("ambient-system-off-first")
            SystemClock.sleep(1300)
            val still = capture("ambient-system-off-second")
            try { assertEquals("Background still drawing after system animation disable", 0, changed(stopped, still)) }
            finally { stopped.recycle(); still.recycle() }
            device.findObject(By.desc("Folders")).click()
            click("Settings"); click("Background motion"); click("Done")
            assertFalse(store.read().motion)
            scale("1")
            SystemClock.sleep(1000)
            val off = capture("ambient-user-off-first")
            SystemClock.sleep(1300)
            val offAgain = capture("ambient-user-off-second")
            try { assertEquals("User-disabled motion resumed", 0, changed(off, offAgain)) }
            finally { off.recycle(); offAgain.recycle() }
            assertTrue(app.store.transfers.value.isEmpty())
        } finally {
            scale(originalScale)
            instrumentation.uiAutomation.dropShellPermissionIdentity()
            store.motion(false)
        }
    }
}

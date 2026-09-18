package io.mirelay.android

import android.app.Application
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
class AppearancePreferencesTest {
    @Test fun oldTransparencyPreferenceAndUnrelatedKeysSurvive() {
        val context = RuntimeEnvironment.getApplication()
        val store = AppearancePreferences(context)
        store.preferences.edit().clear().putBoolean("reduce_transparency", true).putString("future_setting", "keep").commit()
        assertEquals(Appearance(false, true), store.read())
        store.motion(true)
        assertEquals(Appearance(true, true), AppearancePreferences(context).read())
        store.reduceTransparency(false)
        assertEquals(Appearance(true, false), AppearancePreferences(context).read())
        assertEquals("keep", store.preferences.getString("future_setting", null))
    }

    @Test fun missingOrWrongTypeDoesNotEnableMotionOrCrash() {
        val store = AppearancePreferences(RuntimeEnvironment.getApplication())
        store.preferences.edit().clear().commit()
        assertEquals(Appearance(), store.read())
        store.preferences.edit().putString("background_motion", "true").putInt("reduce_transparency", 1).commit()
        assertEquals(Appearance(), store.read())
    }
}

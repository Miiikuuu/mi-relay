package io.mirelay.android

import org.junit.Assert.*
import org.junit.Test

class AppearanceTest {
    private val enabled = Appearance(motion = true)
    private val foreground = AppearanceEnvironment(true, false, false, false)

    @Test fun animationRequiresEverySafetyCondition() {
        assertTrue(animateAmbient(enabled, foreground, true, true))
        assertFalse(animateAmbient(Appearance(), foreground, true, true))
        assertFalse(animateAmbient(enabled.copy(reduceTransparency = true), foreground, true, true))
        assertFalse(animateAmbient(enabled, foreground.copy(resumed = false), true, true))
        assertFalse(animateAmbient(enabled, foreground.copy(powerSave = true), true, true))
        assertFalse(animateAmbient(enabled, foreground.copy(reducedMotion = true), true, true))
        assertFalse(animateAmbient(enabled, foreground.copy(lowRam = true), true, true))
        assertFalse(animateAmbient(enabled, foreground, false, true))
        assertFalse(animateAmbient(enabled, foreground, true, false))
    }

    @Test fun disabledAndInvalidSystemScalesFailClosed() {
        for (scale in listOf(0f, -1f, Float.NaN, Float.POSITIVE_INFINITY)) assertTrue(systemMotionReduced(scale))
        for (scale in listOf(0.5f, 1f, 10f)) assertFalse(systemMotionReduced(scale))
    }

    @Test fun lightCurvesAreBoundedContinuousAndPeriodic() {
        for (period in listOf(12.0, 15.0, 14.0, 22.0)) {
            assertEquals(0f, ambientWave(0.0, period), 0.00001f)
            assertEquals(1f, ambientWave(period / 2, period), 0.00001f)
            assertEquals(ambientWave(0.3, period), ambientWave(period + 0.3, period), 0.00001f)
            for (i in 0..1000) assertTrue(ambientWave(i * 0.1, period) in 0f..1f)
        }
    }
}

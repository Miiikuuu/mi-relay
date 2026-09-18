package io.mirelay.android

import org.junit.Assert.*
import org.junit.Test

class ScanDeadlineTest {
    private var now = 0L
    private fun advance(millis: Long) { now += millis * 1_000_000 }

    @Test fun progressAllowsLongScansButIdleTimeStillExpires() {
        val deadline = ScanDeadline(90_000, renewable = true, clock = { now })
        repeat(100) { advance(60_000); assertTrue(deadline.progress()); assertFalse(deadline.expired()) }
        advance(89_999); assertFalse(deadline.expired())
        advance(1); assertTrue(deadline.expired()); assertFalse(deadline.progress())
        assertTrue(deadline.expired()) // Late progress cannot resurrect an expired scan.
    }

    @Test fun checkingIsNotProgress() {
        val deadline = ScanDeadline(90_000, renewable = true, clock = { now })
        repeat(89) { advance(1_000); assertFalse(deadline.expired()) }
        advance(1_000); assertTrue(deadline.expired())
    }

    @Test fun backgroundHardDeadlineCannotBeExtended() {
        val deadline = ScanDeadline(90_000, renewable = true, maxDurationMillis = 480_000, clock = { now })
        repeat(7) { advance(60_000); assertTrue(deadline.progress()) }
        advance(60_000); assertTrue(deadline.expired()); assertFalse(deadline.progress())
    }

    @Test fun ordinaryOperationsKeepTheirAbsoluteDeadline() {
        val deadline = ScanDeadline(90_000, clock = { now })
        advance(60_000); assertTrue(deadline.progress())
        advance(30_000); assertTrue(deadline.expired())
    }
}

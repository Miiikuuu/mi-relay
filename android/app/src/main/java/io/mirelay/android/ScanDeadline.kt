package io.mirelay.android

import java.util.concurrent.TimeUnit

/** Monotonic inactivity deadline. Only successful I/O extends it; cancellation
 * and an optional background work deadline can never be extended by progress. */
internal class ScanDeadline(
    timeoutMillis: Long,
    private val renewable: Boolean = false,
    maxDurationMillis: Long? = null,
    private val clock: () -> Long = System::nanoTime,
) {
    private val idleNanos = TimeUnit.MILLISECONDS.toNanos(timeoutMillis)
    private val maxNanos = maxDurationMillis?.let(TimeUnit.MILLISECONDS::toNanos)
    private val started = clock()
    private var lastProgress = started

    init {
        require(timeoutMillis > 0)
        require(maxDurationMillis == null || maxDurationMillis > 0)
    }

    @Synchronized fun expired(): Boolean = expiredAt(clock())
    private fun expiredAt(now: Long) = now - lastProgress >= idleNanos ||
        (maxNanos != null && now - started >= maxNanos)

    @Synchronized fun progress(): Boolean {
        val now = clock()
        if (expiredAt(now)) return false
        if (renewable) lastProgress = now
        return true
    }
}

package io.mirelay.android

import java.util.concurrent.TimeUnit
import java.util.concurrent.locks.ReentrantReadWriteLock
import kotlin.concurrent.read

/** Concurrent staging/uploads proceed normally. Cleanup drains actual work,
 * not just WorkManager's cancellation acknowledgement. A blocked provider
 * leaves cleanup pending instead of allowing files to be recreated afterward. */
internal object FolderWorkGate {
    private val lock = ReentrantReadWriteLock(true)
    fun <T> work(block: () -> T): T = lock.read(block)
    fun <T> clean(block: () -> T): T {
        val writer = lock.writeLock()
        check(writer.tryLock(3, TimeUnit.SECONDS)) { "A file operation is still stopping. Retry clean exit shortly." }
        try { return block() } finally { writer.unlock() }
    }
}

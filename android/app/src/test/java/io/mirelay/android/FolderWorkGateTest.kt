package io.mirelay.android

import org.junit.Assert.*
import org.junit.Test
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.Executors

class FolderWorkGateTest {
    @Test fun cleanupWaitsForActualWorkAndDoesNotRunAfterCancellationAlone() {
        val active = CountDownLatch(1)
        val release = CountDownLatch(1)
        val pool = Executors.newSingleThreadExecutor()
        try {
            val task = pool.submit { FolderWorkGate.work { active.countDown(); check(release.await(10, TimeUnit.SECONDS)) } }
            assertTrue(active.await(2, TimeUnit.SECONDS))
            var deleted = false
            assertTrue(runCatching { FolderWorkGate.clean { deleted = true } }.isFailure)
            assertFalse(deleted)
            release.countDown(); task.get(2, TimeUnit.SECONDS)
            FolderWorkGate.clean { deleted = true }
            assertTrue(deleted)
        } finally { release.countDown(); pool.shutdownNow() }
    }
}

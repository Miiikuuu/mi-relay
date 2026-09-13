package io.mirelay.android

import android.app.Application
import android.graphics.Bitmap
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import java.util.concurrent.atomic.AtomicInteger

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
class PreviewMemoryCacheTest {
    private fun bitmap(edge: Int = 4) = Bitmap.createBitmap(edge, edge, Bitmap.Config.ARGB_8888)

    @Test fun viewerChurnDoesNotEvictTilesAndEvictionDoesNotRecycleVisiblePixels() = runBlocking {
        val cache = PreviewMemoryCache(128, 64)
        val tile = bitmap()
        cache.load("tile", 256) { tile }
        val visible = cache.load("viewer-0", 1024) { bitmap() }!!
        repeat(20) { cache.load("viewer-$it", 1024) { bitmap() } }
        assertSame(tile, cache.load("tile", 256) { error("Tile was evicted by viewer") })
        assertFalse(visible.isRecycled)
    }

    @Test fun tileBudgetRemainsBoundedAndOversizedViewerCannotFlushTiles() = runBlocking {
        val cache = PreviewMemoryCache(64, 64)
        cache.load("old", 256) { bitmap() }
        val current = cache.load("current", 256) { bitmap() }
        repeat(2) { cache.load("too-large", 1024) { bitmap(8) } }
        assertSame(current, cache.load("current", 256) { error("Tile lost") })
        var reloads = 0
        cache.load("old", 256) { reloads++; bitmap() }
        assertEquals(1, reloads)
    }

    @Test fun concurrentIdenticalRequestsDecodeOnce() = runBlocking {
        val cache = PreviewMemoryCache(1024, 1024)
        val calls = AtomicInteger()
        val results = List(40) { async {
            cache.load("shared", 256) { calls.incrementAndGet(); delay(30); bitmap() }
        } }.awaitAll()
        assertEquals(1, calls.get())
        assertTrue(results.all { it === results.first() })
    }

    @Test fun permissionValidationStillRunsOnEveryHit() = runBlocking {
        val cache = PreviewMemoryCache(1024, 1024)
        cache.load("source", 256) { bitmap() }
        try {
            cache.load("source", 256, validate = { throw SecurityException("revoked") }) { error("Must not decode") }
            fail("Revoked grants must not expose cached pixels")
        } catch (_: SecurityException) { }
    }

    @Test fun cancellationReleasesKeyAndDoesNotCacheCompletedNativeWork() = runBlocking {
        val cache = PreviewMemoryCache(1024, 1024)
        val entered = CompletableDeferred<Unit>()
        val finish = CompletableDeferred<Unit>()
        val abandoned = bitmap()
        val job = launch {
            cache.load("cancelled", 256) {
                entered.complete(Unit)
                withContext(NonCancellable) { finish.await() }
                abandoned
            }
        }
        entered.await(); job.cancel(); finish.complete(Unit); job.join()
        val replacement = bitmap()
        assertSame(replacement, cache.load("cancelled", 256) { replacement })
        assertFalse(abandoned.isRecycled)
    }

    @Test fun failedOrNullDecodeCanRetry() = runBlocking {
        val cache = PreviewMemoryCache(1024, 1024)
        assertNull(cache.load("missing", 256) { null })
        try { cache.load("missing", 256) { error("codec") }; fail() }
        catch (_: IllegalStateException) { }
        val ready = bitmap()
        assertSame(ready, cache.load("missing", 256) { ready })
    }

    @Test fun sizesAndGenerationsNeverAlias() = runBlocking {
        val cache = PreviewMemoryCache(1024, 1024)
        val tile = cache.load("source:g1", 256) { bitmap() }
        assertNotSame(tile, cache.load("source:g1", 128) { bitmap() })
        assertNotSame(tile, cache.load("source:g1", 1024) { bitmap() })
        assertNotSame(tile, cache.load("source:g2", 256) { bitmap() })
    }

    @Test fun invalidTargetsNeverReachValidationOrDecode() = runBlocking {
        val cache = PreviewMemoryCache(1024, 1024)
        for (target in listOf(-1, 0, 1025, Int.MAX_VALUE)) {
            try {
                cache.load("invalid", target, validate = { error("Must not validate") }) { error("Must not decode") }
                fail("Invalid target accepted")
            } catch (_: IllegalArgumentException) { }
        }
    }
}

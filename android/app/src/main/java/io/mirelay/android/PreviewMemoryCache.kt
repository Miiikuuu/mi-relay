package io.mirelay.android

import android.graphics.Bitmap
import android.util.LruCache
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** Separate tile/viewer budgets prevent paging large previews from flushing the
 * grid. Fixed stripes coalesce identical requests without an unbounded job map. */
internal class PreviewMemoryCache(tileBytes: Int, viewerBytes: Int) {
    private fun cache(bytes: Int) = object : LruCache<String, Bitmap>(bytes) {
        override fun sizeOf(key: String, value: Bitmap) = value.byteCount
        // Eviction must not recycle a bitmap still held by a visible composable.
    }
    private val tiles = cache(tileBytes)
    private val viewers = cache(viewerBytes)
    private val locks = Array(32) { Mutex() }

    suspend fun load(
        identity: String,
        target: Int,
        validate: () -> Unit = {},
        decode: suspend () -> Bitmap?,
    ): Bitmap? = withContext(Dispatchers.IO) {
        require(target in 1..1024)
        val key = "$identity:$target"
        val cache = if (target <= 256) tiles else viewers
        locks[(key.hashCode() and Int.MAX_VALUE) % locks.size].withLock {
            currentCoroutineContext().ensureActive()
            validate() // In particular, recheck SAF permission even on a hit.
            cache.get(key) ?: decode()?.also {
                currentCoroutineContext().ensureActive()
                cache.put(key, it)
            }
        }
    }
}

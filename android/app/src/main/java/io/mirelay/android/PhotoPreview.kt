package io.mirelay.android

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.util.LruCache
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withContext
import java.io.File

/** Private previews or immutable staging only: no network, SAF reads or writes
 * inside the user's synchronized directory. At most two UI decodes, 8 MiB RAM. */
internal object PhotoThumbnails {
    private val permits = Semaphore(2)
    private val cache = object : LruCache<String, Bitmap>(8 * 1024 * 1024) {
        override fun sizeOf(key: String, value: Bitmap) = value.byteCount
        // Do not recycle evicted bitmaps: a visible composable may still own one.
    }
    internal fun sampleSize(width: Int, height: Int, target: Int): Int? {
        if (width <= 0 || height <= 0 || width.toLong() * height > 100_000_000L || target !in 1..1024) return null
        var sample = 1
        while ((width.toLong() + sample - 1) / sample > target || (height.toLong() + sample - 1) / sample > target) sample *= 2
        return sample
    }
    internal fun decode(file: File, target: Int): Bitmap? {
        if (!file.isFile || file.length() !in 1..MAX_FILE_BYTES) return null
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeFile(file.path, bounds)
        val sample = sampleSize(bounds.outWidth, bounds.outHeight, target) ?: return null
        return BitmapFactory.decodeFile(file.path, BitmapFactory.Options().apply { inSampleSize = sample })
    }
    suspend fun load(store: RelayStore, id: String, target: Int): Bitmap? = withContext(Dispatchers.IO) {
        val key = "${store.directory(id).absolutePath}:$target"
        cache.get(key) ?: permits.withPermit {
            cache.get(key) ?: try {
                (store.photoPreviews.load(id, target) ?: decode(File(store.directory(id), "payload"), target))?.also { cache.put(key, it) }
            } catch (_: Exception) { null }
        }
    }
}

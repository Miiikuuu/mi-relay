package io.mirelay.android

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import java.io.File

/** Private previews or immutable staging only: no network, SAF reads or writes
 * inside the user's synchronized directory. At most two UI decodes, 8 MiB RAM. */
internal object PhotoThumbnails {
    private val permits = Semaphore(2)
    private val cache = PreviewMemoryCache(4 * 1024 * 1024, 4 * 1024 * 1024)
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
    suspend fun load(store: RelayStore, id: String, target: Int): Bitmap? =
        cache.load(store.directory(id).absolutePath, target) {
            permits.withPermit {
                try {
                    store.photoPreviews.load(id, target) ?: decode(File(store.directory(id), "payload"), target)
                } catch (error: CancellationException) { throw error }
                catch (_: Exception) { null }
            }
        }
}

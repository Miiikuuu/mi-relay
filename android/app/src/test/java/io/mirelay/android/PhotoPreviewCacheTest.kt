package io.mirelay.android

import android.app.Application
import android.graphics.Bitmap
import android.graphics.Color
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.io.File
import java.util.UUID

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class PhotoPreviewCacheTest {
    private lateinit var root: File
    private lateinit var payload: File
    private fun id() = UUID.randomUUID().toString()
    @Before fun setup() {
        val temp = File(RuntimeEnvironment.getApplication().cacheDir, id()).apply { mkdirs() }
        root = File(temp, "previews")
        payload = File(temp, "payload")
        val bitmap = Bitmap.createBitmap(2049, 100, Bitmap.Config.ARGB_8888)
        bitmap.eraseColor(Color.RED)
        payload.outputStream().use { assertTrue(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)) }
        bitmap.recycle()
    }
    @Test fun derivedImageSurvivesStagingDeletionAndNewCacheInstance() {
        val id = id(); val before = payload.readBytes()
        assertTrue(PhotoPreviewCache(root).prepare(id, payload))
        assertArrayEquals(before, payload.readBytes())
        assertTrue(payload.delete())
        val image = PhotoPreviewCache(root).load(id, 1024)!!
        assertTrue(image.width <= 1024); assertEquals(Color.RED, image.getPixel(0, 0))
        assertNotNull(PhotoPreviewCache(root).load(id, 256))
    }
    @Test fun countAndByteBudgetsEvictOldestAndKeepLatest() {
        val first = id(); val second = id(); val third = id()
        val cache = PhotoPreviewCache(root, maxEntries = 2)
        assertTrue(cache.prepare(first, payload)); File(root, "$first.png").setLastModified(1)
        assertTrue(cache.prepare(second, payload)); assertTrue(cache.prepare(third, payload))
        assertNull(cache.load(first, 256)); assertNotNull(cache.load(third, 256))
        assertEquals(2, root.listFiles()!!.size)
        val size = File(root, "$third.png").length()
        val bounded = PhotoPreviewCache(root, maxBytes = size)
        val last = id(); assertTrue(bounded.prepare(last, payload))
        assertTrue(root.listFiles()!!.sumOf { it.length() } <= size)
        assertNotNull(bounded.load(last, 256))
    }
    @Test fun corruptMissingAndUnwritableCacheFailWithoutChangingOriginal() {
        root.writeText("blocks directory creation")
        val before = payload.readBytes()
        assertFalse(PhotoPreviewCache(root).prepare(id(), payload))
        assertArrayEquals(before, payload.readBytes())
        assertEquals("blocks directory creation", root.readText())
        payload.writeText("invalid jpeg")
        assertFalse(PhotoPreviewCache(root).prepare(id(), payload))
        payload.delete(); assertFalse(PhotoPreviewCache(root).prepare(id(), payload))
    }
    @Test fun interruptedWriteIsIgnoredThenRemovedOnNextSuccessfulWrite() {
        root.mkdirs(); val id = id()
        val partial = File(root, "$id.png.new").apply { writeText("partial PNG") }
        assertNull(PhotoPreviewCache(root).load(id, 256))
        assertTrue(PhotoPreviewCache(root).prepare(id, payload))
        assertFalse(partial.exists()); assertNotNull(PhotoPreviewCache(root).load(id, 256))
    }
    @Test fun invalidIdsTargetsAndInsufficientBudgetAreHarmless() {
        val cache = PhotoPreviewCache(root)
        assertFalse(cache.prepare("../payload", payload)); assertNull(cache.load("../payload", 256))
        assertFalse(PhotoPreviewCache(root, maxBytes = 1).prepare(id(), payload))
        assertFalse(PhotoPreviewCache(root, maxEntries = 0).prepare(id(), payload))
        val id = id(); assertTrue(cache.prepare(id, payload)); assertNull(cache.load(id, 2048))
    }
    @Test fun systemEvictionAndCorruptionReturnPlaceholder() {
        val cache = PhotoPreviewCache(root); val id = id()
        assertTrue(cache.prepare(id, payload))
        val file = File(root, "$id.png"); file.writeText("corrupt cache")
        assertNull(cache.load(id, 256)); file.delete(); assertNull(cache.load(id, 256))
        assertTrue(payload.isFile)
    }
}

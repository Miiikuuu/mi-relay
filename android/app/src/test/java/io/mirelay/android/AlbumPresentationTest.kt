package io.mirelay.android

import android.app.Application
import android.net.Uri
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.unit.IntSize
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
class AlbumPresentationTest {
    private fun file(id: String, name: String, date: Long?) = SourceFile(id, Uri.parse("content://fixture/$id"), name, 100, date, "nested/$name")

    @Test fun directoryAlbumIncludesNeverUploadedImagesAndOrdersUnknownDatesLast() {
        val files = listOf(file("unknown", "a.jpg", null), file("old", "old.PNG", 10),
            file("new", "new.jpg", 20), file("note", "note.txt", 30), file("part", "new.jpg.part", 40))
        val photos = AlbumPhotos.source("content://fixture/tree/root", files, "scan-1")
        assertEquals(listOf("new", "old", "unknown"), photos.map { it.key })
        assertTrue(photos.all { it.transfer == null && it.source != null && it.generation == "scan-1" })
        assertEquals(photos, AlbumPhotos.source("content://fixture/tree/root", files.reversed(), "scan-1"))
    }
    @Test fun emptyImagesRemainVisibleAndRefreshChangesDecodeCacheIdentity() {
        val file = file("image", "empty.jpg", 1).copy(size = 0)
        val a = AlbumPhotos.source("tree", listOf(file), "first").single()
        val b = AlbumPhotos.source("tree", listOf(file), "second").single()
        assertEquals(a.key, b.key); assertNotEquals(a.generation, b.generation)
        assertEquals(0L, a.size)
    }
    @Test fun galleryMetadataIsBoundedByTheExistingFiveThousandEntryContract() {
        val files = List(AutoStore.MAX_ENTRIES) { file("id-$it", "$it.jpg", it.toLong()) }
        val photos = AlbumPhotos.source("tree", files, "generation")
        assertEquals(5000, photos.size); assertEquals(5000, photos.map { it.key }.toSet().size)
        assertEquals("id-4999", photos.first().key)
    }
    @Test fun panRespectsActualFittedImageAndRejectsNonFiniteCoordinates() {
        val view = IntSize(300, 600); val image = IntSize(600, 300)
        assertEquals(Offset.Zero, albumPan(Offset(999f, 999f), 1f, view, image))
        assertEquals(Offset(150f, 0f), albumPan(Offset(999f, 999f), 2f, view, image))
        assertEquals(Offset.Zero, albumPan(Offset(Float.NaN, Float.POSITIVE_INFINITY), 2f, view, image))
        assertEquals(Offset.Zero, albumPan(Offset(10f, 10f), Float.NaN, view, image))
        assertEquals(Offset.Zero, albumPan(Offset(10f, 10f), 4f, IntSize.Zero, image))
    }
    @Test fun glassRequiresModernHardwareWithoutLowRamOrPowerSaving() {
        assertTrue(supportsAlbumGlass(31, false, false, true))
        assertTrue(supportsAlbumGlass(36, false, false, true))
        assertFalse(supportsAlbumGlass(30, false, false, true))
        assertFalse(supportsAlbumGlass(36, true, false, true))
        assertFalse(supportsAlbumGlass(36, false, true, true))
        assertFalse(supportsAlbumGlass(36, false, false, false))
    }
}

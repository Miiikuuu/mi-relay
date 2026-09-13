package io.mirelay.android

import android.app.Application
import android.net.Uri
import org.junit.Assert.*
import org.junit.Before
import org.junit.After
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.io.File

@RunWith(RobolectricTestRunner::class)
@Config(sdk=[35], application=Application::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class FolderKindTest {
    private lateinit var store: RelayStore
    private val context get() = RuntimeEnvironment.getApplication()
    private val cipher = object: TokenCipher { override fun seal(id:String,token:String)="sealed:$token"; override fun open(id:String,value:String)=value.removePrefix("sealed:") }
    @Before fun setup() { context.deleteDatabase("relay.db"); store=RelayStore(context,cipher) }
    @After fun close() { store.close() }
    private fun folder()=store.saveFolder(null,"Photos","https://example.com","original-secret",false)
    private fun file(path:String)=SourceFile(path,Uri.parse("content://source/document/item"),path.substringAfterLast('/'),42,1,path)

    @Test fun existingAndUnknownCategoriesAreGeneral() {
        val id=folder();assertEquals(FolderKind.GENERAL,store.folder(id)!!.kind)
        store.writableDatabase.execSQL("UPDATE folders SET kind='future-books' WHERE id=?",arrayOf(id))
        assertEquals(FolderKind.GENERAL,store.folder(id)!!.kind)
    }
    @Test fun presentationChangesNeverChangeConsentCredentialsOrQueue() {
        val id=folder()
        val source=store.automatic.enable(id,"content://source/tree/root","Source",true,listOf(file("note.txt")),1000)
        store.setFolderKind(id,FolderKind.PHOTOS)
        assertEquals(source,store.automatic.source(id));assertEquals("original-secret",store.token(id))
        assertTrue(store.transfers.value.isEmpty());assertEquals(FileFilter.ALL,store.automatic.source(id)!!.fileFilter)
        store.close();store=RelayStore(context,cipher);assertEquals(FolderKind.PHOTOS,store.folder(id)!!.kind)
    }
    @Test fun migrationFromFourKeepsPreparedSourceAndTokenAndDefaultsToAll() {
        val id=folder();store.automatic.enable(id,"content://source/tree/root","Source",true,listOf(file("note.txt")),1000,startImmediately=false)
        val db=store.writableDatabase
        // Reconstruct the v4 schema by removing only the v5 additive columns.
        db.execSQL("ALTER TABLE folders DROP COLUMN kind")
        db.execSQL("ALTER TABLE auto_sources DROP COLUMN file_filter")
        db.execSQL("ALTER TABLE auto_sources DROP COLUMN filtered")
        db.execSQL("ALTER TABLE directory_previews DROP COLUMN file_filter")
        db.execSQL("ALTER TABLE directory_previews DROP COLUMN skipped")
        db.version=4;store.close();store=RelayStore(context,cipher)
        assertEquals(6,store.readableDatabase.version);assertEquals("original-secret",store.token(id))
        assertEquals(FolderKind.GENERAL,store.folder(id)!!.kind)
        assertEquals(FileFilter.ALL,store.automatic.source(id)!!.fileFilter)
        assertTrue(store.automatic.source(id)!!.prepared);assertFalse(store.automatic.source(id)!!.enabled)
    }
    @Test fun unknownFilterFailsClosed() { assertThrows(IllegalArgumentException::class.java) { FileFilter.fromKey("images-v2") } }
    @Test fun allFilesDoesNotSilentlyExcludeMetadataOrEmptyFiles() {
        val files=listOf(file(".nomedia"),file("cover.jpg.part"),file("notes.txt").copy(size=0))
        val selected=FileFilter.ALL.select(files)
        assertEquals(files.toSet(),selected.files.toSet());assertTrue(selected.skipped.isEmpty())
    }
    @Test fun imageExtensionsHandleUnicodeCaseAndCompoundNamesWithoutMimeTrust() {
        for(path in listOf("画师/猫.JPG","album.v1/scan.TIFF","cover.HEIC","image.avif","animated.gif")) assertNull(FileFilter.IMAGES.skipReason(path))
        for(path in listOf("image.jpg.exe","jpg","file.svg","notes.txt",".nomedia","album.jpg/file")) assertNotNull(FileFilter.IMAGES.skipReason(path))
    }
    @Test fun temporaryDownloadsAreReportedAndSelectionIsStable() {
        val files=listOf(file("b.PNG"),file("a.jpg.part"),file("c.txt"),file("a.jpg"))
        val first=FileFilter.IMAGES.select(files);val second=FileFilter.IMAGES.select(files.reversed())
        assertEquals(first,second);assertEquals(listOf("a.jpg","b.PNG"),first.files.map {it.relativePath})
        assertEquals("Temporary download",first.skipped.first().reason)
        assertEquals(first.skipped,skippedFromJson(first.skipped.toJson()))
    }
    @Test fun thumbnailSamplingBoundsDecodeSizeAndRejectsHugeOrInvalidHeaders() {
        assertEquals(1,PhotoThumbnails.sampleSize(120,100,256))
        val sample=PhotoThumbnails.sampleSize(12000,8000,256)!!
        assertTrue(12000/sample<=256 && 8000/sample<=256)
        assertNull(PhotoThumbnails.sampleSize(Int.MAX_VALUE,Int.MAX_VALUE,256))
        assertNull(PhotoThumbnails.sampleSize(0,100,256));assertNull(PhotoThumbnails.sampleSize(100,100,4096))
    }
    @Test fun corruptOrMissingPicturesFailToPlaceholderWithoutAWrite() {
        val file=File.createTempFile("invalid-picture", ".jpg",context.cacheDir)
        try {file.writeText("not an image");assertNull(PhotoThumbnails.decode(file,256));assertEquals("not an image",file.readText())}
        finally {file.delete()}
        assertNull(PhotoThumbnails.decode(file,256))
    }
}

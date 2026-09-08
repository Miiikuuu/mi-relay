package io.mirelay.android

import android.app.Application
import android.net.Uri
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
class DirectoryScanExceptionTest {
    private fun file(size: Long? = 42, modified: Long? = 1, path: String = "Pixiv/empty.bin") =
        SourceFile("private-document-id", Uri.parse("content://private.provider/tree/private-root"), path.substringAfterLast('/'), size, modified, path)
    private fun message(vararg files: SourceFile) = assertThrows(DirectoryScanException::class.java) {
        DirectoryScanException.validate(files.toList())
    }.userMessage

    @Test fun emptyFileIdentifiesRelativePathReasonAndRecoveryWithoutExposingProviderIdentity() {
        val text = message(file(0))
        assertTrue(text.contains("\"Pixiv/empty.bin\"")); assertTrue(text.contains("empty (0 bytes)"))
        assertTrue(text.contains("Add content or move it out")); assertTrue(text.contains("This scan queued no files."))
        assertFalse(text.contains("private-document-id")); assertFalse(text.contains("private.provider")); assertFalse(text.contains("content://"))
    }
    @Test fun allPreviouslyIneligibleMetadataRemainsRejectedWithSpecificReasons() {
        assertTrue(message(file(MAX_FILE_BYTES + 1)).contains("100 MiB"))
        for (size in listOf(null, -1L, Long.MIN_VALUE)) assertTrue(message(file(size)).contains("size is unavailable"))
        for (modified in listOf(null, 0L, -1L)) assertTrue(message(file(modified = modified)).contains("modification time is unavailable"))
    }
    @Test fun validEmptyDirectoryAndBoundarySizedFilesRemainAllowed() {
        DirectoryScanException.validate(emptyList())
        DirectoryScanException.validate(listOf(file(1), file(MAX_FILE_BYTES)))
        assertTrue(file(1).eligible); assertTrue(file(MAX_FILE_BYTES).eligible)
    }
    @Test fun multipleUnsupportedFilesAreCountedWithoutListingUnboundedNames() {
        val text = message(file(0), file(42, path = "valid.bin"), file(null), file(MAX_FILE_BYTES + 1))
        assertTrue(text.contains("2 other unsupported file(s).")); assertFalse(text.contains("valid.bin"))
    }
    @Test fun fourGiBTotalLimitRemainsInclusive() {
        val exact = List(40) { file(MAX_FILE_BYTES) } + file(96L * 1024 * 1024)
        DirectoryScanException.validate(exact)
        assertTrue(message(*(exact + file(1)).toTypedArray()).contains("exceeds 4 GiB"))
    }
    @Test fun displayedPathIsBoundedAndRetainsBothParentAndFilenameWithUnicodeIntact() {
        val path = "parent/" + "😀".repeat(300) + "/empty.bin"
        val text = message(file(0, path = path))
        assertTrue(text.contains("parent/")); assertTrue(text.contains("…")); assertTrue(text.contains("/empty.bin"))
        assertTrue(text.length < 800); assertEquals(text, text.toByteArray(Charsets.UTF_8).toString(Charsets.UTF_8))
    }
    @Test fun displayedFilenameCannotInjectLinesQuotesOrDirectionOverrides() {
        val text = message(file(0, path = "Pixiv/a\n\r\t\u0000\u202e\u2066\u2028\u2029\"\\.bin"))
        assertFalse(text.any { it.isISOControl() || Character.getType(it) in listOf(Character.FORMAT.toInt(), Character.LINE_SEPARATOR.toInt(), Character.PARAGRAPH_SEPARATOR.toInt()) })
        assertEquals(2, text.count { it == '"' }); assertFalse(text.contains('\\'))
    }
}

package io.mirelay.android

import android.app.Application
import android.content.ContentProvider
import android.content.ContentValues
import android.content.pm.ProviderInfo
import android.content.res.AssetFileDescriptor
import android.database.MatrixCursor
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.os.OperationCanceledException
import android.provider.DocumentsContract
import android.provider.DocumentsContract.Document
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.shadows.ShadowContentResolver
import java.io.File
import java.io.RandomAccessFile

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
class DirectorySourceTest {
    @Test fun cancellationClosesTheTrackedStreamAndCannotBeRenewed() {
        var closed = false
        AutoSession.directoryScan().use { session ->
            session.track(java.io.Closeable { closed = true })
            session.cancel()
            assertTrue(closed)
            assertThrows(OperationCanceledException::class.java) { session.progress() }
            assertThrows(OperationCanceledException::class.java) { session.check() }
        }
    }

    @Test fun timerCancelsAnIdleReadWithoutWaitingForAnotherCheck() {
        val closed = java.util.concurrent.CountDownLatch(1)
        AutoSession(timeoutMillis = 100, progressTimeout = true).use { session ->
            session.track(java.io.Closeable { closed.countDown() })
            assertTrue(closed.await(5, java.util.concurrent.TimeUnit.SECONDS))
            assertTrue(session.signal.isCanceled)
            assertThrows(OperationCanceledException::class.java) { session.progress() }
        }
    }

    private class Provider(val file: File) : ContentProvider() {
        var opens = 0
        var reportedSize = MAX_FILE_BYTES
        override fun onCreate() = true
        override fun getType(uri: Uri) = "application/octet-stream"
        override fun query(uri: Uri, projection: Array<out String>?, selection: String?, args: Array<out String>?, order: String?) =
            MatrixCursor(projection!!).apply {
                val id = DocumentsContract.getDocumentId(uri)
                addRow(projection.map { column -> when (column) {
                    Document.COLUMN_DOCUMENT_ID, Document.COLUMN_DISPLAY_NAME -> id
                    Document.COLUMN_MIME_TYPE -> "application/octet-stream"
                    Document.COLUMN_SIZE -> reportedSize
                    Document.COLUMN_LAST_MODIFIED -> 1L
                    Document.COLUMN_FLAGS -> 0
                    else -> null
                } }.toTypedArray<Any?>())
            }
        override fun openAssetFile(uri: Uri, mode: String): AssetFileDescriptor {
            opens++
            return AssetFileDescriptor(ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY), 0, file.length())
        }
        override fun insert(uri: Uri, values: ContentValues?): Uri? = error("read only")
        override fun update(uri: Uri, values: ContentValues?, selection: String?, args: Array<out String>?) = error("read only")
        override fun delete(uri: Uri, selection: String?, args: Array<out String>?) = error("read only")
    }

    @Test fun hashesMoreThanFourGiBWithStreamingReadsAndPreservesSource() {
        val context = RuntimeEnvironment.getApplication()
        val file = File.createTempFile("large-inventory", ".bin", context.cacheDir)
        try {
            // One sparse 100 MiB backing file, exposed at 43 distinct SAF paths.
            // The reader really hashes 4.199 GiB; no multi-GiB allocation/write.
            RandomAccessFile(file, "rw").use { it.setLength(MAX_FILE_BYTES) }
            val provider = Provider(file)
            provider.attachInfo(context, ProviderInfo().apply { authority = "large.scan.test"; exported = true })
            ShadowContentResolver.registerProviderInternal("large.scan.test", provider)
            val tree = DocumentsContract.buildTreeDocumentUri("large.scan.test", "root")
            val files = List(43) { index ->
                val name = "image-$index.bin"
                SourceFile(name, DocumentsContract.buildDocumentUriUsingTree(tree, name), name, MAX_FILE_BYTES, 1L, name)
            }
            val reader = DirectorySource(context.contentResolver)
            val hashes = AutoSession.directoryScan().use { reader.hashes(tree, files, it) }
            assertEquals(43, hashes.size); assertEquals(43, provider.opens)
            assertEquals(1, hashes.map { it.sha256 }.distinct().size)
            val expected = java.security.MessageDigest.getInstance("SHA-256")
            val block = ByteArray(64 * 1024)
            repeat((MAX_FILE_BYTES / block.size).toInt()) { expected.update(block) }
            assertEquals(expected.digest().joinToString("") { "%02x".format(it.toInt() and 255) }, hashes[0].sha256)
            assertEquals(MAX_FILE_BYTES, file.length())
            assertTrue(files.sumOf { it.size!! } > 4L * 1024 * 1024 * 1024)
            // Unsupported and stale inputs fail before opening any payload.
            assertThrows(DirectoryScanException::class.java) {
                AutoSession.directoryScan().use { reader.hashes(tree, files + files[0].copy(size = 0), it) }
            }
            provider.reportedSize--
            assertThrows(IllegalStateException::class.java) {
                AutoSession.directoryScan().use { reader.hashes(tree, files.take(1), it) }
            }
            assertEquals(43, provider.opens)
        } finally { file.delete() }
    }
}

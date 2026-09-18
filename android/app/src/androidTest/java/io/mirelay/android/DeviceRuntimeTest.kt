package io.mirelay.android

import android.os.SystemClock
import android.util.Base64
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.UUID

@RunWith(AndroidJUnit4::class)
class DeviceRuntimeTest {
    @Before fun setup() { DeviceSupport.reset() }
    @Test fun imageFixtureOwnershipSurvivesQueueReset() {
        val app = DeviceSupport.app
        val folder = DeviceSupport.folder()
        val id = DeviceSupport.image(folder, "Owned-original.png")
        val dir = app.store.directory(id)
        val payload = File(dir, "payload").readBytes()
        assertEquals(folder, File(dir, "folder-owner").readText())
        DeviceSupport.reset()
        val other = DeviceSupport.folder()
        assertTrue(app.store.cleanupTransfers(other).isEmpty())
        assertArrayEquals(payload, File(dir, "payload").readBytes())
    }
    @Test fun genuinelyUnownedPayloadStillBlocksCleanupWithoutDeletingIt() {
        val app = DeviceSupport.app
        val folder = DeviceSupport.folder()
        val id = DeviceSupport.image(folder, "Unowned-original.png")
        val dir = app.store.directory(id)
        val marker = File(dir, "folder-owner")
        val payload = File(dir, "payload").readBytes()
        assertTrue(marker.delete())
        try {
            app.store.writableDatabase.delete("transfers", "id=?", arrayOf(id))
            app.store.refresh()
            val failure = assertThrows(IllegalArgumentException::class.java) {
                app.store.cleanupTransfers(folder)
            }
            assertTrue(failure.message!!.contains("Old unowned staging data"))
            assertArrayEquals(payload, File(dir, "payload").readBytes())
        } finally {
            // Restore this deliberately malformed synthetic fixture, even on a
            // failed assertion; never poison later cases or relax the guard.
            marker.writeText(folder, Charsets.US_ASCII)
        }
    }
    @Test fun nativeLibraryLoadsAndInitializesOnArt() {
        assertTrue(NativeBridge.initialize(DeviceSupport.app))
        assertTrue(JSONObject(NativeBridge.upload("{}") { _, _ -> true }).has("error"))
        assertTrue(JSONObject(NativeBridge.upload("x".repeat(65537)) { _, _ -> true }).has("error"))
    }
    @Test fun actualKeystoreRoundTripUsesRandomIvAndSurvivesNewVault() {
        val vault = TokenVault()
        val first = vault.seal("folder-one", "qa-secret")
        val second = vault.seal("folder-one", "qa-secret")
        assertNotEquals(first, second)
        assertEquals("qa-secret", TokenVault().open("folder-one", first))
        assertFalse(first.contains("qa-secret"))
    }
    @Test fun keystoreRejectsWrongFolderAndTamperedCiphertext() {
        val vault = TokenVault()
        val sealed = vault.seal("correct-folder", "qa-secret")
        assertThrows(Exception::class.java) { vault.open("wrong-folder", sealed) }
        val corrupt = Base64.decode(sealed, Base64.NO_WRAP)
        corrupt[corrupt.lastIndex] = (corrupt.last().toInt() xor 1).toByte()
        assertThrows(Exception::class.java) { vault.open("correct-folder", Base64.encodeToString(corrupt, Base64.NO_WRAP)) }
    }
    @Test fun credentialsPersistEncryptedInRealSqlite() {
        val folder = DeviceSupport.folder()
        RelayStore(DeviceSupport.app).use { reopened -> assertEquals(DeviceSupport.token, reopened.token(folder)) }
        DeviceSupport.app.store.readableDatabase.rawQuery("SELECT token FROM folders", null).use {
            assertTrue(it.moveToFirst()); assertFalse(it.getString(0).contains(DeviceSupport.token))
        }
    }
    @Test fun externalProviderStagesExactBytesWithSafeUnicodeName() {
        val id = DeviceSupport.import(DeviceSupport.folder(), 65539, "../中文😀.bin")
        val dir = DeviceSupport.app.store.directory(id)
        val expected = ByteArray(65539) { (it % 251).toByte() }
        assertArrayEquals(expected, File(dir, "payload").readBytes())
        assertFalse(File(dir, "payload.part").exists())
        assertFalse(DeviceSupport.app.store.transfer(id)!!.name.contains('/'))
        assertTrue(DeviceSupport.app.store.transfer(id)!!.name.contains("中文😀"))
        assertTrue(dir.path.startsWith(DeviceSupport.app.noBackupFilesDir.path))
    }
    @Test fun emptyRevokedOversizeAndNonContentInputsNeverEnqueue() {
        val folder = DeviceSupport.folder()
        val importer = FileImporter(DeviceSupport.app.contentResolver, DeviceSupport.app.store)
        for (uri in listOf(DeviceSupport.uri(0), DeviceSupport.uri(denied = true),
            DeviceSupport.uri(declared = MAX_FILE_BYTES + 1), android.net.Uri.parse("file:///etc/passwd"))) {
            assertThrows(Exception::class.java) { importer.import(uri, folder) }
        }
        assertTrue(DeviceSupport.app.store.transfers.value.isEmpty())
    }
    @Test fun dishonestProviderSizeIsStillBoundedByStreamingLimit() {
        val importer = FileImporter(DeviceSupport.app.contentResolver, DeviceSupport.app.store)
        assertThrows(IllegalArgumentException::class.java) {
            importer.import(DeviceSupport.uri(101 * 1024 * 1024, declared = 1), DeviceSupport.folder())
        }
        assertTrue(DeviceSupport.app.store.transfers.value.isEmpty())
    }
    @Test fun nativeTusPauseResumeAndReceiptReplayUseOneDelivery() {
        assertTrue(NativeBridge.initialize(DeviceSupport.app))
        val file = File(DeviceSupport.app.cacheDir, "${UUID.randomUUID()}.bin").apply { writeBytes(ByteArray(8192) { (it % 251).toByte() }) }
        val request = DeviceSupport.request(file).toString()
        val paused = JSONObject(NativeBridge.upload(request) { bytes, _ -> bytes < 1024 }).getJSONObject("result")
        assertEquals(1024, paused.getLong("uploaded_bytes")); assertTrue(paused.isNull("delivery_id"))
        val done = JSONObject(NativeBridge.upload(request) { _, _ -> true }).getJSONObject("result")
        val replay = JSONObject(NativeBridge.upload(request) { _, _ -> true }).getJSONObject("result")
        assertEquals(8192, done.getLong("uploaded_bytes")); assertEquals(done.toString(), replay.toString())
    }
    @Test fun javaCallbackExceptionsPropagateWithoutNativeAbort() {
        assertTrue(NativeBridge.initialize(DeviceSupport.app))
        val file = File(DeviceSupport.app.cacheDir, "${UUID.randomUUID()}.txt").apply { writeText("Android ART callback sample") }
        assertThrows(IllegalStateException::class.java) {
            NativeBridge.upload(DeviceSupport.request(file).toString()) { _, _ -> error("qa callback exception") }
        }
        assertTrue(JSONObject(NativeBridge.upload("{}") { _, _ -> true }).has("error"))
    }
    @Test fun badAuthenticationIsNotRetriedAndTokenIsRedacted() {
        assertTrue(NativeBridge.initialize(DeviceSupport.app))
        val file = File(DeviceSupport.app.cacheDir, "${UUID.randomUUID()}.txt").apply { writeText("authentication sample") }
        val result = JSONObject(NativeBridge.upload(DeviceSupport.request(file, secret = "wrong-emulator-token").toString()) { _, _ -> true })
        assertTrue(result.has("error")); assertFalse(result.getBoolean("retryable")); assertFalse(result.toString().contains("wrong-emulator-token"))
    }
    @Test fun unreachableServerIsBoundedAndRetryable() {
        assertTrue(NativeBridge.initialize(DeviceSupport.app))
        val file = File(DeviceSupport.app.cacheDir, "${UUID.randomUUID()}.txt").apply { writeText("offline sample") }
        val started = SystemClock.elapsedRealtime()
        val result = JSONObject(NativeBridge.upload(DeviceSupport.request(file, url = "http://127.0.0.1:18999").toString()) { _, _ -> true })
        assertTrue(result.has("error")); assertTrue(result.getBoolean("retryable"))
        assertTrue(SystemClock.elapsedRealtime() - started < 10000)
    }
    @Test fun untrustedTlsCertificateIsRejectedByAndroidVerifier() {
        assertTrue(NativeBridge.initialize(DeviceSupport.app))
        val file = File(DeviceSupport.app.cacheDir, "${UUID.randomUUID()}.bin").apply { writeBytes(ByteArray(32)) }
        val result = JSONObject(NativeBridge.upload(DeviceSupport.request(file, url = "https://10.0.2.2:18082").toString()) { _, _ -> true })
        assertTrue(result.toString(), result.has("error"))
        assertTrue(result.toString(), result.getString("error").contains("certificate", ignoreCase = true))
    }
    @Test fun sqliteOwnershipGuardsWorkOnActualAndroid() {
        val store = DeviceSupport.app.store
        val id = DeviceSupport.import(DeviceSupport.folder())
        store.assign(id, "old"); store.pause(id); store.assign(id, "new")
        assertFalse(store.updateOwned(id, "old", TransferStatus.UPLOADED, 4096, delivery = "stale"))
        assertTrue(store.updateOwned(id, "new", TransferStatus.UPLOADED, 4096, delivery = "receipt"))
        assertThrows(IllegalStateException::class.java) { store.assign(id, "duplicate") }
    }
    @Test fun refreshWaitingForTransactionDoesNotBlockDatabaseHelperMonitor() {
        val store=DeviceSupport.app.store
        val database=store.writableDatabase
        val pool=java.util.concurrent.Executors.newFixedThreadPool(2)
        try {
            repeat(5) {
                val readerThread=java.util.concurrent.atomic.AtomicReference<Thread>()
                database.beginTransaction()
                val reader=pool.submit {
                    readerThread.set(Thread.currentThread())
                    store.refresh()
                }
                try {
                    // Force the exact lock order seen in the batch-upload hang:
                    // refresh waits for this connection while the writer obtains
                    // the helper again to finish its transaction.
                    DeviceSupport.await(5000) {readerThread.get()?.stackTrace?.any {
                        it.className=="android.database.sqlite.SQLiteConnectionPool" && it.methodName=="waitForConnection"
                    }==true}
                    val handle=pool.submit<android.database.sqlite.SQLiteDatabase> {store.writableDatabase}
                    assertSame(database,handle.get(2,java.util.concurrent.TimeUnit.SECONDS))
                } finally {
                    // Use the cached handle even on failure, so the pre-fix
                    // deadlock fails within a timeout instead of wedging the suite.
                    database.endTransaction()
                }
                reader.get(5,java.util.concurrent.TimeUnit.SECONDS)
            }
            database.rawQuery("PRAGMA integrity_check",null).use {assertTrue(it.moveToFirst());assertEquals("ok",it.getString(0))}
        } finally {
            pool.shutdownNow()
            assertTrue(pool.awaitTermination(10,java.util.concurrent.TimeUnit.SECONDS))
        }
    }
}

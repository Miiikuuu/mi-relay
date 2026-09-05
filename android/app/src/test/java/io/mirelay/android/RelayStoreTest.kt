package io.mirelay.android

import android.app.Application
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import java.util.UUID

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
class RelayStoreTest {
    private lateinit var store: RelayStore
    private lateinit var folder: String
    private val cipher = object : TokenCipher {
        override fun seal(id: String, token: String) = "$id:${token.reversed()}"
        override fun open(id: String, value: String) = value.removePrefix("$id:").reversed()
    }
    @Before fun setup() {
        val context = RuntimeEnvironment.getApplication()
        context.deleteDatabase("relay.db")
        store = RelayStore(context, cipher)
        folder = store.saveFolder(null, "Linux", "https://example.com", "test-secret", false)
    }
    @After fun close() { store.close() }
    private fun transfer(): String = UUID.randomUUID().toString().also { store.addTransfer(it, folder, "file.txt", 42) }

    @Test fun folderCredentialsUseCipherAndNeverAppearInUiModels() {
        assertEquals("test-secret", store.token(folder))
        assertFalse(store.folders.value.toString().contains("test-secret"))
        store.readableDatabase.rawQuery("SELECT token FROM folders", null).use {
            assertTrue(it.moveToFirst()); assertFalse(it.getString(0).contains("test-secret"))
        }
    }
    @Test fun pausedAndReplacedWorkersCannotOverwriteNewOwner() {
        val id = transfer()
        store.assign(id, "old")
        assertTrue(store.updateOwned(id, "old", TransferStatus.UPLOADING, 10))
        store.pause(id)
        assertFalse(store.updateOwned(id, "old", TransferStatus.UPLOADED, 42, delivery = "stale"))
        store.assign(id, "new")
        assertFalse(store.updateOwned(id, "old", TransferStatus.FAILED, error = "stale"))
        assertTrue(store.updateOwned(id, "new", TransferStatus.UPLOADED, 42, delivery = "actual"))
        assertEquals("actual", store.transfer(id)?.deliveryId)
        assertFalse(store.updateOwned(id, "new", TransferStatus.UPLOADING, 0))
    }
    @Test fun completedReceiptSurvivesReopeningAndCannotBeRequeued() {
        val id = transfer()
        store.assign(id, "worker")
        store.updateOwned(id, "worker", TransferStatus.UPLOADED, 42, delivery = "receipt")
        store.close()
        store = RelayStore(RuntimeEnvironment.getApplication(), cipher)
        assertEquals("receipt", store.transfer(id)?.deliveryId)
        assertThrows(IllegalStateException::class.java) { store.assign(id, "duplicate") }
    }
    @Test fun changingServerDoesNotRetargetExistingTransfers() {
        assertThrows(IllegalArgumentException::class.java) { store.saveFolder(folder, "Changed", "https://other.example", "secret", false) }
        assertEquals("https://example.com", store.folder(folder)?.server)
        store.saveFolder(folder, "Renamed", "https://example.com", "replacement", false)
        assertEquals("replacement", store.token(folder))
    }
    @Test fun stagingPathsRejectTraversal() {
        for (id in listOf("../other", "", "../../", "/tmp/x")) {
            assertThrows(IllegalArgumentException::class.java) { store.directory(id) }
        }
    }
}

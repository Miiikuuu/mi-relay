package io.mirelay.android

import org.junit.Assert.*
import org.junit.Test

class DesignV2Test {
    @Test fun onlyConfirmedConnectionsCanShowPaired() {
        val folder = Folder("id", "Photos", "https://example.com/f/id", false)
        for (state in listOf("legacy", "pending", "pairing_pending", "disconnect_pending", "disconnected", "unknown")) {
            assertFalse(state, pairedForDisplay(folder.copy(pairingState = state)))
        }
        assertTrue(pairedForDisplay(folder.copy(pairingState = "ready")))
    }

    @Test fun categoryAndNameCannotImplyPairing() {
        for (kind in FolderKind.entries) {
            assertFalse(pairedForDisplay(Folder("id", "Paired", "https://example.com", false, kind = kind)))
        }
        assertEquals(3, ALBUM_COLUMNS)
    }
}

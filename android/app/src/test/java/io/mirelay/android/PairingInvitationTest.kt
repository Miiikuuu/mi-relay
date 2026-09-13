package io.mirelay.android

import android.app.Application
import com.google.zxing.BinaryBitmap
import com.google.zxing.RGBLuminanceSource
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader
import com.google.zxing.qrcode.QRCodeWriter
import com.google.zxing.BarcodeFormat
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import java.io.File

@RunWith(RobolectricTestRunner::class)
@Config(sdk=[35], application=Application::class)
class PairingInvitationTest {
    private val code = "00000000-0000-4000-8000-000000000001." + "ab".repeat(32)
    private fun invitation() = JSONObject().put("kind", "mirelay-pairing").put("version", 1)
        .put("server_url", "https://example.com").put("pairing_code", code).put("expires_at_unix", 1600L)
    private fun parse(json: JSONObject) = PairingInvitation.parse(json.toString(), 1000)

    @Test fun validInvitationIsOnlyParsedNotClaimedAndDoesNotExposeSecrets() {
        val value = parse(invitation())
        assertEquals("https://example.com", value.server); assertEquals(code, value.code); assertEquals(1600L, value.expires)
        assertFalse(value.toString().contains(code))
        for (server in listOf("https://192.0.2.1", "https://example.com:8443", "https://example.com/relay/", "https://[::1]")) {
            assertEquals(server.trimEnd('/'), parse(invitation().put("server_url", server)).server)
        }
    }
    @Test fun unsafeUrlsNeverBecomeInvitations() {
        for (server in listOf("http://example.com", "file:///tmp/x", "https://secret@example.com", "https://example.com/?token=secret",
            "https://example.com/#secret", "https://example.com/f/id", "https://example.com/%66/id", "https://example.com\\evil",
            "https://example.com/\n", "https://example.com:0", "https://example.com:65536", "https://example.com:")) {
            assertThrows(server, IllegalArgumentException::class.java) { parse(invitation().put("server_url", server)) }
        }
    }
    @Test fun expiryBoundaryAndFutureClockSkewFailClosed() {
        for (expires in listOf(0L, 999, 1000, 1901, Long.MAX_VALUE)) {
            assertThrows(IllegalArgumentException::class.java) { parse(invitation().put("expires_at_unix", expires)) }
        }
        assertEquals(1001L, parse(invitation().put("expires_at_unix", 1001)).expires)
        assertEquals(1900L, parse(invitation().put("expires_at_unix", 1900)).expires)
    }
    @Test fun strictEnvelopeRejectsUnknownDuplicateMissingAndCoercedFields() {
        for (json in listOf(invitation().put("kind", "other"), invitation().put("version", 2), invitation().put("version", "1"),
            invitation().put("admin_token", "secret"), invitation().put("expires_at_unix", "1600"), invitation().put("pairing_code", true))) {
            assertThrows(IllegalArgumentException::class.java) { parse(json) }
        }
        val missing = invitation(); missing.remove("kind")
        assertThrows(IllegalArgumentException::class.java) { parse(missing) }
        val text = invitation().toString()
        for (raw in listOf("https://example.com", "{}", "[]", text + "{}", text.dropLast(1) + ",\"version\":1}",
            text.replace("\"version\":1", "\"version\":1.0"), "x".repeat(1537), text.replace("\"kind\"", "kind"))) {
            assertThrows(IllegalArgumentException::class.java) { PairingInvitation.parse(raw, 1000) }
        }
    }
    @Test fun invalidSecretNeverAppearsInErrors() {
        for (bad in listOf("secret-do-not-print", "", "bad." + "ab".repeat(32), code + "\n", code.replace("ab", "gg"))) {
            val error = assertThrows(IllegalArgumentException::class.java) { parse(invitation().put("pairing_code", bad)) }
            assertFalse(error.message.orEmpty().contains("secret-do-not-print"))
            assertFalse(error.message.orEmpty().contains(code))
        }
    }
    @Test fun offlineDecoderRoundTripPreservesInvitation() {
        val text = invitation().toString()
        val matrix = QRCodeWriter().encode(text, BarcodeFormat.QR_CODE, 360, 360)
        val pixels = IntArray(matrix.width * matrix.height) { i -> if (matrix[i % matrix.width, i / matrix.width]) 0xff000000.toInt() else -1 }
        val decoded = QRCodeReader().decode(BinaryBitmap(HybridBinarizer(RGBLuminanceSource(matrix.width, matrix.height, pixels))))
        assertEquals(code, PairingInvitation.parse(decoded.text, 1000).code)
    }
    @Test fun decodeLinuxGeneratedFixtureWhenExplicitlyProvided() {
        val path = System.getProperty("mirelay.qr.fixture")
        org.junit.Assume.assumeTrue("Explicit Linux QR fixture required", path != null)
        val words = File(checkNotNull(path)).readText().trim().split(Regex("\\s+"))
        assertEquals("P2", words[0]); assertEquals("255", words[3])
        val width = words[1].toInt(); val height = words[2].toInt()
        assertEquals(width * height + 4, words.size)
        val pixels = IntArray(width * height) { i -> if (words[i+4] == "0") 0xff000000.toInt() else -1 }
        val decoded = QRCodeReader().decode(BinaryBitmap(HybridBinarizer(RGBLuminanceSource(width, height, pixels))))
        val value = PairingInvitation.parse(decoded.text, 1_999_999_900)
        assertEquals(code, value.code); assertEquals("https://example.com", value.server)
    }
}

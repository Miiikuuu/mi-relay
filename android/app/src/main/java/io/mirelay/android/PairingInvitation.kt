package io.mirelay.android

import android.util.JsonReader
import android.util.JsonToken
import java.io.StringReader
import java.net.URI
import java.util.UUID

/** Untrusted camera text. Parsing does not open a URL, claim a Folder, or save a credential. */
internal class PairingInvitation(val server: String, val code: String, val expires: Long) {
    // Do not generate a data-class toString exposing the invitation secret.
    companion object {
        private const val INVALID = "Not a valid MiRelay invitation. Scan the QR code shown by Linux."
        fun parse(raw: String, now: Long = System.currentTimeMillis() / 1000): PairingInvitation {
            require(raw.length <= 1536 && raw.toByteArray(Charsets.UTF_8).size <= 1536) { INVALID }
            try {
                val values = mutableMapOf<String, String>()
                JsonReader(StringReader(raw)).use { reader ->
                    reader.isLenient = false
                    reader.beginObject()
                    while (reader.hasNext()) {
                        val key = reader.nextName()
                        require(key in setOf("kind", "version", "server_url", "pairing_code", "expires_at_unix") && key !in values) { INVALID }
                        val numeric = key == "version" || key == "expires_at_unix"
                        require(reader.peek() == if (numeric) JsonToken.NUMBER else JsonToken.STRING) { INVALID }
                        val value = reader.nextString()
                        require(!numeric || value.matches(Regex("[0-9]{1,12}"))) { INVALID }
                        values[key] = value
                    }
                    reader.endObject()
                    require(reader.peek() == JsonToken.END_DOCUMENT) { INVALID }
                }
                require(values.size == 5 && values["kind"] == "mirelay-pairing" && values["version"] == "1") { INVALID }
                val server = values.getValue("server_url")
                require(server.length <= 512 && server.all { it.code in 33..126 } && '%' !in server && '\\' !in server) { INVALID }
                val uri = URI(server).parseServerAuthority()
                require(uri.scheme == "https" && !uri.host.isNullOrEmpty() && uri.rawUserInfo == null
                    && uri.rawQuery == null && uri.rawFragment == null && !uri.rawAuthority.endsWith(':') && uri.port in -1..65535
                    && uri.port != 0 && !uri.rawPath.orEmpty().contains("/f/")) { INVALID }
                val code = values.getValue("pairing_code")
                val parts = code.split('.')
                require(parts.size == 2 && UUID.fromString(parts[0]).toString() == parts[0]
                    && parts[1].length == 64 && parts[1].all { it in "0123456789abcdefABCDEF" }) { INVALID }
                val expires = values.getValue("expires_at_unix").toLong()
                require(now >= 0 && expires > now && expires - now <= 900) {
                    "Invitation expired or device clock is incorrect. Replace the invitation on Linux and scan again."
                }
                return PairingInvitation(server.trimEnd('/'), code, expires)
            } catch (error: Exception) {
                // Parser/URI exception messages may echo the complete secret input.
                if (error is IllegalArgumentException && error.message?.startsWith("Invitation expired") == true) throw error
                throw IllegalArgumentException(INVALID)
            }
        }
    }
}

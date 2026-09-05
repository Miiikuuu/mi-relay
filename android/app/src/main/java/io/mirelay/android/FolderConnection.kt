package io.mirelay.android

import android.content.Context
import org.json.JSONObject
import java.net.URI
import java.security.SecureRandom
import java.util.UUID

/** Shared Rust HTTPS and protocol checks. Call only on an IO thread. */
internal class FolderConnection(private val context: Context) {
    fun newSenderToken(): String = ByteArray(32).also { SecureRandom().nextBytes(it) }.joinToString("") { "%02x".format(it.toInt() and 255) }
    fun folderUrl(server: String, code: String): String {
        val parts = code.split('.')
        require(parts.size == 2 && runCatching { UUID.fromString(parts[0]).toString() == parts[0] }.getOrDefault(false)
            && parts[1].length == 64 && parts[1].all { it in "0123456789abcdefABCDEF" }) { "Paste the full pairing code from Linux." }
        require(!URI(server).path.orEmpty().contains("/f/")) { "Use the original server URL, without /f/ or a Folder ID." }
        return "${server.trimEnd('/')}/f/${parts[0]}"
    }
    fun claim(server: String, code: String, token: String, insecure: Boolean): JSONObject =
        call("claim", server, token, insecure, code).getJSONObject("result")

    fun check(folder: Folder, token: String): JSONObject? {
        val scoped = URI(folder.server).path.orEmpty().contains("/f/")
        val response = call(if (scoped) "handshake" else "legacy_check", folder.server, token, folder.insecure)
        val info = response.optJSONObject("result")
        if (scoped) require(info?.getString("role") == "sender") { "Use a sender credential on Android." }
        return info
    }
    private fun call(action: String, server: String, token: String, insecure: Boolean, code: String? = null): JSONObject {
        check(NativeBridge.initialize(context)) { "Could not initialize HTTPS certificate verification." }
        val request = JSONObject().apply {
            put("action", action); put("server_url", server); put("token", token)
            put("allow_insecure_http", insecure && BuildConfig.DEBUG)
            code?.let { put("pairing_code", it) }
        }
        val result = JSONObject(NativeBridge.pairing(request.toString()))
        require(!result.has("error")) { result.optString("error", "Connection check failed.") }
        return result
    }
}

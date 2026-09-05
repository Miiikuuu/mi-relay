package io.mirelay.android

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID

internal interface DirectoryRemote {
    fun state(folder: Folder, token: String): JSONObject
    fun compare(source: JSONObject, remote: JSONObject): JSONObject
}

internal class DirectoryConnection(private val context: Context) : DirectoryRemote {
    override fun state(folder: Folder, token: String): JSONObject = call(JSONObject().apply {
        put("action", "state"); put("server_url", folder.server); put("token", token)
        put("allow_insecure_http", folder.insecure && BuildConfig.DEBUG)
    })
    override fun compare(source: JSONObject, remote: JSONObject): JSONObject = call(JSONObject().apply {
        put("action", "compare"); put("source", source); put("remote", remote)
    })
    private fun call(request: JSONObject): JSONObject {
        require(request.toString().toByteArray().size <= 8 * 1024 * 1024) { "Directory preview is too large. Choose a smaller source." }
        check(NativeBridge.initialize(context)) { "Could not initialize HTTPS certificate verification." }
        val response = JSONObject(NativeBridge.directory(request.toString()))
        require(!response.has("error")) { response.optString("error", "Directory connection failed.") }
        return response.getJSONObject("result")
    }
}

internal fun List<HashedSource>.inventory(): JSONObject {
    val entries = JSONArray().apply { sortedBy { it.file.relativePath }.forEach { add -> put(JSONObject().apply {
        put("path", add.file.relativePath); put("sha256", add.sha256); put("size", add.file.size)
    }) } }
    return JSONObject().put("id", UUID.nameUUIDFromBytes(entries.toString().toByteArray()).toString()).put("entries", entries)
}
internal fun List<HashedSource>.snapshotJson(): String = JSONArray().apply {
    sortedBy { it.file.relativePath }.forEach { value -> put(JSONObject().apply {
        put("document_id", value.file.documentId); put("path", value.file.relativePath)
        put("fingerprint", value.file.fingerprint); put("sha256", value.sha256); put("size", value.file.size)
    }) }
}.toString()
internal fun JSONObject.strings(key: String): List<String> = getJSONArray(key).let { array -> List(array.length()) { array.getString(it) } }

package io.mirelay.android

import android.net.Uri
import android.os.Build
import android.os.SystemClock
import androidx.test.platform.app.InstrumentationRegistry
import androidx.work.WorkManager
import java.io.File
import java.util.UUID
import java.util.concurrent.TimeUnit
import org.json.JSONObject

object DeviceSupport {
    val app get() = InstrumentationRegistry.getInstrumentation().targetContext.applicationContext as RelayApplication
    const val server = "http://10.0.2.2:18080"
    const val token = "isolated-emulator-test-token"
    fun setupRequest(path: String, credential: String, body: JSONObject): JSONObject {
        guard()
        val connection = java.net.URL(server + path).openConnection() as java.net.HttpURLConnection
        try {
            connection.requestMethod = "POST"; connection.connectTimeout = 5000; connection.readTimeout = 5000
            connection.instanceFollowRedirects = false; connection.doOutput = true
            connection.setRequestProperty("Authorization", "Bearer $credential")
            connection.setRequestProperty("Mirelay-Protocol-Version", "1")
            connection.setRequestProperty("Content-Type", "application/json")
            connection.outputStream.use { it.write(body.toString().toByteArray()) }
            check(connection.responseCode == 200) { "Test setup request failed: ${connection.responseCode}" }
            return JSONObject(connection.inputStream.bufferedReader().use { it.readText() })
        } finally { connection.disconnect() }
    }
    fun invitation() = setupRequest("/api/v1/folders", "isolated-emulator-admin-credential-000001", JSONObject().put("name", "Pixiv source"))
    fun guard() {
        check(InstrumentationRegistry.getArguments().getString("isolatedAvd") == "MiRelay_API36_QA") { "Run only through the dedicated emulator test script." }
        check(Build.HARDWARE == "ranchu" || Build.HARDWARE == "goldfish") { "Refusing to clear data on a physical device." }
    }
    fun reset() {
        guard()
        WorkManager.getInstance(app).cancelAllWork().result.get(15, TimeUnit.SECONDS)
        app.store.writableDatabase.execSQL("DELETE FROM transfers")
        app.store.writableDatabase.execSQL("DELETE FROM folders")
        app.store.refresh()
    }
    fun folder(name: String = "QA Linux", url: String = server, secret: String = token) =
        app.store.saveFolder(null, name, url, secret, url.startsWith("http:"))
    fun uri(size: Int = 4096, name: String = "fixture.bin", declared: Long? = null, denied: Boolean = false): Uri =
        Uri.Builder().scheme("content").authority("io.mirelay.android.test.fixtures")
            .path(if (denied) "denied" else "data").appendQueryParameter("size", size.toString())
            .appendQueryParameter("name", name).apply { declared?.let { appendQueryParameter("declared", it.toString()) } }.build()
    fun import(folder: String, size: Int = 4096, name: String = "fixture.bin") = FileImporter(app.contentResolver, app.store).import(uri(size, name), folder)
    fun await(timeout: Long = 30000, predicate: () -> Boolean) {
        val end = SystemClock.elapsedRealtime() + timeout
        while (!predicate()) {
            check(SystemClock.elapsedRealtime() < end) { "Condition timed out after $timeout ms" }
            SystemClock.sleep(100)
        }
    }
    fun request(file: File, state: File = File(file.parentFile, "${UUID.randomUUID()}.json"), url: String = server, secret: String = token) = JSONObject().apply {
        put("file", file.path); put("state_file", state.path); put("server_url", url); put("token", secret)
        put("name", file.name); put("chunk_size_bytes", 1024); put("max_chunks", JSONObject.NULL)
        put("allow_insecure_http", url.startsWith("http:")); put("request_timeout_seconds", 5); put("keep_completed_state", true)
    }
}

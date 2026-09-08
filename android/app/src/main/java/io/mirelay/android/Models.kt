package io.mirelay.android

import java.net.URI

const val MAX_FILE_BYTES = 100L * 1024 * 1024
const val MAX_SHARED_FILES = 20

data class Folder(val id: String, val name: String, val server: String, val insecure: Boolean,
    val pairingState: String = "legacy", val verification: String? = null)
enum class TransferStatus { QUEUED, UPLOADING, PAUSED, FAILED, UPLOADED }
data class Transfer(
    val id: String, val folderId: String, val name: String, val size: Long,
    val status: TransferStatus, val uploaded: Long, val error: String?,
    val deliveryId: String?, val workId: String, val createdAt: Long,
    val autoRevision: String? = null, val autoUnmetered: Boolean = false,
    val relativePath: String? = null, val sourceVersion: Long? = null, val sourceSha256: String? = null,
    val received: Boolean = false, val conflict: Boolean = false, val superseded: Boolean = false,
)

data class AutoSource(
    val folderId: String, val treeUri: String, val name: String, val revision: String,
    val enabled: Boolean, val unmetered: Boolean, val lastScan: Long?, val error: String?,
    val waiting: Int, val skipped: Int,
    val prepared: Boolean = false,
    val directorySync: Boolean = false,
) {
    val attentionCount get() = skipped + if (error != null) 1 else 0
    val attentionSummary get() = "$waiting waiting · $attentionCount need attention" + if (error != null) " (includes a source issue)" else ""
}

internal data class SourceFile(val documentId: String, val uri: android.net.Uri, val name: String, val size: Long?, val modified: Long?, val relativePath: String = name) {
    // Document IDs are provider identities, never filesystem paths.
    val fingerprint get() = "$size:$modified:$name"
    val eligible get() = size != null && size in 1..MAX_FILE_BYTES && modified != null && modified > 0
}

internal data class StagedFile(val id: String, val name: String, val size: Long, val sha256: String)

internal data class HashedSource(val file: SourceFile, val sha256: String)
data class DirectoryPreview(val id: String, val tree: String, val name: String, val identical: Int,
    val missing: List<String>, val different: List<String>, val destinationOnly: Int)

internal object DirectoryPaths {
    fun validate(path: String) {
        require(path.isNotEmpty() && path.toByteArray(Charsets.UTF_8).size <= 1024 && '\\' !in path) { "Unsupported relative file path." }
        val parts = path.split('/')
        require(parts.size <= 17) { "Directory nesting exceeds 16 levels." }
        require(parts.all { it.isNotBlank() && it != "." && it != ".." && it.toByteArray(Charsets.UTF_8).size <= 255 &&
            it.none(Char::isISOControl) && ':' !in it && !it.startsWith(".mirelay", ignoreCase = true) }) { "Unsafe, reserved or ambiguous filename. Rename it before initializing." }
    }
}

object InputRules {
    fun server(value: String, insecure: Boolean, debug: Boolean): String {
        val text = value.trim().trimEnd('/')
        require(text.length in 1..2048) { "Enter a server URL." }
        val uri = try { URI(text) } catch (_: Exception) { throw IllegalArgumentException("Enter a valid server URL.") }
        require(!uri.host.isNullOrBlank() && uri.rawUserInfo == null && uri.rawQuery == null && uri.rawFragment == null) {
            "URL must have a host, without credentials, query, or fragment."
        }
        require(uri.scheme == "https" || (debug && insecure && uri.scheme == "http")) {
            "Use HTTPS. Plain HTTP is available only in debug builds with explicit approval."
        }
        require(uri.port == -1 || uri.port in 1..65535) { "Invalid server port." }
        return text
    }

    fun token(value: String): String {
        val token = value.trim()
        require(token.isNotEmpty() && token.toByteArray(Charsets.UTF_8).size <= 4096 && token.none { it.isISOControl() }) {
            "Enter a valid bearer token (up to 4096 bytes)."
        }
        return token
    }

    fun filename(value: String?): String {
        val clean = value.orEmpty().map { if (it.isISOControl() || it == '/' || it == '\\') '_' else it }.joinToString("").trim()
        val candidate = if (clean.isEmpty() || clean == "." || clean == "..") "Shared file" else clean
        val result = StringBuilder()
        var bytes = 0
        candidate.codePoints().forEachOrdered { point ->
            val part = String(Character.toChars(point))
            val size = part.toByteArray(Charsets.UTF_8).size
            if (bytes + size <= 255) { result.append(part); bytes += size }
        }
        return result.toString()
    }
}

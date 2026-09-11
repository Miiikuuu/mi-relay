package io.mirelay.android

import java.util.Locale
import org.json.JSONArray
import org.json.JSONObject

/** Presentation is local. It never changes the consented transfer policy. */
enum class FolderKind(val key: String, val label: String, val icon: String) {
    GENERAL("general", "General", "folder"), PHOTOS("photos", "Photos", "photos");
    companion object { fun fromKey(key: String?) = entries.find { it.key == key } ?: GENERAL }
}

/** Versioned policy keys must fail closed when unknown, not silently widen scope. */
enum class FileFilter(val key: String, val label: String) {
    ALL("all", "All files"), IMAGES("images-v1", "Images only");
    fun skipReason(path: String): String? {
        if (this == ALL) return null
        val name = path.substringAfterLast('/').lowercase(Locale.ROOT)
        if (temporarySuffixes.any(name::endsWith)) return "Temporary download"
        return if (isImageName(name)) null else "Not an included image extension"
    }
    companion object {
        private val temporarySuffixes = listOf(".part", ".tmp", ".crdownload", ".download")
        private val imageExtensions = setOf("jpg", "jpeg", "png", "webp", "gif", "bmp", "heic", "heif", "avif", "tif", "tiff")
        fun fromKey(key: String): FileFilter = requireNotNull(entries.find { it.key == key }) { "Unsupported sync filter. Update the app before resuming." }
        // Filename classification is a selection rule, NOT content validation.
        fun isImageName(path: String) = path.substringAfterLast('/').substringAfterLast('.', "").lowercase(Locale.ROOT) in
            imageExtensions
    }
}

data class SkippedFile(val path: String, val reason: String, val fingerprint: String)
internal data class FilteredSource(val files: List<SourceFile>, val skipped: List<SkippedFile>)
internal fun FileFilter.select(files: List<SourceFile>): FilteredSource {
    val selected = mutableListOf<SourceFile>()
    val skipped = mutableListOf<SkippedFile>()
    files.sortedBy { it.relativePath }.forEach { file ->
        val reason = skipReason(file.relativePath)
        if (reason == null) selected.add(file) else skipped.add(SkippedFile(file.relativePath, reason, "${file.documentId}:${file.fingerprint}"))
    }
    return FilteredSource(selected, skipped)
}
internal fun List<SkippedFile>.toJson(): String = JSONArray().apply {
    sortedBy { it.path }.forEach { put(JSONObject().put("path", it.path).put("reason", it.reason).put("fingerprint", it.fingerprint)) }
}.toString()
internal fun skippedFromJson(json: String): List<SkippedFile> = JSONArray(json).let { rows ->
    List(rows.length()) { rows.getJSONObject(it).let { row -> SkippedFile(row.getString("path"), row.getString("reason"), row.getString("fingerprint")) } }
}

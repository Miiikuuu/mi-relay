package io.mirelay.android

/** Only app-owned, safe diagnostics may pass from a background scan to the UI. */
internal class DirectoryScanException private constructor(val userMessage: String) : IllegalArgumentException(userMessage) {
    companion object {
        fun validate(files: List<SourceFile>) {
            val unsupported = files.filterNot { it.eligible }
            if (unsupported.isNotEmpty()) {
                val file = unsupported.first()
                val reason = when {
                    file.size == 0L -> "The file is empty (0 bytes). Add content or move it out of the source directory."
                    file.size != null && file.size > MAX_FILE_BYTES -> "The file exceeds the 100 MiB limit. Use a smaller file or move it out of the source directory."
                    file.size == null || file.size < 0 -> "The file is virtual or its size is unavailable. Make it available as a regular local file."
                    else -> "The modification time is unavailable. Use a provider that supplies a valid modification time."
                }
                val more = if (unsupported.size > 1) " ${unsupported.size - 1} other unsupported file(s)." else ""
                throw DirectoryScanException("Cannot scan \"${displayPath(file.relativePath)}\": $reason$more Check again after fixing the source. This scan queued no files.")
            }
            if (files.sumOf { requireNotNull(it.size) } > 4L * 1024 * 1024 * 1024) {
                throw DirectoryScanException("Directory scan exceeds 4 GiB. Choose a smaller directory. This scan queued no files.")
            }
        }

        // Show a bounded relative path, never a provider URI, document ID or raw
        // exception. Escape controls and quotes so filenames cannot spoof UI text.
        private fun displayPath(path: String): String {
            val points = path.codePoints().toArray()
            val visible = if (points.size <= 240) points else points.take(119).toIntArray() + intArrayOf(0x2026) + points.takeLast(120).toIntArray()
            return buildString {
                visible.forEach { point ->
                    if (Character.isISOControl(point) || Character.getType(point) in listOf(Character.FORMAT.toInt(), Character.LINE_SEPARATOR.toInt(), Character.PARAGRAPH_SEPARATOR.toInt()) || point == '"'.code || point == '\\'.code) append('_')
                    else appendCodePoint(point)
                }
            }
        }
    }
}

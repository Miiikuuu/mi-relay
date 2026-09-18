package io.mirelay.android

internal enum class TransferSection(val label: String) {
    ACTIVE("In progress"), ATTENTION("Needs attention"), WAITING("Waiting for Linux"), HISTORY("Transfer history");
    companion object {
        fun of(file: Transfer) = when (file.status) {
            TransferStatus.QUEUED, TransferStatus.UPLOADING -> ACTIVE
            TransferStatus.PAUSED, TransferStatus.FAILED -> ATTENTION
            TransferStatus.UPLOADED -> if (file.relativePath != null && !file.received && !file.superseded) WAITING else HISTORY
        }
    }
}

/** Stable within each section; every record appears exactly once. */
internal fun transferSections(files: List<Transfer>): List<Pair<TransferSection, List<Transfer>>> {
    val groups = files.groupBy(TransferSection::of)
    return TransferSection.entries.mapNotNull { section -> groups[section]?.let { section to it } }
}

internal fun transferProgress(uploaded: Long, size: Long): Float = when {
    size <= 0 || uploaded <= 0 -> 0f
    uploaded >= size -> 1f
    else -> (uploaded.toDouble() / size.toDouble()).toFloat().coerceIn(0f, 1f)
}

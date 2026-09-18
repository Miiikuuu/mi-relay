package io.mirelay.android

import org.junit.Assert.*
import org.junit.Test

class TransferSectionsTest {
    private fun file(id: String, status: TransferStatus) = Transfer(id, "folder", id, 100, status, 0, null, null, "work-$id", 1)

    @Test fun everyStateAppearsOnceWithActiveFirstAndStableOrder() {
        val input = listOf(file("done", TransferStatus.UPLOADED), file("paused", TransferStatus.PAUSED),
            file("upload", TransferStatus.UPLOADING), file("failed", TransferStatus.FAILED), file("queued", TransferStatus.QUEUED))
        val grouped = transferSections(input)
        assertEquals(listOf(TransferSection.ACTIVE, TransferSection.ATTENTION, TransferSection.HISTORY), grouped.map { it.first })
        assertEquals(listOf("upload", "queued", "paused", "failed", "done"), grouped.flatMap { it.second }.map { it.id })
        assertEquals(input.toSet(), grouped.flatMap { it.second }.toSet())
    }

    @Test fun serverUploadIsNotCalledReceivedAndSupersededIsHistory() {
        val uploaded = file("uploaded", TransferStatus.UPLOADED).copy(relativePath = "existing/empty.txt")
        assertEquals(TransferSection.WAITING, TransferSection.of(uploaded))
        assertEquals(TransferSection.HISTORY, TransferSection.of(uploaded.copy(received = true)))
        assertEquals(TransferSection.HISTORY, TransferSection.of(uploaded.copy(superseded = true)))
        assertTrue(transferSections(emptyList()).isEmpty())
    }

    @Test fun progressIsFiniteForEmptyFilesAndBadCounters() {
        for (size in listOf(-1L, 0L, 1L, Long.MAX_VALUE)) for (uploaded in listOf(-1L, 0L, 1L, Long.MAX_VALUE)) {
            assertTrue(transferProgress(uploaded, size).isFinite())
            assertTrue(transferProgress(uploaded, size) in 0f..1f)
        }
        assertEquals(0.5f, transferProgress(50, 100), 0.00001f)
        assertEquals(0f, transferProgress(0, 0), 0f)
    }
}

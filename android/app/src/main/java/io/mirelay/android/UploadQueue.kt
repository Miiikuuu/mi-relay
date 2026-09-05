package io.mirelay.android

import android.content.Context
import androidx.work.BackoffPolicy
import androidx.work.Constraints
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.workDataOf
import java.util.concurrent.TimeUnit

class UploadQueue(context: Context, private val store: RelayStore) {
    private val manager = WorkManager.getInstance(context)
    @Synchronized fun enqueue(id: String, manual: Boolean = false) {
        if (manual) store.automatic.makeManual(id)
        val transfer = requireNotNull(store.transfer(id))
        // A pause may win the race between staging's DB commit and this call.
        // Background recovery must never act like an explicit Resume button.
        if (!manual && transfer.status !in listOf(TransferStatus.QUEUED, TransferStatus.UPLOADING)) return
        check(store.automatic.permits(transfer)) { "Auto is paused. Resume this file manually if needed." }
        val automatic = transfer.autoRevision != null
        val constraints = Constraints.Builder().setRequiredNetworkType(if (automatic && transfer.autoUnmetered) NetworkType.UNMETERED else NetworkType.CONNECTED)
        if (automatic) constraints.setRequiresBatteryNotLow(true).setRequiresStorageNotLow(true)
        val builder = OneTimeWorkRequestBuilder<UploadWorker>()
            .setInputData(workDataOf("transfer_id" to id))
            .setConstraints(constraints.build())
            .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 15, TimeUnit.SECONDS)
        if (automatic) builder.addTag("auto:${transfer.folderId}")
        val request = builder.build()
        store.assign(id, request.id.toString())
        manager.enqueueUniqueWork("upload:$id", ExistingWorkPolicy.REPLACE, request).result.get()
    }
    @Synchronized fun pause(id: String) {
        store.pause(id) // Invalidate ownership before cancellation reaches the worker.
        manager.cancelUniqueWork("upload:$id").result.get()
    }
    @Synchronized fun pauseAutomatic(folder: String, reason: String? = null) {
        store.automatic.disable(folder, reason)
        manager.cancelAllWorkByTag("auto:$folder").result.get()
    }
    @Synchronized fun recover(autoSource: AutoSource? = null) {
        store.refresh()
        for (transfer in store.transfers.value) {
            if (autoSource != null && (transfer.folderId != autoSource.folderId || transfer.autoRevision != autoSource.revision)) continue
            if (transfer.status !in listOf(TransferStatus.QUEUED, TransferStatus.UPLOADING)) continue
            if (!store.automatic.permits(transfer)) { store.pause(transfer.id); continue }
            val jobs = manager.getWorkInfosForUniqueWork("upload:${transfer.id}").get()
            if (jobs.none { !it.state.isFinished }) enqueue(transfer.id)
        }
    }
}

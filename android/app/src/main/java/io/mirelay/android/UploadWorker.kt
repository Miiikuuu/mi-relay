package io.mirelay.android

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.SystemClock
import androidx.core.app.NotificationCompat
import androidx.work.ForegroundInfo
import androidx.work.Worker
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import org.json.JSONObject
import java.io.File

class UploadWorker(context: Context, params: WorkerParameters) : Worker(context, params) {
    private val app = context.applicationContext as RelayApplication
    private val store = app.store
    override fun doWork(): Result {
        val transferId = inputData.getString("transfer_id") ?: return Result.failure()
        val workId = id.toString()
        val transfer = store.transfer(transferId) ?: return Result.failure()
        if (transfer.status == TransferStatus.UPLOADED) return Result.success()
        if (transfer.workId != workId || isStopped || !store.automatic.permits(transfer)) return Result.success()
        try {
            val source = transfer.autoRevision?.let { revision -> store.automatic.source(transfer.folderId)?.takeIf { it.revision == revision } }
            if (transfer.autoRevision != null && (source == null || !app.auto.hasAccess(source))) return Result.success()
            val folder = requireNotNull(store.folder(transfer.folderId)) { "Folder no longer exists." }
            if (!store.updateOwned(transferId, workId, TransferStatus.UPLOADING)) return Result.success()
            setForegroundAsync(notification()).get()
            check(NativeBridge.initialize(applicationContext)) { "Could not initialize Android certificate verification." }
            val dir = store.directory(transferId)
            if (transfer.relativePath != null) verifyDirectoryPayload(File(dir,"payload"), transfer)
            val request = JSONObject().apply {
                put("file", File(dir, "payload").path); put("state_file", File(dir, "resume.json").path)
                put("server_url", folder.server); put("token", store.token(folder.id)); put("name", transfer.name)
                put("chunk_size_bytes", 1024 * 1024); put("max_chunks", JSONObject.NULL)
                put("allow_insecure_http", folder.insecure && BuildConfig.DEBUG)
                put("request_timeout_seconds", 30); put("keep_completed_state", true)
                transfer.relativePath?.let { path ->
                    DirectoryPaths.validate(path)
                    require(transfer.sourceVersion != null && transfer.sourceVersion in 1 until Long.MAX_VALUE)
                    require(transfer.sourceSha256?.matches(Regex("[0-9a-f]{64}")) == true)
                    put("directory", JSONObject().put("path", path).put("version", transfer.sourceVersion))
                }
            }
            var lastUpdate = 0L
            val response = JSONObject(NativeBridge.upload(request.toString()) { uploaded, total ->
                if (isStopped || store.transfer(transferId)?.workId != workId || !store.automatic.permits(transfer) || (source != null && !app.auto.hasAccess(source))) false else {
                    val now = SystemClock.elapsedRealtime()
                    if (now - lastUpdate >= 250 || uploaded == total) {
                        store.updateOwned(transferId, workId, TransferStatus.UPLOADING, uploaded)
                        setProgressAsync(workDataOf("uploaded" to uploaded, "total" to total))
                        lastUpdate = now
                    }
                    true
                }
            })
            if (isStopped || store.transfer(transferId)?.workId != workId || !store.automatic.permits(transfer)) return Result.success()
            if (response.has("error")) {
                val retry = response.optBoolean("retryable") && runAttemptCount < 4
                store.updateOwned(transferId, workId, if (retry) TransferStatus.QUEUED else TransferStatus.FAILED,
                    error = response.getString("error"))
                return if (retry) Result.retry() else Result.failure()
            }
            val result = response.getJSONObject("result")
            if(transfer.relativePath != null) check(result.getString("sha256") == transfer.sourceSha256 && result.getLong("size") == transfer.size) { "Directory upload differs from its durable version." }
            if (result.isNull("delivery_id")) {
                store.updateOwned(transferId, workId, TransferStatus.PAUSED, result.getLong("uploaded_bytes"))
                return Result.success()
            }
            // The durable local receipt comes BEFORE removing the replayable tus session.
            // Build a bounded, disposable preview BEFORE publishing completion or
            // deleting the staging original. This never changes the sync source.
            if (FileFilter.isImageName(transfer.name)) store.photoPreviews.prepare(transferId, File(dir, "payload"))
            if (store.updateOwned(transferId, workId, TransferStatus.UPLOADED, result.getLong("size"), delivery = result.getString("delivery_id"))) {
                File(dir, "payload").delete()
                File(dir, "resume.json").delete()
            }
            return Result.success()
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
            return Result.retry()
        } catch (_: Exception) {
            // Do not leak token/URI values through arbitrary provider or keystore errors.
            if (!isStopped) store.updateOwned(transferId, workId, TransferStatus.FAILED,
                error = "Could not start or save this transfer. Check storage and the Folder token, then retry.")
            return Result.failure()
        } catch (_: UnsatisfiedLinkError) {
            store.updateOwned(transferId, workId, TransferStatus.FAILED, error = "The native transfer library is missing for this device architecture.")
            return Result.failure()
        }
    }

    private fun notification(): ForegroundInfo {
        val channel = "transfers"
        val manager = applicationContext.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel(channel, "Transfers", NotificationManager.IMPORTANCE_LOW))
        val open = PendingIntent.getActivity(applicationContext, 0, Intent(applicationContext, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val notification = NotificationCompat.Builder(applicationContext, channel)
            .setSmallIcon(R.drawable.ic_relay).setContentTitle("MiRelay")
            .setContentText("Uploading to your server").setOngoing(true).setOnlyAlertOnce(true)
            .setVisibility(NotificationCompat.VISIBILITY_PRIVATE).setContentIntent(open).build()
        val notificationId = id.hashCode() and Int.MAX_VALUE
        return if (Build.VERSION.SDK_INT >= 29) ForegroundInfo(notificationId, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        else ForegroundInfo(notificationId, notification)
    }
}

internal fun verifyDirectoryPayload(file: File, transfer: Transfer) {
    require(transfer.sourceSha256?.matches(Regex("[0-9a-f]{64}")) == true && file.isFile && file.length() == transfer.size) { "Staged directory file is unavailable or changed." }
    val hash=java.security.MessageDigest.getInstance("SHA-256")
    file.inputStream().use { input ->
        val buffer=ByteArray(64*1024);var size=0L
        while(true) {val count=input.read(buffer);if(count<0)break;check(count>0);size+=count;require(size<=transfer.size);hash.update(buffer,0,count)}
        require(size==transfer.size)
    }
    require(hash.digest().joinToString("") {"%02x".format(it.toInt() and 255)} == transfer.sourceSha256) { "Staged directory file differs from its recorded hash." }
}

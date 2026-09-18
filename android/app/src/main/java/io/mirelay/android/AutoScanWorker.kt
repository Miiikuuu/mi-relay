package io.mirelay.android

import android.content.Context
import android.os.OperationCanceledException
import androidx.work.Worker
import androidx.work.WorkerParameters

class AutoScanWorker(context: Context, parameters: WorkerParameters) : Worker(context, parameters) {
    @Volatile private var session: AutoSession? = null
    override fun doWork(): Result {
        val app = applicationContext as RelayApplication
        val folder = inputData.getString("folder_id") ?: return Result.failure()
        val revision = inputData.getString("revision") ?: return Result.failure()
        val source = app.store.automatic.source(folder)?.takeIf { it.enabled && it.revision == revision } ?: return Result.success()
        return try {
            val stopped = { isStopped || !app.store.automatic.active(folder, revision) }
            val scanSession = if (source.directorySync) AutoSession.directoryScan(stopped, background = true)
                else AutoSession(stopped = stopped)
            scanSession.use { operation ->
                session = operation
                val pending = app.auto.scan(source, operation)
                // At most ONE delayed stability follow-up per periodic/manual check, not a poll loop.
                if (pending && operation.directoryProgress) app.auto.continueDirectory(source,inputData.getInt("batch",0)+1)
                else if (pending && !inputData.getBoolean("settle", false)) app.auto.settle(source)
            }
            Result.success()
        } catch (_: SecurityException) {
            app.auto.pauseIfCurrent(source, "Source access was revoked. Choose the directory again to enable Auto.")
            Result.failure()
        } catch (_: OperationCanceledException) {
            if (!isStopped && app.store.automatic.active(folder, revision)) {
                app.store.automatic.scanResult(source, System.currentTimeMillis(), "Source check stopped making progress or reached its background time budget. No partial scan was accepted. Keep the source locally available and retry.")
            }
            if (isStopped || runAttemptCount >= 3) Result.failure() else Result.retry()
        } catch (error: DirectoryScanException) {
            app.store.automatic.scanResult(source, System.currentTimeMillis(), error.userMessage)
            if (runAttemptCount < 3) Result.retry() else Result.failure()
        } catch (_: Exception) {
            app.store.automatic.scanResult(source, System.currentTimeMillis(), "Could not completely check this source. Access, nesting, loading state and the 5,000-entry limit must allow a complete scan.")
            if (runAttemptCount < 3) Result.retry() else Result.failure()
        } finally { session = null }
    }
    override fun onStopped() { session?.cancel(); super.onStopped() }
}

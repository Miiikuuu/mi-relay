package io.mirelay.android

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.OperationCanceledException
import androidx.core.net.toUri
import androidx.work.BackoffPolicy
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.workDataOf
import java.util.concurrent.TimeUnit

class AutoCoordinator(private val context: Context, private val store: RelayStore, private val uploads: UploadQueue) {
    private val manager = WorkManager.getInstance(context)
    private val resolver = context.contentResolver
    private val reader = DirectorySource(resolver)
    private val directory: DirectoryRemote = DirectoryConnection(context)

    @Synchronized internal fun previewDirectory(folderId: String, tree: Uri, filter: FileFilter = FileFilter.ALL): DirectoryPreview {
        val folder = requireNotNull(store.folder(folderId))
        val previous = store.automatic.source(folderId)
        store.directorySync.requirePreviewAllowed(folderId, tree.toString())
        require(folder.pairingState != "legacy") { "Directory sync requires a paired Folder. Create one on Linux and pair it here first." }
        DirectorySource.validate(tree)
        val granted = resolver.persistedUriPermissions.any { it.uri == tree && it.isReadPermission }
        try {
            resolver.takePersistableUriPermission(tree, Intent.FLAG_GRANT_READ_URI_PERMISSION)
            return AutoSession().use { session ->
                val snapshot = reader.snapshot(tree, session, true)
                val selection = filter.select(snapshot.files)
                val files = reader.hashes(tree, selection.files, session)
                val remote = directory.state(folder, store.token(folderId))
                val comparison = directory.compare(files.inventory(), remote)
                session.check(); reader.requirePermission(tree)
                store.directorySync.savePreview(folder, tree.toString(), snapshot.name, files, remote, comparison, filter, selection.skipped)
            }
        } catch (error: Exception) {
            if (!granted && previous?.treeUri != tree.toString() && store.directorySync.preview(folderId)?.tree != tree.toString()) {
                try { resolver.releasePersistableUriPermission(tree, Intent.FLAG_GRANT_READ_URI_PERMISSION) } catch (_: SecurityException) { }
            }
            throw error
        }
    }
    @Synchronized internal fun confirmDirectory(folderId: String, tree: Uri, previewId: String, unmetered: Boolean, filter: FileFilter = FileFilter.ALL) {
        val folder = requireNotNull(store.folder(folderId))
        val source = AutoSession().use { session ->
            val selection = filter.select(reader.snapshot(tree, session, true).files)
            val files = reader.hashes(tree, selection.files, session)
            val remote = directory.state(folder, store.token(folderId))
            directory.compare(files.inventory(), remote)
            session.check(); reader.requirePermission(tree)
            store.directorySync.confirm(folder, previewId, tree.toString(), files, remote, unmetered, System.currentTimeMillis(), filter, selection.skipped)
        }
        startDirectory(source)
    }
    @Synchronized internal fun resumeDirectory(folderId: String, tree: Uri, unmetered: Boolean) {
        val folder = requireNotNull(store.folder(folderId))
        require(store.automatic.source(folderId)?.treeUri == tree.toString()) { "Choose the initialized directory to restore access." }
        resolver.takePersistableUriPermission(tree, Intent.FLAG_GRANT_READ_URI_PERMISSION)
        reader.requirePermission(tree)
        directory.state(folder, store.token(folderId)) // Validate role, pairing and protocol before resuming.
        startDirectory(store.directorySync.resume(folderId, tree.toString(), unmetered))
    }
    private fun startDirectory(source: AutoSource) {
        try { schedule(source); uploads.recover(source); checkNow(source.folderId) }
        catch (error: Exception) { uploads.pauseAutomatic(source.folderId, "Directory sync is initialized but scheduling failed. Resume to retry without resetting it."); throw error }
    }
    internal fun refreshReceipts(folderId: String) {
        require(store.automatic.source(folderId)?.directorySync == true) { "Initialize directory sync first." }
        val folder = requireNotNull(store.folder(folderId))
        store.directorySync.receipts(folderId, directory.state(folder, store.token(folderId)))
    }

    @Synchronized fun enable(folder: String, tree: Uri, unmetered: Boolean, includeExisting: Boolean = false, startImmediately: Boolean = true) {
        requireNotNull(store.folder(folder)) { "Folder no longer exists." }
        val previous = store.automatic.source(folder)
        require(previous?.enabled != true) { "Pause Auto before changing its source." }
        DirectorySource.validate(tree)
        if (startImmediately && previous?.prepared == true && previous.treeUri == tree.toString()) {
            reader.requirePermission(tree)
            val source = store.automatic.startPrepared(folder, unmetered)
            try { schedule(source); checkNow(folder) }
            catch (error: Exception) { uploads.pauseAutomatic(folder, "Auto could not be scheduled. Enable it again to retry."); throw error }
            return
        }
        val alreadyGranted = resolver.persistedUriPermissions.any { it.uri == tree && it.isReadPermission }
        try {
            // Only read access is retained, even if a picker offered write access too.
            resolver.takePersistableUriPermission(tree, Intent.FLAG_GRANT_READ_URI_PERMISSION)
            val snapshot = AutoSession().use { reader.snapshot(tree, it) }
            val source = store.automatic.enable(folder, tree.toString(), snapshot.name, unmetered, snapshot.files, System.currentTimeMillis(), includeExisting, startImmediately)
            try { if (startImmediately) { schedule(source); if (includeExisting) checkNow(folder) } }
            catch (error: Exception) {
                uploads.pauseAutomatic(folder, "Auto could not be scheduled. Enable it again to retry.")
                throw error
            }
            if (previous != null && previous.treeUri != tree.toString() && store.automatic.sources.value.none { it.treeUri == previous.treeUri }) {
                try { resolver.releasePersistableUriPermission(previous.treeUri.toUri(), Intent.FLAG_GRANT_READ_URI_PERMISSION) } catch (_: SecurityException) { }
            }
        } catch (error: Exception) {
            store.automatic.refresh()
            if (!alreadyGranted && store.automatic.sources.value.none { it.treeUri == tree.toString() }) {
                try { resolver.releasePersistableUriPermission(tree, Intent.FLAG_GRANT_READ_URI_PERMISSION) } catch (_: SecurityException) { }
            }
            throw error
        }
    }
    @Synchronized fun pause(folder: String, reason: String? = null) = uploads.pauseAutomatic(folder, reason)
    @Synchronized internal fun pauseIfCurrent(source: AutoSource, reason: String) {
        if (store.automatic.active(source.folderId, source.revision)) pause(source.folderId, reason)
    }
    internal fun hasAccess(source: AutoSource): Boolean {
        if (!store.automatic.active(source.folderId, source.revision)) return false
        return try { reader.requirePermission(source.treeUri.toUri()); true }
        catch (_: SecurityException) {
            pauseIfCurrent(source, "Source access was revoked. Choose the directory again to enable Auto.")
            false
        }
    }
    @Synchronized fun recover() {
        store.automatic.refresh()
        for (source in store.automatic.sources.value.filter { it.enabled }) {
            try { reader.requirePermission(source.treeUri.toUri()); schedule(source) }
            catch (_: SecurityException) { pause(source.folderId, "Source access was revoked. Choose the directory again to enable Auto.") }
        }
    }
    private fun constraints(source: AutoSource) = Constraints.Builder()
        .setRequiredNetworkType(if (source.unmetered) NetworkType.UNMETERED else NetworkType.CONNECTED)
        .setRequiresBatteryNotLow(true).setRequiresStorageNotLow(true).build()

    private fun schedule(source: AutoSource) {
        val request = PeriodicWorkRequestBuilder<AutoScanWorker>(30, TimeUnit.MINUTES)
            .setInitialDelay(30, TimeUnit.MINUTES).setConstraints(constraints(source))
            .setInputData(workDataOf("folder_id" to source.folderId, "revision" to source.revision))
            .addTag("auto:${source.folderId}").setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 30, TimeUnit.SECONDS).build()
        manager.enqueueUniquePeriodicWork("auto-periodic:${source.folderId}", ExistingPeriodicWorkPolicy.KEEP, request).result.get()
    }
    @Synchronized fun checkNow(folder: String) {
        val source = requireNotNull(store.automatic.source(folder))
        require(source.enabled) { "Enable Auto before checking." }
        enqueueCheck(source, settle = false)
    }
    @Synchronized internal fun settle(source: AutoSource) {
        if (store.automatic.active(source.folderId, source.revision)) enqueueCheck(source, settle = true)
    }
    @Synchronized internal fun continueDirectory(source: AutoSource, batch: Int) {
        if (!source.directorySync || batch !in 1..250 || !store.automatic.active(source.folderId, source.revision)) return
        val request = OneTimeWorkRequestBuilder<AutoScanWorker>()
            .setInputData(workDataOf("folder_id" to source.folderId, "revision" to source.revision, "batch" to batch, "settle" to true))
            .setInitialDelay(1, TimeUnit.SECONDS).setConstraints(constraints(source))
            .addTag("auto:${source.folderId}").setBackoffCriteria(BackoffPolicy.EXPONENTIAL,30,TimeUnit.SECONDS).build()
        manager.enqueueUniqueWork("auto-batch:${source.folderId}:${source.revision}:$batch",ExistingWorkPolicy.KEEP,request).result.get()
    }
    private fun enqueueCheck(source: AutoSource, settle: Boolean) {
        val request = OneTimeWorkRequestBuilder<AutoScanWorker>()
            .setInputData(workDataOf("folder_id" to source.folderId, "revision" to source.revision, "settle" to settle))
            .setInitialDelay(if (settle) AutoStore.STABLE_MILLIS else 0, TimeUnit.MILLISECONDS)
            .setConstraints(constraints(source)).addTag("auto:${source.folderId}")
            .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 30, TimeUnit.SECONDS).build()
        val kind = if (settle) "settle" else "check"
        manager.enqueueUniqueWork("auto-$kind:${source.folderId}", ExistingWorkPolicy.KEEP, request).result.get()
    }

    internal fun scan(source: AutoSource, session: AutoSession, clock: () -> Long = System::currentTimeMillis): Boolean {
        if (!store.automatic.active(source.folderId, source.revision)) return false
        if (source.directorySync) return scanDirectory(source, session, clock)
        val tree = source.treeUri.toUri()
        val files = reader.snapshot(tree, session).files
        val ready = store.automatic.observe(source, files, clock())
        var stagedBytes = 0L
        for (file in ready.take(MAX_SHARED_FILES)) {
            session.check()
            if (!store.automatic.active(source.folderId, source.revision)) return false
            if (stagedBytes + requireNotNull(file.size) > MAX_FILE_BYTES) break
            stagedBytes += file.size
            try {
                check(reader.metadata(tree, file, session).fingerprint == file.fingerprint) { "Source changed." }
                FileImporter(resolver, store).stage(file.uri, session) { staged ->
                    check(staged.size == file.size && reader.metadata(tree, file, session).fingerprint == file.fingerprint) { "Source changed while copying." }
                    session.check(); reader.requirePermission(tree)
                    store.automatic.commit(source, file, staged)
                }?.let { uploads.enqueue(it) }
            } catch (error: SecurityException) { throw error }
            catch (error: OperationCanceledException) { throw error }
            catch (_: Exception) {
                session.check()
                store.automatic.fileError(source, file, "File changed, could not be read, or could not be queued. Auto will try again later; manual sending is available.", clock())
            }
        }
        // A process may have died between the atomic DB commit and WorkManager enqueue.
        uploads.recover(source)
        store.automatic.scanResult(source, clock())
        return store.automatic.needsSettle(source)
    }

    private fun scanDirectory(source: AutoSource, session: AutoSession, clock: () -> Long): Boolean {
        val tree = source.treeUri.toUri()
        val folder = requireNotNull(store.folder(source.folderId))
        val remote = directory.state(folder, store.token(source.folderId))
        store.directorySync.receipts(source.folderId, remote)
        val selection = source.fileFilter.select(reader.snapshot(tree, session, true).files)
        val files = reader.hashes(tree, selection.files, session)
        directory.compare(files.inventory(), remote)
        session.check()
        val ready = store.directorySync.observe(source, files, clock())
        var stagedBytes = 0L
        for (value in ready.take(MAX_SHARED_FILES)) {
            session.check()
            if (!store.automatic.active(source.folderId, source.revision)) return false
            if (stagedBytes + requireNotNull(value.file.size) > MAX_FILE_BYTES) break
            stagedBytes += value.file.size
            try {
                FileImporter(resolver, store).stage(value.file.uri, session, exactName = value.file.name) { staged ->
                    check(reader.metadata(tree, value.file, session, true).fingerprint == value.file.fingerprint) { "Source changed." }
                    session.check(); reader.requirePermission(tree)
                    store.directorySync.commit(source, value, staged).also { if(it) session.directoryProgress=true }
                }?.let { uploads.enqueue(it) }
            } catch (error: SecurityException) { throw error }
            catch (error: OperationCanceledException) { throw error }
            catch (_: Exception) { session.check(); store.directorySync.fileError(source, value, clock()) }
        }
        uploads.recover(source)
        store.directorySync.filteredResult(source, selection.skipped.size)
        store.automatic.scanResult(source, clock())
        return store.directorySync.needsSettle(source)
    }
}

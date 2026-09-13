package io.mirelay.android

import android.app.Application
import android.net.Uri
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

class RelayViewModel(application: Application) : AndroidViewModel(application) {
    private val app = application as RelayApplication
    val folders = app.store.folders
    val transfers = app.store.transfers
    val selected = MutableStateFlow<String?>(null)
    val pending = MutableStateFlow<List<Uri>>(emptyList())
    val busy = MutableStateFlow(false)
    val error = MutableStateFlow<String?>(null)
    val autoSources = app.store.automatic.sources
    val autoEditor = MutableStateFlow<String?>(null)
    val autoTree = MutableStateFlow<Uri?>(null)
    val creationTree = MutableStateFlow<Uri?>(null)
    val directoryPreview = MutableStateFlow<DirectoryPreview?>(null)
    val directoryFilter = MutableStateFlow(FileFilter.ALL)
    private val connection = FolderConnection(application)

    init { task { withContext(Dispatchers.IO) { app.auto.recover(); app.uploads.recover() } } }

    fun openAuto(folder: String) {
        if (app.store.folder(folder)?.connectionClosed != false) {
            error.value = "This Folder is stopped. Open Folder settings to retry disconnection or remove it."
            return
        }
        directoryPreview.value = null
        autoEditor.value = folder
        directoryFilter.value = app.store.automatic.sources.value.find { it.folderId == folder }?.takeIf { it.directorySync }?.fileFilter
            ?: if (app.store.folders.value.find { it.id == folder }?.kind == FolderKind.PHOTOS) FileFilter.IMAGES else FileFilter.ALL
        autoTree.value = app.store.automatic.sources.value.find { it.folderId == folder }?.treeUri?.let(Uri::parse)
        task {
            directoryPreview.value = withContext(Dispatchers.IO) { app.store.directorySync.preview(folder) }
            directoryPreview.value?.let { if (autoEditor.value == folder) { autoTree.value = Uri.parse(it.tree); directoryFilter.value = it.fileFilter } }
        }
    }
    fun selectDirectoryFilter(filter: FileFilter) {
        if (busy.value) return
        directoryFilter.value = filter
        directoryPreview.value = null
    }
    fun setFolderKind(id: String, kind: FolderKind) = task {
        withContext(Dispatchers.IO) { app.store.setFolderKind(id, kind) }
    }
    fun acceptAutoTree(folder: String, tree: Uri) {
        if (folder == "new-folder") {
            try { DirectorySource.validate(tree); creationTree.value = tree }
            catch (_: IllegalArgumentException) { error.value = "Choose a source directory using the system picker." }
            return
        }
        if (app.store.folders.value.none { it.id == folder }) return
        val source = app.store.automatic.sources.value.find { it.folderId == folder }
        if(source?.directorySync == true && source.treeUri != tree.toString()) { error.value="Choose the initialized directory to restore access. Create a new Folder to select a different directory.";return }
        try { DirectorySource.validate(tree) }
        catch (_: IllegalArgumentException) { error.value = "Choose a source directory using the system picker."; return }
        autoEditor.value = folder; autoTree.value = tree
        directoryPreview.value = null
    }
    fun previewDirectory(folder: String) = task {
        directoryPreview.value = null
        val tree = requireNotNull(autoTree.value) { "Choose a source directory." }
        val filter = directoryFilter.value
        directoryPreview.value = withContext(Dispatchers.IO) { app.auto.previewDirectory(folder, tree, filter) }
    }
    fun confirmDirectory(folder: String, unmetered: Boolean) = task {
        val tree = requireNotNull(autoTree.value) { "Choose a source directory." }
        val preview = requireNotNull(directoryPreview.value) { "Preview both directories before initializing." }
        require(preview.tree == tree.toString()) { "Source changed. Preview again." }
        require(preview.fileFilter == directoryFilter.value) { "Filter changed. Preview again." }
        withContext(Dispatchers.IO) { app.auto.confirmDirectory(folder, tree, preview.id, unmetered, preview.fileFilter) }
        directoryPreview.value = null
    }
    fun resumeDirectory(folder: String, unmetered: Boolean) = task {
        val tree = requireNotNull(autoTree.value) { "Choose the initialized source directory." }
        withContext(Dispatchers.IO) { app.auto.resumeDirectory(folder, tree, unmetered) }
    }
    fun refreshReceipts(folder: String) = task { withContext(Dispatchers.IO) { app.auto.refreshReceipts(folder) } }
    fun enableAuto(folder: String, unmetered: Boolean, includeExisting: Boolean = false) = task {
        val tree = requireNotNull(autoTree.value) { "Choose a source directory." }
        try { withContext(Dispatchers.IO) { requireReady(folder); app.auto.enable(folder, tree, unmetered, includeExisting) } }
        catch (e: IllegalArgumentException) { error.value = e.message }
        catch (_: SecurityException) { error.value = "Source access is unavailable. Choose the directory again." }
        catch (_: Exception) { error.value = "Auto was not enabled. A complete, readable source of at most 5,000 entries is required. Check access and free space, then try again." }
    }
    fun pauseAuto(folder: String) = task { withContext(Dispatchers.IO) { app.auto.pause(folder) } }
    fun checkAuto(folder: String) = task { withContext(Dispatchers.IO) { app.auto.checkNow(folder) } }

    fun accept(uris: List<Uri>) {
        // A cold-start share can arrive while queue recovery is still running.
        // Keep its URIs; confirmation stays disabled until preparation finishes.
        val files = uris.distinct()
        if (files.isEmpty() || files.size > MAX_SHARED_FILES || files.any { it.scheme != "content" }) {
            error.value = "Choose up to 20 files using Android's file picker or share menu. Text-only shares are not supported yet."
            return
        }
        pending.value = files
    }

    fun saveFolder(folder: Folder?, name: String, server: String, token: String, insecure: Boolean,
        pairingCode: String = "", includeExisting: Boolean = false, kind: FolderKind = folder?.kind ?: FolderKind.GENERAL, done: () -> Unit) = task {
        val tree = if (folder == null) creationTree.value else null
        val id = withContext(Dispatchers.IO) {
            val base = InputRules.server(server, insecure, BuildConfig.DEBUG)
            val id = if (folder == null && pairingCode.isNotBlank()) {
                val code = pairingCode.trim()
                val scoped = connection.folderUrl(base, code)
                val draft = app.store.folders.value.find { it.server == scoped }
                require(draft == null || draft.pairingState == "awaiting_peer") { "This Folder is already linked. Open its settings or check pairing." }
                // Persist the encrypted sender credential BEFORE claiming. A lost
                // response or process death can retry the same invitation safely.
                val secret = draft?.let { app.store.token(it.id) } ?: connection.newSenderToken()
                val key = app.store.saveFolder(draft?.id, name, scoped, secret, insecure, "awaiting_peer")
                selected.value = key
                val info = connection.claim(base, code, secret, insecure)
                app.store.pairingResult(key, info.getString("state"), info.optString("verification").takeIf { it != "null" && it.isNotBlank() })
                key
            } else {
                val secret = if (token.isBlank() && folder != null) app.store.token(folder.id) else InputRules.token(token)
                val info = connection.check(Folder(folder?.id.orEmpty(), name, base, insecure), secret)
                val key = app.store.saveFolder(folder?.id, name, base, secret, insecure)
                info?.let { app.store.pairingResult(key, it.getString("state"), it.optString("verification").takeIf { value -> value != "null" && value.isNotBlank() }) }
                key
            }
            app.store.setFolderKind(id, kind)
            if (tree != null) {
                try { app.auto.enable(id, tree, true, includeExisting, startImmediately = false) }
                catch (_: Exception) { error.value = "Folder connection saved, but the source directory could not be initialized. Open Auto to choose it again. Nothing was automatically sent." }
            }
            id
        }
        selected.value = id
        creationTree.value = null
        done()
    }

    fun checkPairing(folder: String) = task { withContext(Dispatchers.IO) { checkConnection(folder) } }
    fun disconnectFolder(id: String) = task {
        withContext(Dispatchers.IO) {
            val folder = requireNotNull(app.store.folder(id)) { "Folder no longer exists." }
            if (folder.pairingState == "disconnected") return@withContext
            require(folder.scoped) { "This is a legacy connection. Only local removal is available." }
            app.auto.stopFolder(id)
            try {
                connection.disconnect(folder, app.store.token(id))
                app.store.finishDisconnect(id)
            } catch (_: Exception) {
                error.value = "Stopped locally, but server disconnection is not confirmed. Keep this Folder and use Retry disconnect. Check connectivity and update the relay if it does not support disconnection."
            }
        }
    }
    fun removeFolder(id: String, done: () -> Unit) = task {
        withContext(Dispatchers.IO) {
            val folder = requireNotNull(app.store.folder(id)) { "Folder no longer exists." }
            require(!folder.scoped || folder.pairingState == "disconnected") { "Disconnect this Folder successfully before removing it." }
            app.auto.stopFolder(id)
            app.store.removeFolder(id)
        }
        if (selected.value == id) { selected.value = null; pending.value = emptyList() }
        if (autoEditor.value == id) autoEditor.value = null
        done()
    }
    private fun checkConnection(id: String): org.json.JSONObject? {
        val folder = requireNotNull(app.store.folder(id)) { "Folder no longer exists." }
        require(!folder.connectionClosed) { "Use Retry disconnect, or create a new Folder after disconnection." }
        val info = connection.check(folder, app.store.token(id))
        if (info?.optString("state") == "disconnected") app.auto.stopFolder(id)
        info?.let { app.store.pairingResult(id, it.getString("state"), it.optString("verification").takeIf { value -> value != "null" && value.isNotBlank() }) }
        return info
    }
    private fun requireReady(id: String) {
        val info = checkConnection(id)
        require(info == null || info.getString("state") == "ready") { "Awaiting pairing. Compare the verification code on Linux, confirm there, then check pairing here." }
    }

    fun send(folderId: String) = task {
        require(app.store.automatic.sources.value.none { it.folderId == folderId && it.directorySync }) { "Use a delivery Folder for one-off sends. Add files to this Folder's initialized directory to sync them." }
        withContext(Dispatchers.IO) { requireReady(folderId) }
        val files = pending.value
        pending.value = emptyList() // Duplicate taps cannot enqueue this selection twice.
        var failed = 0
        withContext(Dispatchers.IO) {
            val importer = FileImporter(app.contentResolver, app.store)
            for (uri in files) {
                try { app.uploads.enqueue(importer.import(uri, folderId)) }
                catch (_: Exception) { failed++ }
            }
        }
        if (failed > 0) error.value = "$failed file(s) could not be prepared or scheduled. Check access, free space, and the 100 MiB limit. Queued files can be retried."
    }

    fun retry(id: String) = task { withContext(Dispatchers.IO) { app.uploads.enqueue(id, manual = true) } }
    fun pause(id: String) = task { withContext(Dispatchers.IO) { app.uploads.pause(id) } }

    private fun task(block: suspend () -> Unit) {
        if (busy.value) return
        busy.value = true
        viewModelScope.launch {
            try { block() }
            catch (e: IllegalArgumentException) { error.value = e.message ?: "Invalid input." }
            catch (_: Exception) { error.value = "Could not update local data. Check free space and try again." }
            finally { busy.value = false }
        }
    }
}

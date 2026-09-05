package io.mirelay.android

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.path
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.launch
import java.util.Locale

private val ink = Color(0xFF171717)
private val paper = Color(0xFFFAFAF9)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RelayScreen(model: RelayViewModel, chooseFiles: () -> Unit, chooseDirectory: (String) -> Unit, requestNotifications: () -> Unit) {
    val dark = isSystemInDarkTheme()
    val colors = if (dark) darkColorScheme(primary = Color.White, onPrimary = ink, surface = ink, background = ink,
        surfaceContainer = Color(0xFF242424), secondary = Color(0xFFCCCCCC))
    else lightColorScheme(primary = ink, onPrimary = Color.White, surface = paper, background = paper,
        surfaceContainer = Color(0xFFF0F0EE), secondary = Color(0xFF60605D))
    MaterialTheme(colorScheme = colors) {
        val folders by model.folders.collectAsStateWithLifecycle()
        val transfers by model.transfers.collectAsStateWithLifecycle()
        val selected by model.selected.collectAsStateWithLifecycle()
        val pending by model.pending.collectAsStateWithLifecycle()
        val busy by model.busy.collectAsStateWithLifecycle()
        val error by model.error.collectAsStateWithLifecycle()
        val autoSources by model.autoSources.collectAsStateWithLifecycle()
        val autoEditor by model.autoEditor.collectAsStateWithLifecycle()
        val autoTree by model.autoTree.collectAsStateWithLifecycle()
        val creationTree by model.creationTree.collectAsStateWithLifecycle()
        val directoryPreview by model.directoryPreview.collectAsStateWithLifecycle()
        val folder = folders.find { it.id == selected } ?: folders.firstOrNull()
        val directoryMode = autoSources.any { it.folderId == folder?.id && it.directorySync }
        var editor by remember { mutableStateOf(false) }
        var editing by remember { mutableStateOf<Folder?>(null) }
        val drawer = rememberDrawerState(DrawerValue.Closed)
        val scope = rememberCoroutineScope()
        val snackbar = remember { SnackbarHostState() }
        LaunchedEffect(error) { error?.let { snackbar.showSnackbar(it); model.error.value = null } }

        ModalNavigationDrawer(drawerState = drawer, drawerContent = {
            ModalDrawerSheet(Modifier.width(280.dp)) {
                LazyColumn(Modifier.fillMaxSize().padding(horizontal = 12.dp).testTag("folder-drawer")) {
                    item("heading") {
                        Row(Modifier.padding(12.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                            BrandIcon()
                            Text("MiRelay", fontSize = 24.sp, fontWeight = FontWeight.SemiBold)
                        }
                        Text("Folders", Modifier.padding(12.dp), style = MaterialTheme.typography.labelMedium)
                    }
                    items(folders, key = { "folder:${it.id}" }) { item ->
                        NavigationDrawerItem(label = { Text(item.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                            selected = item.id == folder?.id, onClick = {
                                model.selected.value = item.id
                                scope.launch { drawer.close() }
                            }, shape = RoundedCornerShape(10.dp),
                            colors = NavigationDrawerItemDefaults.colors(selectedContainerColor = ink, selectedTextColor = Color.White))
                    }
                    item("add-folder") {
                        TextButton(onClick = { editing = null; editor = true }, enabled = !busy, modifier = Modifier.padding(vertical = 12.dp)) {
                            RelayIcon("plus", null); Spacer(Modifier.width(10.dp)); Text("Add Folder")
                        }
                    }
                }
            }
        }) {
            Scaffold(snackbarHost = { SnackbarHost(snackbar) }, topBar = {
                TopAppBar(title = { Text("MiRelay", fontSize = 19.sp, fontWeight = FontWeight.SemiBold) },
                    navigationIcon = { IconButton(onClick = { scope.launch { drawer.open() } }) { RelayIcon("menu", "Folders") } },
                    actions = {
                        if (folder != null) TextButton(enabled = !busy, onClick = { model.openAuto(folder.id) }) {
                            Text(if (autoSources.any { it.folderId == folder.id && it.enabled }) "Auto on" else "Auto")
                        }
                        if (folder != null) IconButton(enabled = !busy, onClick = { editing = folder; editor = true }) { RelayIcon("settings", "Folder settings") }
                    })
            }) { padding ->
                if (folder == null) {
                    Column(Modifier.padding(padding).fillMaxSize().testTag("welcome").verticalScroll(rememberScrollState()).padding(28.dp), verticalArrangement = Arrangement.Center) {
                        BrandWordmark()
                        Spacer(Modifier.height(20.dp))
                        Text("Your files.\nYour server.", fontSize = 34.sp, lineHeight = 40.sp, fontWeight = FontWeight.SemiBold)
                        Spacer(Modifier.height(12.dp))
                        Text("Add a Folder to send files to your Linux device through MiRelay.", color = MaterialTheme.colorScheme.onSurfaceVariant)
                        Spacer(Modifier.height(24.dp))
                        BlackButton("Add Folder", "plus", !busy) { editing = null; editor = true }
                    }
                } else {
                    val files = remember(transfers, folder.id) {
                        transfers.filter { it.folderId == folder.id }.sortedBy { if (it.status in listOf(TransferStatus.UPLOADING, TransferStatus.QUEUED)) 0 else 1 }
                    }
                    // Header, transfers and receipt note share one viewport. Fixed siblings
                    // must not consume the list's height in landscape, split-screen or large text.
                    BoxWithConstraints(Modifier.padding(padding).fillMaxSize()) {
                        val compact = maxHeight < 400.dp && maxWidth >= 480.dp
                        key(folder.id) {
                            LazyColumn(Modifier.fillMaxSize().testTag("folder-content"), contentPadding = PaddingValues(horizontal = 24.dp, vertical = 12.dp)) {
                                item("heading") {
                                    if (compact) Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(20.dp)) {
                                        FolderHeading(folder, Modifier.weight(1f), compact = true)
                                        BlackButton(if (busy) "Preparing…" else if (directoryMode) "Sync settings" else "Choose files", if(directoryMode) "folder" else "upload", !busy) { if(directoryMode) model.openAuto(folder.id) else chooseFiles() }
                                    } else Column {
                                        FolderHeading(folder)
                                        Spacer(Modifier.height(20.dp))
                                        BlackButton(if (busy) "Preparing…" else if (directoryMode) "Sync settings" else "Choose files", if(directoryMode) "folder" else "upload", !busy) { if(directoryMode) model.openAuto(folder.id) else chooseFiles() }
                                    }
                                    if (folder.pairingState != "legacy") {
                                        Spacer(Modifier.height(12.dp))
                                        Text(if (folder.pairingState == "ready") "Paired · ready to send" else "Awaiting pairing · sending is blocked",
                                            style = MaterialTheme.typography.bodySmall)
                                        folder.verification?.let { Text("Verification: $it", fontWeight = FontWeight.Medium) }
                                        if (folder.pairingState != "ready") Text("Compare this code on Linux and confirm there, then check pairing here.", style = MaterialTheme.typography.bodySmall)
                                        TextButton(enabled = !busy, onClick = { model.checkPairing(folder.id) }) { Text("Check pairing") }
                                    }
                                }
                                item("transfers-heading") {
                                    Text("Transfers", Modifier.padding(top = 20.dp, bottom = 12.dp), fontWeight = FontWeight.Medium)
                                    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                                }
                                if (files.isEmpty()) item("empty") {
                                    Column(Modifier.fillMaxWidth().padding(vertical = 24.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                                        Text("Nothing here yet", fontWeight = FontWeight.Medium)
                                        Spacer(Modifier.height(6.dp))
                                        Text(if(directoryMode) "Up to date, or waiting for the next directory check." else "Choose files, or share them from another app.", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                    }
                                }
                                items(files, key = { "transfer:${it.id}" }) { transfer ->
                                    TransferRow(transfer, busy, { model.retry(transfer.id) }, { model.pause(transfer.id) })
                                }
                                item("receipt-note") {
                                    Text(if(directoryMode) "Waiting for Linux means uploaded, not yet received. Receipts refresh with directory checks; conflicts keep both copies on Linux." else "Uploads are confirmed by your server. Linux delivery receipts are not available for delivery-only files.",
                                        Modifier.padding(vertical = 16.dp), style = MaterialTheme.typography.labelSmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant)
                                    if(directoryMode) TextButton(enabled = !busy, onClick = { model.refreshReceipts(folder.id) }) { Text("Refresh receipts") }
                                }
                            }
                        }
                    }
                }
            }
        }
        if (editor) FolderEditor(editing, busy, error, creationTree,
            chooseDirectory = { chooseDirectory("new-folder") },
            onDismiss = { if (!busy) { editor = false; model.creationTree.value = null } }) { name, url, token, insecure, code, history ->
            model.saveFolder(editing, name, url, token, insecure, code, history) { editor = false }
        }
        val autoFolder = folders.find { it.id == autoEditor }
        if (autoFolder != null && !editor) AutoEditor(autoFolder, autoSources.find { it.folderId == autoFolder.id }, autoTree,
            busy, error, directoryPreview, preview = { model.previewDirectory(autoFolder.id) }, confirm = { model.confirmDirectory(autoFolder.id,it) }, resume = { model.resumeDirectory(autoFolder.id,it) },
            choose = { chooseDirectory(autoFolder.id) }, close = { model.autoEditor.value = null },
            enable = { unmetered, history -> requestNotifications(); model.enableAuto(autoFolder.id, unmetered, history) },
            pause = { model.pauseAuto(autoFolder.id) }, check = { model.checkAuto(autoFolder.id) })
        if (pending.isNotEmpty() && folder != null && !editor && autoFolder == null) {
            var destination by remember(pending) { mutableStateOf(folder.id) }
            var destinationsOpen by remember { mutableStateOf(false) }
            val target = folders.find { it.id == destination } ?: folder
            AlertDialog(onDismissRequest = { if (!busy) model.pending.value = emptyList() },
                title = { Text("Send ${pending.size} file(s)") },
                text = { Column(Modifier.verticalScroll(rememberScrollState())) {
                    Box {
                        TextButton(onClick = { destinationsOpen = true }) { RelayIcon("folder", null); Spacer(Modifier.width(8.dp)); Text(target.name) }
                        DropdownMenu(destinationsOpen, { destinationsOpen = false }) {
                            folders.forEach { item -> DropdownMenuItem(text = { Text(item.name) }, onClick = { destination = item.id; destinationsOpen = false }) }
                        }
                    }
                    Spacer(Modifier.height(8.dp))
                    Text("Files will be copied into MiRelay's private storage, then uploaded to ${target.server}. Originals stay unchanged.")
                } },
                confirmButton = { TextButton(enabled = !busy, onClick = { requestNotifications(); model.selected.value = target.id; model.send(target.id) }) { Text("Send") } },
                dismissButton = { Row {
                    TextButton(enabled = !busy, onClick = { model.pending.value = emptyList() }) { Text("Cancel") }
                } })
        }
    }
}

@Composable private fun BrandIcon() {
    Image(painterResource(R.drawable.mirelay_brand_icon), contentDescription = null,
        modifier = Modifier.size(40.dp).clip(RoundedCornerShape(8.dp)).background(Color.White),
        contentScale = ContentScale.Fit)
}

@Composable private fun BrandWordmark() {
    // Do not crop or tint the supplied opaque artwork, even in dark mode.
    Image(painterResource(R.drawable.mirelay_wordmark), contentDescription = stringResource(R.string.mirelay_brand_description),
        modifier = Modifier.height(160.dp).fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(Color.White).testTag("brand-wordmark"),
        contentScale = ContentScale.Fit)
}

@Composable private fun AutoEditor(folder: Folder, source: AutoSource?, tree: android.net.Uri?, busy: Boolean, failure: String?,
    directoryPreview: DirectoryPreview?, preview: () -> Unit, confirm: (Boolean) -> Unit, resume: (Boolean) -> Unit,
    choose: () -> Unit, close: () -> Unit, enable: (Boolean, Boolean) -> Unit, pause: () -> Unit, check: () -> Unit) {
    var directoryMode by rememberSaveable(folder.id, source?.directorySync) { mutableStateOf(source?.directorySync == true || (folder.pairingState != "legacy" && (source == null || source.prepared))) }
    if(directoryMode) {
        DirectoryEditor(folder,source,tree,busy,failure,directoryPreview,choose,close,preview,confirm,resume,pause,check) { directoryMode=false }
        return
    }
    var unmetered by rememberSaveable(folder.id, source?.revision) { mutableStateOf(source?.unmetered ?: true) }
    var includeExisting by rememberSaveable(folder.id, tree) { mutableStateOf(false) }
    val enabled = source?.enabled == true
    AlertDialog(onDismissRequest = { if (!busy) close() }, title = { Text("Automatic sending") },
        text = { Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(folder.name, fontWeight = FontWeight.Medium)
            if(folder.pairingState != "legacy" && !enabled) TextButton(enabled = !busy, onClick = { directoryMode=true }) { Text("Set up directory sync") }
            Text(if (enabled) "On · system-scheduled" else "Off · manual sending still works", style = MaterialTheme.typography.bodySmall)
            if (tree != null) Text(if (tree.toString() == source?.treeUri) source.name else android.provider.DocumentsContract.getTreeDocumentId(tree),
                maxLines = 3, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodyMedium)
            if (!enabled) TextButton(onClick = choose, enabled = !busy) { RelayIcon("folder", null); Spacer(Modifier.width(8.dp)); Text("Choose source directory") }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(unmetered, { unmetered = it }, enabled = !enabled && !busy)
                Text("Unmetered network only", style = MaterialTheme.typography.bodySmall)
            }
            Text("Checks about every 30 minutes; Android may delay them. Low battery or storage pauses work. No always-on service.", style = MaterialTheme.typography.bodySmall)
            if (!enabled && !(source?.prepared == true && source.treeUri == tree?.toString())) Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(includeExisting, { includeExisting = it }, enabled = !busy)
                Text("Also send existing files", style = MaterialTheme.typography.bodySmall)
            }
            Text(if (source?.prepared == true && source.treeUri == tree?.toString()) "This directory was initialized when you created the Folder. Enabling keeps that baseline and your existing-file choice."
                else "Existing files stay local unless you select Also send existing files. Only eligible files are sent; originals stay unchanged.", style = MaterialTheme.typography.bodySmall)
            Text("Re-enabling starts a fresh baseline. Pausing also pauses queued Auto uploads; manually resuming a file uses normal network settings.", style = MaterialTheme.typography.bodySmall)
            source?.lastScan?.let { Text("Last check: ${java.text.DateFormat.getDateTimeInstance(java.text.DateFormat.SHORT, java.text.DateFormat.SHORT).format(java.util.Date(it))}", style = MaterialTheme.typography.labelSmall) }
            if (source != null && (source.waiting > 0 || source.skipped > 0)) Text("${source.waiting} waiting · ${source.skipped} need attention (changed, unavailable or unsupported files)", style = MaterialTheme.typography.bodySmall)
            (source?.error ?: failure)?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
            if (enabled) TextButton(enabled = !busy, onClick = check) { Text("Check now") }
        } },
        confirmButton = {
            TextButton(enabled = !busy && (enabled || tree != null), onClick = { if (enabled) pause() else enable(unmetered, includeExisting) }) {
                Text(if (busy) "Preparing…" else if (enabled) "Pause Auto" else "Enable Auto")
            }
        }, dismissButton = { TextButton(enabled = !busy, onClick = close) { Text("Close") } })
}

@Composable private fun DirectoryEditor(folder: Folder, source: AutoSource?, tree: android.net.Uri?, busy: Boolean, failure: String?,
    preview: DirectoryPreview?, choose: () -> Unit, close: () -> Unit, compare: () -> Unit, confirm: (Boolean) -> Unit,
    resume: (Boolean) -> Unit, pause: () -> Unit, check: () -> Unit, deliveryMode: () -> Unit) {
    val initialized=source?.directorySync == true
    val enabled=source?.enabled == true
    val currentPreview=preview?.takeIf { it.tree == tree?.toString() }
    var unmetered by rememberSaveable(folder.id,source?.revision) {mutableStateOf(source?.unmetered ?: true)}
    AlertDialog(onDismissRequest={if(!busy) close()}, title={Text("Directory sync")}, containerColor=MaterialTheme.colorScheme.surface,
        text={Column(Modifier.verticalScroll(rememberScrollState()).testTag("directory-editor"),verticalArrangement=Arrangement.spacedBy(12.dp)) {
            Text(folder.name,fontWeight=FontWeight.Medium)
            Text("Android → Linux",style=MaterialTheme.typography.labelMedium)
            if(currentPreview==null || initialized) Text("Keep filenames and subfolders. Sync new and modified files. Deletions do not propagate; conflicts keep both copies on Linux.",style=MaterialTheme.typography.bodySmall)
            if(initialized || currentPreview==null) Text(if(initialized) if(enabled) "On · system-scheduled" else "Paused · version history kept" else "Preview both directories before sending existing files.",style=MaterialTheme.typography.bodySmall)
            tree?.let { Text(currentPreview?.name ?: source?.takeIf { it.treeUri==tree.toString() }?.name ?: android.provider.DocumentsContract.getTreeDocumentId(it),maxLines=3,overflow=TextOverflow.Ellipsis) }
            if(currentPreview!=null && !initialized) {
                HorizontalDivider()
                Text("${currentPreview.identical} identical · ${currentPreview.missing.size} missing · ${currentPreview.different.size} different",fontWeight=FontWeight.Medium)
                Text("${currentPreview.destinationOnly} Linux-only files will be kept.",style=MaterialTheme.typography.bodySmall)
                (currentPreview.missing.map {"Add · $it"}+currentPreview.different.map {"Update · $it"}).take(50).forEach {Text(it,style=MaterialTheme.typography.bodySmall)}
                if(currentPreview.missing.size+currentPreview.different.size>50) Text("Showing the first 50 changed paths.",style=MaterialTheme.typography.labelSmall)
                Text("Initialize sync allows these files and future changes to upload automatically. Linux keeps replaced copies. A changed preview must be reviewed again.",style=MaterialTheme.typography.bodySmall)
                TextButton(enabled=!busy,onClick=compare) {Text("Refresh preview")}
            }
            if(!enabled) TextButton(enabled=!busy,onClick=choose) {RelayIcon("folder",null);Spacer(Modifier.width(8.dp));Text(if(initialized) "Restore source access" else "Choose source directory")}
            Row(verticalAlignment=Alignment.CenterVertically) {Checkbox(unmetered,{unmetered=it},enabled=!busy && !enabled);Text("Unmetered network only",style=MaterialTheme.typography.bodySmall)}
            Text("Checks about every 30 minutes; Android may delay them. Low battery or storage pauses work. No always-on service.",style=MaterialTheme.typography.bodySmall)
            if(!initialized && currentPreview==null) Text("On Linux, add a Folder, enable Directory sync, then review and initialize the destination directory. Complete pairing before previewing here.",style=MaterialTheme.typography.bodySmall)
            source?.lastScan?.let {Text("Last check: ${java.text.DateFormat.getDateTimeInstance(java.text.DateFormat.SHORT,java.text.DateFormat.SHORT).format(java.util.Date(it))}",style=MaterialTheme.typography.labelSmall)}
            if(initialized) Text("${source.waiting} waiting · ${source.skipped} need attention",style=MaterialTheme.typography.bodySmall)
            (failure ?: source?.error)?.let {Text(it,color=MaterialTheme.colorScheme.error,style=MaterialTheme.typography.bodySmall)}
            if(enabled) TextButton(enabled=!busy,onClick=check) {Text("Check now")}
            if(!initialized && !enabled) TextButton(enabled=!busy,onClick=deliveryMode) {Text("Use delivery Auto")}
        }},
        confirmButton={TextButton(enabled=!busy && tree!=null && (initialized || !enabled),onClick={
            if(initialized) {if(enabled) pause() else resume(unmetered)} else if(currentPreview==null) compare() else confirm(unmetered)
        }) {Text(if(busy) "Preparing…" else if(initialized) if(enabled) "Pause sync" else "Resume sync" else if(currentPreview==null) "Preview changes" else "Initialize sync")}},
        dismissButton={TextButton(enabled=!busy,onClick=close) {Text("Close")}})
}

@Composable private fun FolderHeading(folder: Folder, modifier: Modifier = Modifier, compact: Boolean = false) {
    Column(modifier) {
        Text("FOLDER", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Spacer(Modifier.height(6.dp))
        Text(folder.name, fontSize = if (compact) 22.sp else 28.sp, fontWeight = FontWeight.SemiBold, maxLines = if (compact) 1 else 2, overflow = TextOverflow.Ellipsis)
        Spacer(Modifier.height(4.dp))
        Text(folder.server, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

@Composable private fun BlackButton(text: String, icon: String, enabled: Boolean, onClick: () -> Unit) {
    Button(onClick = onClick, enabled = enabled, shape = RoundedCornerShape(10.dp), contentPadding = PaddingValues(horizontal = 18.dp, vertical = 12.dp),
        border = if (isSystemInDarkTheme()) BorderStroke(1.dp, Color(0xFF555555)) else null,
        colors = ButtonDefaults.buttonColors(containerColor = ink, contentColor = Color.White), modifier = Modifier.heightIn(min = 48.dp)) {
        RelayIcon(icon, null, Modifier.size(18.dp)); Spacer(Modifier.width(10.dp)); Text(text)
    }
}

@Composable private fun TransferRow(file: Transfer, busy: Boolean, retry: () -> Unit, pause: () -> Unit) {
    val active = file.status in listOf(TransferStatus.QUEUED, TransferStatus.UPLOADING)
    Column(Modifier.padding(vertical = 14.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            RelayIcon("file", null, Modifier.size(20.dp)); Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                Text(file.relativePath ?: file.name, maxLines = 2, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.Medium)
                Spacer(Modifier.height(4.dp))
                val label = when (file.status) {
                    TransferStatus.QUEUED -> if (file.error == null) "Queued" else "Waiting to retry"
                    TransferStatus.UPLOADING -> "Uploading · ${((file.uploaded.toDouble() / file.size) * 100).toInt().coerceIn(0, 100)}%"
                    TransferStatus.PAUSED -> "Paused"
                    TransferStatus.FAILED -> "Needs attention"
                    TransferStatus.UPLOADED -> if(file.relativePath==null) "Uploaded" else if(file.superseded) "Superseded" else if(file.received) if(file.conflict) "Synced · conflict copy kept" else "Synced" else "Waiting for Linux"
                }
                Text("${fileSize(file.size)} · $label", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            if (active) IconButton(onClick = pause, enabled = !busy) { RelayIcon("pause", "Pause transfer") }
            else if (file.status != TransferStatus.UPLOADED) IconButton(onClick = retry, enabled = !busy) { RelayIcon("play", "Resume or retry transfer") }
        }
        if (file.status == TransferStatus.UPLOADING) LinearProgressIndicator(
            progress = { (file.uploaded.toFloat() / file.size).coerceIn(0f, 1f) },
            modifier = Modifier.padding(start = 34.dp, top = 10.dp).fillMaxWidth().height(2.dp))
        file.error?.let { Text(it, Modifier.padding(start = 34.dp, top = 8.dp), color = MaterialTheme.colorScheme.error,
            style = MaterialTheme.typography.bodySmall, maxLines = 5, overflow = TextOverflow.Ellipsis) }
    }
}

@Composable private fun FolderEditor(folder: Folder?, busy: Boolean, failure: String?, tree: android.net.Uri?, chooseDirectory: () -> Unit,
    onDismiss: () -> Unit, save: (String, String, String, Boolean, String, Boolean) -> Unit) {
    var name by rememberSaveable(folder?.id) { mutableStateOf(folder?.name.orEmpty()) }
    var server by rememberSaveable(folder?.id) { mutableStateOf(folder?.server.orEmpty()) }
    // Credentials are intentionally not saved in Activity instance state.
    var token by remember(folder?.id) { mutableStateOf("") }
    var insecure by rememberSaveable(folder?.id) { mutableStateOf(folder?.insecure ?: false) }
    var usePairing by rememberSaveable(folder?.id) { mutableStateOf(folder == null) }
    var pairingCode by remember(folder?.id) { mutableStateOf("") }
    var includeExisting by rememberSaveable(tree) { mutableStateOf(false) }
    AlertDialog(onDismissRequest = onDismiss, title = { Text(if (folder == null) "Add Folder" else "Folder settings") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                OutlinedTextField(name, { name = it }, label = { Text("Name") }, singleLine = true, enabled = !busy)
                OutlinedTextField(server, { server = it }, label = { Text("Server URL") }, placeholder = { Text("https://relay.example.com") },
                    singleLine = true, enabled = !busy && folder == null, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri))
                if (folder == null) {
                    TextButton(enabled = !busy, onClick = chooseDirectory) { RelayIcon("folder", null); Spacer(Modifier.width(8.dp)); Text("Choose existing directory") }
                    tree?.let {
                        Text(android.provider.DocumentsContract.getTreeDocumentId(it), maxLines = 3, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodySmall)
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Checkbox(includeExisting, { includeExisting = it }, enabled = !busy, modifier = Modifier.testTag("include-history"))
                            Text("Also send existing files", style = MaterialTheme.typography.bodySmall)
                        }
                        Text("Initializes this directory in place. Originals are never moved or deleted. Auto stays off until you enable it after pairing.", style = MaterialTheme.typography.bodySmall)
                    }
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Checkbox(usePairing, { usePairing = it }, enabled = !busy, modifier = Modifier.testTag("pair-folder"))
                        Text("Pair with Linux", style = MaterialTheme.typography.bodySmall)
                    }
                }
                if (usePairing && folder == null) OutlinedTextField(pairingCode, { pairingCode = it }, label = { Text("Pairing code") },
                    singleLine = true, enabled = !busy, visualTransformation = PasswordVisualTransformation(), keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password))
                else OutlinedTextField(token, { token = it }, label = { Text(if (folder == null) "Bearer token" else "New token (optional)") },
                    singleLine = true, enabled = !busy, visualTransformation = PasswordVisualTransformation(), keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password))
                if (BuildConfig.DEBUG) Row(verticalAlignment = Alignment.CenterVertically) {
                    Checkbox(insecure, { insecure = it }, enabled = !busy, modifier = Modifier.testTag("setup-http"))
                    Text("Allow trusted-network HTTP", style = MaterialTheme.typography.bodySmall)
                }
                Text(if (usePairing && folder == null) "Use the original server URL and the temporary invitation from Linux. Never paste the administrator credential here. Pairing is verified before sending; credentials are encrypted on this device."
                    else "Legacy connection or existing sender credential. The connection is checked before saving. Tokens are encrypted on this device; Auto requires explicit consent.", style = MaterialTheme.typography.bodySmall)
                failure?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
            }
        }, confirmButton = { TextButton(enabled = !busy && name.isNotBlank() && server.isNotBlank() && (folder != null || if (usePairing) pairingCode.isNotBlank() else token.isNotBlank()),
            onClick = { save(name, server, token, insecure, if (usePairing && folder == null) pairingCode else "", includeExisting) }) { Text(if (busy) "Checking…" else "Save") } },
        dismissButton = { TextButton(enabled = !busy, onClick = onDismiss) { Text("Cancel") } })
}

private fun fileSize(bytes: Long): String = when {
    bytes >= 1024 * 1024 -> String.format(Locale.ROOT, "%.1f MiB", bytes / (1024.0 * 1024))
    bytes >= 1024 -> String.format(Locale.ROOT, "%.1f KiB", bytes / 1024.0)
    else -> "$bytes B"
}

@Composable private fun RelayIcon(name: String, description: String?, modifier: Modifier = Modifier) {
    val vector = remember(name) {
        ImageVector.Builder(name, 24.dp, 24.dp, 24f, 24f).apply {
            path(fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 1.7f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round) {
                when (name) {
                    "menu" -> { moveTo(4f, 7f); lineTo(20f, 7f); moveTo(4f, 12f); lineTo(17f, 12f); moveTo(4f, 17f); lineTo(20f, 17f) }
                    "plus" -> { moveTo(12f, 5f); lineTo(12f, 19f); moveTo(5f, 12f); lineTo(19f, 12f) }
                    "folder" -> { moveTo(3f, 7f); lineTo(9f, 7f); lineTo(11f, 9f); lineTo(21f, 9f); lineTo(21f, 19f); lineTo(3f, 19f); close(); moveTo(3f, 7f); lineTo(3f, 5f); lineTo(10f, 5f); lineTo(12f, 7f); lineTo(20f, 7f) }
                    "upload" -> { moveTo(12f, 16f); lineTo(12f, 4f); moveTo(7f, 9f); lineTo(12f, 4f); lineTo(17f, 9f); moveTo(4f, 16f); lineTo(4f, 20f); lineTo(20f, 20f); lineTo(20f, 16f) }
                    "pause" -> { moveTo(8f, 5f); lineTo(8f, 19f); moveTo(16f, 5f); lineTo(16f, 19f) }
                    "play" -> { moveTo(8f, 5f); lineTo(19f, 12f); lineTo(8f, 19f); close() }
                    "settings" -> { moveTo(4f, 7f); lineTo(10f, 7f); moveTo(14f, 7f); lineTo(20f, 7f); moveTo(12f, 4f); lineTo(12f, 10f); moveTo(4f, 17f); lineTo(7f, 17f); moveTo(11f, 17f); lineTo(20f, 17f); moveTo(9f, 14f); lineTo(9f, 20f) }
                    else -> { moveTo(6f, 3f); lineTo(14f, 3f); lineTo(19f, 8f); lineTo(19f, 21f); lineTo(6f, 21f); close(); moveTo(14f, 3f); lineTo(14f, 8f); lineTo(19f, 8f); moveTo(9f, 13f); lineTo(16f, 13f); moveTo(9f, 17f); lineTo(14f, 17f) }
                }
            }
        }.build()
    }
    Icon(vector, description, modifier)
}

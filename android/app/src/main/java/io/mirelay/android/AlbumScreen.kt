package io.mirelay.android

import android.graphics.Bitmap
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.rememberTransformableState
import androidx.compose.foundation.gestures.transformable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.repeatOnLifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import java.text.DateFormat
import java.util.Date
import kotlin.math.min

private data class AlbumState(val photos: List<AlbumPhoto> = emptyList(), val loading: Boolean = true, val error: String? = null)

@OptIn(ExperimentalMaterial3Api::class)
@Composable internal fun AlbumScreen(
    folder: Folder, source: AutoSource?, files: List<Transfer>, busy: Boolean,
    modifier: Modifier = Modifier, folders: () -> Unit, settings: () -> Unit,
    syncSettings: () -> Unit, general: () -> Unit, chooseFiles: () -> Unit,
    refreshReceipts: () -> Unit, checkPairing: () -> Unit,
    retry: (String) -> Unit, pause: (String) -> Unit,
) {
    val app = LocalContext.current.applicationContext as RelayApplication
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    var refresh by remember { mutableIntStateOf(0) }
    var options by remember { mutableStateOf(false) }
    var activity by rememberSaveable { mutableStateOf(false) }
    var selected by rememberSaveable { mutableStateOf<String?>(null) }
    val prefs = remember(app) { app.getSharedPreferences("appearance", 0) }
    var reduced by rememberSaveable { mutableStateOf(prefs.getBoolean("reduce_transparency", false)) }
    val state by produceState(AlbumState(), folder.id, source?.treeUri, refresh, lifecycle) {
        lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            value = AlbumState()
            value = if (source == null) AlbumState(loading = false) else try {
                AlbumState(app.album.scan(source.treeUri), loading = false)
            } catch (error: CancellationException) { throw error }
            catch (_: SecurityException) { AlbumState(loading = false, error = "Folder access is unavailable. Restore access in Sync settings.") }
            catch (_: Exception) { AlbumState(loading = false, error = "Could not read this Folder completely. Check access, then refresh. Sources are limited to 5,000 entries and 16 nested folders.") }
        }
    }
    val photos = if (source == null) remember(files) { AlbumPhotos.transfers(files) } else state.photos
    val activityByPath = remember(files) { files.filter { !it.superseded && it.relativePath != null }
        .sortedBy { it.sourceVersion ?: 0 }.associateBy { it.relativePath } }
    val active = files.count { it.status in listOf(TransferStatus.QUEUED, TransferStatus.UPLOADING) }
    val failed = files.count { it.status == TransferStatus.FAILED } + (source?.attentionCount ?: 0)
    val summary = when {
        failed > 0 -> "$failed need attention"
        active > 0 -> "$active transferring"
        folder.connectionClosed -> folder.connectionLabel
        folder.pairingState !in listOf("legacy", "ready") -> "Pairing needed"
        source?.enabled == true -> "Auto on"
        source != null -> "Sync paused"
        else -> "Transfer activity"
    }
    FrostedHeader(modifier.fillMaxSize(), MaterialTheme.colorScheme.surface, reduced,
        header = {
            TopAppBar(modifier = Modifier.testTag("brand-top-bar"), expandedHeight = 72.dp,
                colors = TopAppBarDefaults.topAppBarColors(containerColor = Color.Transparent),
                title = { BrandWordmark(Modifier.width(104.dp).height(65.dp).testTag("brand-wordmark")) },
                actions = {
                    IconButton(onClick = { refresh++ }, enabled = !state.loading) { RelayIcon("refresh", "Refresh photos") }
                    Box {
                        IconButton(onClick = { options = true }) { RelayIcon("more", "Album options") }
                        DropdownMenu(options, { options = false }) {
                            DropdownMenuItem(text = { Text("Sync settings") }, enabled = !busy && !folder.connectionClosed, onClick = { options = false; syncSettings() })
                            DropdownMenuItem(text = { Text("Folder settings") }, enabled = !busy, onClick = { options = false; settings() })
                            DropdownMenuItem(text = { Text("Transfer activity") }, onClick = { options = false; activity = true })
                            if (source?.directorySync != true) DropdownMenuItem(text = { Text("Choose files") }, enabled = !busy && !folder.connectionClosed, onClick = { options = false; chooseFiles() })
                            DropdownMenuItem(text = { Text("General view") }, enabled = !busy, onClick = { options = false; general() })
                            DropdownMenuItem(text = { Text("Reduce transparency") }, trailingIcon = { Checkbox(reduced, null) }, onClick = {
                                reduced = !reduced; prefs.edit().putBoolean("reduce_transparency", reduced).apply()
                            })
                        }
                    }
                    IconButton(onClick = folders) { RelayIcon("menu", "Folders") }
                })
            Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 8.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(folder.name, Modifier.weight(1f).testTag("album-title"), style = MaterialTheme.typography.titleLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                TextButton(onClick = { activity = true }, modifier = Modifier.testTag("album-status")) {
                    Text(summary, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.labelMedium)
                }
            }
        }) { top ->
        BoxWithConstraints(Modifier.fillMaxSize()) {
            val columns = (maxWidth / 120.dp).toInt().coerceIn(3, 6)
            LazyVerticalGrid(GridCells.Fixed(columns), Modifier.fillMaxSize().testTag("album-grid"),
                contentPadding = PaddingValues(top = top + 2.dp, bottom = 16.dp),
                horizontalArrangement = Arrangement.spacedBy(2.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                if (state.loading || state.error != null || photos.isEmpty()) item(span = { GridItemSpan(maxLineSpan) }) {
                    Column(Modifier.fillMaxWidth().padding(32.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                        when {
                            state.loading -> { CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 2.dp); Text("Reading photos…", Modifier.padding(top = 16.dp)) }
                            state.error != null -> { Text(state.error!!); TextButton(onClick = { refresh++ }) { Text("Try again") } }
                            source != null -> Text("No photos in this Folder yet")
                            else -> { Text("Your photos, in one place"); Text("Configure this Folder's source in Sync settings to browse its existing images. Browsing an already configured source never starts an upload.", Modifier.padding(top = 12.dp)); TextButton(onClick = syncSettings, enabled = !busy && !folder.connectionClosed) { Text("Sync settings") } }
                        }
                    }
                }
                items(photos, key = { it.key }, contentType = { "photo" }) { photo ->
                    Box(Modifier.fillMaxWidth().aspectRatio(1f).testTag("photo-card-${photo.key}")
                        .clickable(onClickLabel = "Open ${photo.name}") { selected = photo.key }) {
                        AlbumImage(photo, 256, Modifier.fillMaxSize(), crop = true)
                        val transfer = photo.transfer ?: activityByPath[photo.path]
                        if (transfer != null && transfer.status != TransferStatus.UPLOADED) Box(Modifier.align(Alignment.BottomEnd).padding(4.dp)
                            .background(Color.Black.copy(alpha = 0.72f), androidx.compose.foundation.shape.CircleShape).padding(5.dp)) {
                            CompositionLocalProvider(LocalContentColor provides Color.White) {
                                if (transfer.status in listOf(TransferStatus.UPLOADING, TransferStatus.QUEUED)) CircularProgressIndicator(Modifier.size(14.dp).semantics { contentDescription = "Transferring" }, strokeWidth = 1.5.dp, color = Color.White)
                                else RelayIcon(if (transfer.status == TransferStatus.FAILED) "warning" else "pause", if (transfer.status == TransferStatus.FAILED) "Transfer failed" else "Transfer paused", Modifier.size(14.dp))
                            }
                        }
                    }
                }
            }
        }
    }
    val selectedIndex = photos.indexOfFirst { it.key == selected }
    if (selectedIndex >= 0) AlbumViewer(photos, selectedIndex, onClose = { selected = null })
    if (activity) AlertDialog(onDismissRequest = { activity = false }, title = { Text("Transfer activity") },
        text = {
            LazyColumn(Modifier.fillMaxWidth().heightIn(max = 480.dp).testTag("album-activity")) {
                item {
                    Text(summary)
                    source?.error?.let { Text("Source needs attention. Open Sync settings to review.", Modifier.padding(top = 8.dp)) }
                    if (folder.pairingState != "legacy") {
                        Text(folder.connectionLabel, Modifier.padding(top = 8.dp))
                        folder.verification?.let { Text("Verification: $it") }
                        TextButton(onClick = checkPairing, enabled = !busy && !folder.connectionClosed) { Text("Check pairing") }
                    }
                    TextButton(onClick = { activity = false; syncSettings() }, enabled = !busy && !folder.connectionClosed) { Text("Sync settings") }
                    if (source?.directorySync == true) TextButton(onClick = refreshReceipts, enabled = !busy && !folder.connectionClosed) { Text("Refresh receipts") }
                    if (files.isEmpty()) Text("No transfers yet. Browsing this Folder does not send files.")
                }
                items(files.sortedBy { if (it.status in listOf(TransferStatus.QUEUED, TransferStatus.UPLOADING, TransferStatus.FAILED)) 0 else 1 }, key = { it.id }) {
                    TransferRow(it, busy || folder.connectionClosed, { retry(it.id) }, { pause(it.id) })
                }
            }
        }, confirmButton = { TextButton(onClick = { activity = false }) { Text("Close") } })
}

@Composable private fun albumBitmap(photo: AlbumPhoto, target: Int): State<Bitmap?> {
    val app = LocalContext.current.applicationContext as RelayApplication
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    return produceState<Bitmap?>(null, photo.key, photo.tree, photo.generation, photo.transfer?.status, target, lifecycle) {
        value = null
        lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            value = try { app.album.load(photo, app.store, target) }
            catch (error: CancellationException) { throw error }
            catch (_: Exception) { null }
        }
    }
}

@Composable private fun AlbumImage(photo: AlbumPhoto, target: Int, modifier: Modifier, crop: Boolean) {
    val bitmap by albumBitmap(photo, target)
    Box(modifier.background(MaterialTheme.colorScheme.surfaceContainerHighest), contentAlignment = Alignment.Center) {
        bitmap?.let { Image(it.asImageBitmap(), photo.name, Modifier.fillMaxSize().testTag("photo-image-${photo.key}"), contentScale = if (crop) ContentScale.Crop else ContentScale.Fit) }
            ?: RelayIcon("photos", "Preview unavailable: ${photo.name}", Modifier.size(24.dp))
    }
}

internal fun albumPan(offset: Offset, scale: Float, viewport: IntSize, image: IntSize): Offset {
    if (image.width <= 0 || image.height <= 0 || viewport.width <= 0 || viewport.height <= 0 || !scale.isFinite()) return Offset.Zero
    val fit = min(viewport.width.toFloat() / image.width, viewport.height.toFloat() / image.height)
    val x = ((image.width * fit * scale - viewport.width) / 2).coerceAtLeast(0f)
    val y = ((image.height * fit * scale - viewport.height) / 2).coerceAtLeast(0f)
    return Offset(offset.x.takeIf { it.isFinite() }?.coerceIn(-x, x) ?: 0f, offset.y.takeIf { it.isFinite() }?.coerceIn(-y, y) ?: 0f)
}

@Composable private fun AlbumViewer(photos: List<AlbumPhoto>, initial: Int, onClose: () -> Unit) {
    // Keep page identity stable while sync status changes underneath the viewer.
    val pages = remember { photos.toList() }
    val pager = rememberPagerState(initialPage = initial, pageCount = { pages.size })
    val scope = rememberCoroutineScope()
    var controls by rememberSaveable { mutableStateOf(true) }
    var details by rememberSaveable { mutableStateOf(false) }
    var zoomed by remember { mutableStateOf(false) }
    Dialog(onDismissRequest = onClose, properties = DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false)) {
        Box(Modifier.fillMaxSize().background(Color.Black).testTag("album-viewer")) {
            HorizontalPager(pager, Modifier.fillMaxSize().testTag("album-pager"), userScrollEnabled = !zoomed, key = { pages[it].key }) { index ->
                ZoomPhoto(pages[index], index == pager.currentPage, toggle = { controls = !controls }, zoomChanged = { if (index == pager.currentPage) zoomed = it })
            }
            if (controls) CompositionLocalProvider(LocalContentColor provides Color.White) {
                Row(Modifier.align(Alignment.TopCenter).fillMaxWidth().background(Color.Black.copy(alpha = 0.6f)).statusBarsPadding().padding(horizontal = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                    IconButton(onClick = onClose) { RelayIcon("back", "Close photo") }
                    Text("${pager.currentPage + 1} / ${pages.size}", Modifier.weight(1f), color = Color.White)
                    IconButton(onClick = { details = true }) { RelayIcon("info", "Photo details") }
                }
                Row(Modifier.align(Alignment.BottomCenter).fillMaxWidth().background(Color.Black.copy(alpha = 0.6f)).navigationBarsPadding().padding(horizontal = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                    IconButton(enabled = pager.currentPage > 0 && !pager.isScrollInProgress, onClick = { val target = (pager.currentPage - 1).coerceAtLeast(0); zoomed = false; scope.launch { pager.animateScrollToPage(target) } }) { RelayIcon("back", "Previous photo") }
                    Text(pages[pager.currentPage].name, Modifier.weight(1f), color = Color.White, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodyMedium)
                    IconButton(enabled = pager.currentPage < pages.lastIndex && !pager.isScrollInProgress, onClick = { val target = (pager.currentPage + 1).coerceAtMost(pages.lastIndex); zoomed = false; scope.launch { pager.animateScrollToPage(target) } }) { RelayIcon("next", "Next photo") }
                }
            }
        }
        if (details) AlertDialog(onDismissRequest = { details = false }, title = { Text(pages[pager.currentPage].name) }, text = {
            val photo = pages[pager.currentPage]
            Column {
                Text(photo.path)
                photo.size?.let { Text("$it bytes", Modifier.padding(top = 12.dp)) }
                photo.modified?.takeIf { it > 0 }?.let { Text("File date: ${DateFormat.getDateTimeInstance().format(Date(it))}") }
                Text("Optimized preview · original file unchanged", Modifier.padding(top = 12.dp))
                if (photo.source != null) Text("Previews support readable images up to 32 MiB. Unsupported images remain in your Folder.", Modifier.padding(top = 8.dp))
            }
        }, confirmButton = { TextButton(onClick = { details = false }) { Text("Close") } })
    }
}

@Composable private fun ZoomPhoto(photo: AlbumPhoto, current: Boolean, toggle: () -> Unit, zoomChanged: (Boolean) -> Unit) {
    val bitmap by albumBitmap(photo, 1024)
    var scale by remember(photo.key) { mutableFloatStateOf(1f) }
    var offset by remember(photo.key) { mutableStateOf(Offset.Zero) }
    var viewport by remember { mutableStateOf(IntSize.Zero) }
    val image = bitmap?.let { IntSize(it.width, it.height) } ?: IntSize.Zero
    LaunchedEffect(current) { if (!current) { scale = 1f; offset = Offset.Zero } }
    LaunchedEffect(scale, current) { if (current) zoomChanged(scale > 1.01f) }
    val transform = rememberTransformableState { zoom, pan, _ ->
        val next = (scale * zoom).takeIf { it.isFinite() }?.coerceIn(1f, 4f) ?: 1f
        offset = albumPan(offset + pan, next, viewport, image)
        scale = next
    }
    Box(Modifier.fillMaxSize().clipToBounds().onSizeChanged { viewport = it; offset = albumPan(offset, scale, it, image) }
        .testTag("album-zoom-${photo.key}").semantics { stateDescription = if (scale > 1.01f) "Zoomed" else "Fit" }
        .pointerInput(photo.key) { detectTapGestures(onTap = { toggle() }, onDoubleTap = { scale = if (scale > 1.01f) 1f else 2.5f; offset = Offset.Zero }) }
        .transformable(transform, canPan = { scale > 1.01f }, enabled = bitmap != null), contentAlignment = Alignment.Center) {
        bitmap?.let { Image(it.asImageBitmap(), photo.name, Modifier.fillMaxSize().graphicsLayer { scaleX = scale; scaleY = scale; translationX = offset.x; translationY = offset.y }.testTag("photo-image-${photo.key}"), contentScale = ContentScale.Fit) }
            ?: Text("Preview unavailable", color = Color.White)
    }
}

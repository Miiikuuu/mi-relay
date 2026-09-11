package io.mirelay.android

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.util.LruCache
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withContext
import java.io.File

/** Private previews or immutable staging only: no network, SAF reads or writes
 * inside the user's synchronized directory. At most two UI decodes, 8 MiB RAM. */
internal object PhotoThumbnails {
    private val permits = Semaphore(2)
    private val cache = object : LruCache<String, Bitmap>(8 * 1024 * 1024) {
        override fun sizeOf(key: String, value: Bitmap) = value.byteCount
        // Do not recycle evicted bitmaps: a visible composable may still own one.
    }
    internal fun sampleSize(width: Int, height: Int, target: Int): Int? {
        if (width <= 0 || height <= 0 || width.toLong() * height > 100_000_000L || target !in 1..1024) return null
        var sample = 1
        while ((width.toLong() + sample - 1) / sample > target || (height.toLong() + sample - 1) / sample > target) sample *= 2
        return sample
    }
    internal fun decode(file: File, target: Int): Bitmap? {
        if (!file.isFile || file.length() !in 1..MAX_FILE_BYTES) return null
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeFile(file.path, bounds)
        val sample = sampleSize(bounds.outWidth, bounds.outHeight, target) ?: return null
        return BitmapFactory.decodeFile(file.path, BitmapFactory.Options().apply { inSampleSize = sample })
    }
    suspend fun load(store: RelayStore, id: String, target: Int): Bitmap? = withContext(Dispatchers.IO) {
        val key = "${store.directory(id).absolutePath}:$target"
        cache.get(key) ?: permits.withPermit {
            cache.get(key) ?: try {
                (store.photoPreviews.load(id, target) ?: decode(File(store.directory(id), "payload"), target))?.also { cache.put(key, it) }
            } catch (_: Exception) { null }
        }
    }
}

@Composable private fun PhotoImage(file: Transfer, target: Int, modifier: Modifier = Modifier) {
    val store = (LocalContext.current.applicationContext as RelayApplication).store
    val bitmap by produceState<Bitmap?>(null, file.id, file.status, target) { value = PhotoThumbnails.load(store, file.id, target) }
    Box(modifier.background(MaterialTheme.colorScheme.surfaceContainerHighest), contentAlignment=Alignment.Center) {
        val current = bitmap
        if (current != null) Image(current.asImageBitmap(), contentDescription=file.name, modifier=Modifier.fillMaxSize().testTag("photo-image-${file.id}"), contentScale=ContentScale.Fit)
        else Text("Preview unavailable",Modifier.padding(12.dp),style=MaterialTheme.typography.labelSmall)
    }
}

@Composable internal fun PhotoCard(file: Transfer, modifier: Modifier = Modifier) {
    var open by remember(file.id) { mutableStateOf(false) }
    Column(modifier.testTag("photo-card-${file.id}")) {
        PhotoImage(file,256,Modifier.fillMaxWidth().aspectRatio(1f).clickable(onClickLabel="Preview ${file.name}") {open=true})
        Text(file.relativePath ?: file.name,Modifier.padding(top=8.dp),maxLines=2,overflow=TextOverflow.Ellipsis,style=MaterialTheme.typography.bodyMedium)
        Text(when {
            file.superseded -> "Superseded"
            file.relativePath == null -> "Uploaded"
            file.received && file.conflict -> "Synced · conflict copy kept"
            file.received -> "Synced"
            else -> "Waiting for Linux"
        },style=MaterialTheme.typography.labelSmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
    }
    if (open) AlertDialog(onDismissRequest={open=false},containerColor=MaterialTheme.colorScheme.surface,title={Text(file.name,maxLines=2,overflow=TextOverflow.Ellipsis)},
        text={Column {
            PhotoImage(file,1024,Modifier.fillMaxWidth().heightIn(max=360.dp).aspectRatio(1f))
            Text("Local preview · original file unchanged",Modifier.padding(top=8.dp),style=MaterialTheme.typography.labelSmall)
        }},confirmButton={TextButton(onClick={open=false}) {Text("Close")}})
}

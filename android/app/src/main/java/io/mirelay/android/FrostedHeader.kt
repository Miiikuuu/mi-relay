package io.mirelay.android

import android.app.ActivityManager
import android.content.Context
import android.os.Build
import android.os.PowerManager
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.BlurEffect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.TileMode
import androidx.compose.ui.graphics.layer.drawLayer
import androidx.compose.ui.graphics.rememberGraphicsLayer
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import kotlin.math.roundToInt

internal fun supportsAlbumGlass(api: Int, lowRam: Boolean, powerSave: Boolean, hardware: Boolean) =
    api >= 31 && !lowRam && !powerSave && hardware

/** Replays the content display list into a HEADER-SIZED blur layer. Never
 * captures bitmaps, blurs text/buttons, or renders the list a second time. */
@Composable internal fun FrostedHeader(
    modifier: Modifier = Modifier,
    surface: Color,
    reducedTransparency: Boolean,
    header: @Composable () -> Unit,
    content: @Composable (androidx.compose.ui.unit.Dp) -> Unit,
) {
    val context = LocalContext.current
    val density = LocalDensity.current
    val view = LocalView.current
    val glass = !reducedTransparency && supportsAlbumGlass(Build.VERSION.SDK_INT,
        (context.getSystemService(Context.ACTIVITY_SERVICE) as ActivityManager).isLowRamDevice,
        (context.getSystemService(Context.POWER_SERVICE) as PowerManager).isPowerSaveMode,
        view.isHardwareAccelerated)
    val source = rememberGraphicsLayer()
    val backdrop = rememberGraphicsLayer()
    val effect = remember(density) { BlurEffect(with(density) { 12.dp.toPx() }, with(density) { 12.dp.toPx() }, TileMode.Clamp) }
    var height by remember { mutableIntStateOf(0) }
    Box(modifier) {
        Box(Modifier.fillMaxSize().then(if (glass) Modifier.drawWithContent {
            source.record { this@drawWithContent.drawContent() }
            drawLayer(source)
        } else Modifier)) { content(with(density) { height.toDp() }) }
        Column(Modifier.fillMaxWidth().onSizeChanged { height = it.height }.clipToBounds()
            .testTag(if (glass) "album-glass" else "album-glass-fallback")
            .drawWithContent {
                if (glass) {
                    backdrop.record(size = IntSize(size.width.roundToInt(), size.height.roundToInt())) { drawLayer(source) }
                    backdrop.renderEffect = effect
                    drawLayer(backdrop)
                }
                drawRect(surface.copy(alpha = if (glass) 0.82f else 0.96f))
                drawContent()
            }) { header() }
    }
}

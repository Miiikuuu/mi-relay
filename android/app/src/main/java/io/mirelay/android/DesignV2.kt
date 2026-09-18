package io.mirelay.android

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithCache
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.withTransform
import androidx.compose.ui.graphics.drawscope.clipRect
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.platform.LocalWindowInfo
import kotlinx.coroutines.isActive
import kotlin.math.PI

/** Three cached gradient pairs, no framebuffer capture or full-screen blur.
 * Animation invalidates only the separate background draw layer, never reads
 * time during composition of the lazy file/photo content. */
@Composable internal fun RelayCanvas(content: @Composable BoxScope.() -> Unit) {
    val base = MaterialTheme.colorScheme.background
    val glass = MaterialTheme.colorScheme.surface.copy(alpha = 0.64f)
    val appearance = LocalAppearance.current
    val enabled = animateAmbient(appearance, LocalAppearanceEnvironment.current,
        LocalWindowInfo.current.isWindowFocused, LocalView.current.isHardwareAccelerated)
    val elapsed = remember { mutableLongStateOf(0L) }
    LaunchedEffect(enabled) {
        if (!enabled) return@LaunchedEffect
        var previous = 0L
        var lastDraw = 0L
        while (isActive) withFrameNanos { now ->
            if (previous == 0L) { previous = now; lastDraw = now }
            // At most 30 background redraws/sec. No timer survives cancellation.
            if (now - lastDraw >= 33_333_333L) {
                elapsed.longValue += (now - previous).coerceIn(0L, 100_000_000L)
                previous = now
                lastDraw = now
            }
        }
    }
    Box(Modifier.fillMaxSize().background(base)) {
    Box(Modifier.fillMaxSize().drawWithCache {
        val radius = size.maxDimension.coerceAtLeast(1f) * 0.72f
        val centers = listOf(Offset(0f, size.height * 0.2f), Offset(size.width, size.height * 0.48f), Offset(size.width * 0.3f, size.height))
        val colors = listOf(Color(0x88B9D9CD), Color(0x88BBCADF), Color(0x77E5CEC6))
        val normal = centers.mapIndexed { i, center -> Brush.radialGradient(listOf(colors[i], Color.Transparent), center, radius) }
        val shifted = centers.mapIndexed { i, center -> Brush.radialGradient(listOf(colors[(i + 1) % 3], Color.Transparent), center, radius) }
        val periods = listOf(12.0, 15.0, 14.0)
        onDrawBehind {
            if (!appearance.reduceTransparency) {
                val seconds = elapsed.longValue / 1_000_000_000.0
                val mix = ambientWave(seconds, 22.0)
                for (i in 0..2) {
                    val wave = ambientWave(seconds, periods[i]) - 0.5f
                    clipRect {
                        withTransform({ translate(size.width * wave * 0.45f * if (i == 1) -1f else 1f, size.height * wave * 0.2f) }) {
                            val origin = Offset(-size.width, -size.height)
                            val coverage = Size(size.width * 3, size.height * 3)
                            drawRect(normal[i], topLeft = origin, size = coverage, alpha = 1f - mix)
                            drawRect(shifted[i], topLeft = origin, size = coverage, alpha = mix)
                        }
                    }
                }
                drawRect(glass)
            }
        }
    })
    content()
    }
}

internal fun ambientWave(seconds: Double, period: Double): Float =
    ((1.0 - kotlin.math.cos((seconds % period) / period * PI * 2.0)) * 0.5).toFloat()

// Presentation only: never turn an unchecked or disconnected Folder into Paired.
internal fun pairedForDisplay(folder: Folder) = folder.pairingState == "ready" && !folder.connectionClosed

internal const val ALBUM_COLUMNS = 3

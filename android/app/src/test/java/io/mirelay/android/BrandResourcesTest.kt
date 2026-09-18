package io.mirelay.android

import android.app.Application
import android.graphics.BitmapFactory
import android.graphics.Color
import android.graphics.drawable.AdaptiveIconDrawable
import android.graphics.drawable.BitmapDrawable
import android.graphics.drawable.ColorDrawable
import android.graphics.drawable.InsetDrawable
import android.graphics.drawable.VectorDrawable
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [26, 35], application = Application::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class BrandResourcesTest {
    private val context get() = RuntimeEnvironment.getApplication()

    @Test fun originalArtKeepsItsFullCanvasAndSourceDimensions() {
        for ((id, width, height) in listOf(
            Triple(R.drawable.mirelay_brand_icon, 512, 512),
            Triple(R.drawable.mirelay_wordmark, 1586, 992),
        )) {
            val bitmap = BitmapFactory.decodeResource(context.resources, id)
            assertNotNull(bitmap)
            assertEquals(width, bitmap.width)
            assertEquals(height, bitmap.height)
            // The approved PNG has a slightly off-white corner; do not whiten it.
            val corner = bitmap.getPixel(0, 0)
            assertEquals(255, Color.alpha(corner))
            assertTrue(Color.red(corner) >= 250 && Color.green(corner) >= 250 && Color.blue(corner) >= 250)
            bitmap.recycle()
        }
        assertEquals("MiRelay — Send · Receive — by MiiiKuuu",
            context.getString(R.string.mirelay_brand_description))
    }

    @Test fun transparentDerivativesHaveRealAlphaAndPreserveFraming() {
        for ((id, width, height) in listOf(
            Triple(R.drawable.mirelay_brand_icon_transparent, 512, 512),
            Triple(R.drawable.mirelay_wordmark_transparent, 512, 320),
        )) {
            val bitmap = BitmapFactory.decodeResource(context.resources, id)
            assertEquals(width, bitmap.width)
            assertEquals(height, bitmap.height)
            assertTrue(bitmap.hasAlpha())
            assertEquals(0, Color.alpha(bitmap.getPixel(0, 0)))
            assertEquals(0, Color.alpha(bitmap.getPixel(width - 1, height - 1)))
            assertTrue((0 until height step 8).any { y ->
                (0 until width step 8).any { x -> Color.alpha(bitmap.getPixel(x, y)) == 255 && Color.red(bitmap.getPixel(x, y)) < 100 }
            })
            bitmap.recycle()
        }
    }

    @Test fun launcherInsetsTransparentArtAndNotificationsKeepTheirSymbol() {
        assertEquals(R.mipmap.ic_launcher, context.applicationInfo.icon)
        // Inflate the manifest's actual resource. Robolectric's PackageManager
        // icon map is a separate stub; real lookup is covered by DeviceUiTest.
        val icon = context.getDrawable(context.applicationInfo.icon)
        assertTrue(icon is AdaptiveIconDrawable)
        icon as AdaptiveIconDrawable
        assertEquals(Color.rgb(231, 237, 229), (icon.background as ColorDrawable).color)
        val foreground = icon.foreground as InsetDrawable
        foreground.setBounds(0, 0, 108, 108)
        val drawing = foreground.drawable as BitmapDrawable
        assertEquals(512, drawing.bitmap.width)
        assertEquals(512, drawing.bitmap.height)
        assertTrue(drawing.bitmap.hasAlpha())
        assertEquals(0, Color.alpha(drawing.bitmap.getPixel(0, 0)))
        assertTrue(drawing.bounds.left in 17..19)
        assertTrue(drawing.bounds.top in 17..19)
        assertTrue(drawing.bounds.right in 89..91)
        assertTrue(drawing.bounds.bottom in 89..91)
        assertTrue(context.getDrawable(R.drawable.ic_relay) is VectorDrawable)
    }
}

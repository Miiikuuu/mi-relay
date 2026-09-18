package io.mirelay.android

import android.app.Application
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class DesignV2IconsTest {
    @Test fun suppliedVectorsRenderAsVisibleOutlinesNotSolidTiles() {
        val context = RuntimeEnvironment.getApplication()
        for (id in listOf(R.drawable.relay_folder, R.drawable.relay_photos, R.drawable.relay_link)) {
            val bitmap = Bitmap.createBitmap(96, 96, Bitmap.Config.ARGB_8888)
            try {
                val drawable = context.getDrawable(id)!!
                drawable.setBounds(0, 0, 96, 96)
                drawable.draw(Canvas(bitmap))
                val pixels = IntArray(96 * 96)
                bitmap.getPixels(pixels, 0, 96, 0, 0, 96, 96)
                val visible = pixels.count { Color.alpha(it) > 32 }
                assertTrue("Vector $id must be visible: $visible", visible > 300)
                assertTrue("Vector $id must remain an outline: $visible", visible < pixels.size / 2)
            } finally { bitmap.recycle() }
        }
    }
}

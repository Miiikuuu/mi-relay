import unittest

from PIL import Image

from prepare_brand_icon import OUTPUT, SIZES, SOURCE
from prepare_brand_wordmark import extract_canvas


class BrandIconTests(unittest.TestCase):
    def test_master_preserves_all_original_rgb_pixels(self):
        with Image.open(SOURCE) as original, Image.open(
            OUTPUT / "mirelay-original-transparent.png"
        ) as result:
            self.assertEqual(result.size, original.size)
            self.assertEqual(result.mode, "RGBA")
            self.assertEqual(result.convert("RGB").tobytes(), original.tobytes())
            self.assertEqual(result.tobytes(), extract_canvas(original).tobytes())
            alpha = result.getchannel("A")
            self.assertEqual(alpha.getextrema(), (0, 255))
            self.assertGreater(alpha.histogram()[0], original.width * original.height * 0.3)
            self.assertLess(alpha.histogram()[0], original.width * original.height * 0.75)

    def test_every_launcher_size_is_transparent_and_matches_master(self):
        with Image.open(OUTPUT / "mirelay-original-transparent.png") as master:
            for size in SIZES:
                with self.subTest(size=size), Image.open(OUTPUT / f"mirelay-{size}.png") as icon:
                    self.assertEqual(icon.mode, "RGBA")
                    self.assertEqual(icon.size, (size, size))
                    self.assertEqual(icon.tobytes(), master.resize((size, size), Image.Resampling.LANCZOS).tobytes())
                    self.assertEqual(icon.getchannel("A").getextrema(), (0, 255))
                    for point in [(0, 0), (size - 1, 0), (0, size - 1), (size - 1, size - 1)]:
                        self.assertEqual(icon.getpixel(point)[3], 0)

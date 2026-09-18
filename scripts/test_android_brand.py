import unittest

from PIL import Image, ImageOps

from prepare_android_brand import DESTINATION, ICON, WORDMARK


class AndroidBrandTests(unittest.TestCase):
    def test_wordmark_is_bounded_alpha_aware_derivative(self):
        with Image.open(WORDMARK) as source, Image.open(DESTINATION / "mirelay_wordmark_transparent.png") as result:
            expected = ImageOps.contain(source, (512, 512), Image.Resampling.LANCZOS)
            self.assertEqual(result.size, expected.size)
            self.assertEqual(result.tobytes(), expected.tobytes())
            self.assertEqual(result.mode, "RGBA")
            self.assertLessEqual(result.width * result.height * 4, 660_000)
            self.assertEqual(result.getpixel((0, 0))[3], 0)

    def test_foreground_matches_transparent_launcher_asset(self):
        self.assertEqual(ICON.read_bytes(), (DESTINATION / "mirelay_brand_icon_transparent.png").read_bytes())

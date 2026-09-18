import unittest

from PIL import Image, ImageDraw

from prepare_brand_wordmark import OUTPUT, SOURCE, extract_canvas


class BrandWordmarkTests(unittest.TestCase):
    def test_only_exterior_white_is_removed(self):
        original = Image.new("RGB", (20, 20), (253, 253, 253))
        ImageDraw.Draw(original).rectangle((4, 4, 15, 15), outline=(210, 210, 210))
        original.putpixel((8, 8), (125, 154, 101))
        result = extract_canvas(original)
        self.assertEqual(result.getpixel((0, 0))[3], 0)
        self.assertEqual(result.getpixel((5, 5))[3], 255)  # enclosed sticker white
        self.assertEqual(result.getpixel((4, 4)), (210, 210, 210, 255))
        self.assertEqual(result.getpixel((8, 8)), (125, 154, 101, 255))
        self.assertEqual(result.convert("RGB").tobytes(), original.tobytes())

    def test_original_pixels_and_dimensions_are_preserved(self):
        with Image.open(SOURCE) as original, Image.open(OUTPUT) as result:
            self.assertEqual(result.mode, "RGBA")
            self.assertEqual(result.size, original.size)
            self.assertEqual(result.convert("RGB").tobytes(), original.tobytes())
            self.assertEqual(result.tobytes(), extract_canvas(original).tobytes())
            alpha = result.getchannel("A")
            self.assertEqual(alpha.getextrema(), (0, 255))
            transparent = alpha.histogram()[0]
            self.assertGreater(transparent, original.width * original.height * 0.25)
            self.assertLess(transparent, original.width * original.height * 0.65)
            for point in [(0, 0), (original.width - 1, original.height - 1)]:
                self.assertEqual(alpha.getpixel(point), 0)

    def test_unexpected_source_format_is_rejected(self):
        with self.assertRaises(ValueError):
            extract_canvas(Image.new("RGBA", (2, 2)))

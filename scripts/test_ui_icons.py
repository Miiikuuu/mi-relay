import unittest
import xml.etree.ElementTree as ET
from prepare_ui_icons import NAMES, ROOT, SVG, symbolic


class UiIconTests(unittest.TestCase):
    def test_checked_in_symbolics_match_original_geometry_conversion(self):
        for name in NAMES:
            with self.subTest(name=name):
                expected = symbolic((ROOT / "icons" / f"{name}.svg").read_text())
                self.assertEqual(expected, (ROOT / "gtk-icons" / f"{name}.svg").read_text())

    def test_every_primitive_has_explicit_symbolic_paint_and_stroke(self):
        for name in NAMES:
            for child in ET.fromstring((ROOT / "gtk-icons" / f"{name}.svg").read_text()):
                self.assertIn(child.tag, (SVG + "path", SVG + "rect", SVG + "circle"))
                self.assertIn("foreground-stroke", child.get("class"))
                self.assertEqual("1.6", child.get("stroke-width"))
                self.assertEqual("round", child.get("stroke-linecap"))

    def test_unsupported_input_is_not_silently_dropped(self):
        with self.assertRaises(ValueError):
            symbolic('<svg xmlns="http://www.w3.org/2000/svg"><image /></svg>')

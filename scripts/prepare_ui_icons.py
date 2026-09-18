#!/usr/bin/env python3
"""Derive GTK symbolics from the supplied Lucide SVGs; no external dependencies.

GTK's symbolic subset does not inherit ordinary SVG paint attributes, and uses
paths instead of line elements. Explicit paints also keep librsvg-era renderers
compatible. Original kit geometry remains in assets/ui/v2/icons.
"""
from pathlib import Path
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1] / "assets/ui/v2"
NAMES = ("folder-open", "images", "link-2", "settings-2", "sliders-horizontal", "folder-plus")
SVG = "{http://www.w3.org/2000/svg}"


def symbolic(source):
    root = ET.fromstring(source)
    for child in root:
        if child.tag == SVG + "line":
            a = child.attrib
            child.tag = SVG + "path"
            child.attrib = {"d": f'M{a["x1"]},{a["y1"]} L{a["x2"]},{a["y2"]}'}
        if child.tag not in (SVG + "path", SVG + "circle", SVG + "rect"):
            raise ValueError(f"Unsupported SVG primitive: {child.tag}")
        filled = child.get("fill", "none") != "none"
        child.set("class", "foreground-fill foreground-stroke" if filled else "transparent-fill foreground-stroke")
        child.set("style", f"fill:{'#2e3436' if filled else 'none'};stroke:#2e3436")
        for name, default in (("stroke-width", "1.6"), ("stroke-linecap", "round"), ("stroke-linejoin", "round")):
            child.set(name, root.get(name, default))
    ET.register_namespace("", SVG[1:-1])
    return ET.tostring(root, encoding="unicode") + "\n"


if __name__ == "__main__":
    output = ROOT / "gtk-icons"
    output.mkdir(exist_ok=True)
    for name in NAMES:
        (output / f"{name}.svg").write_text(symbolic((ROOT / "icons" / f"{name}.svg").read_text()), encoding="utf-8")

"""Extract only the border-connected white canvas; never redraw the artwork.

Requires Pillow. Run from any directory with no arguments to regenerate the
checked-in Linux sidebar asset. The approved source and its RGB pixels stay
unchanged, including enclosed white lettering/sticker outlines and green ink.
"""

from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageOps

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "assets/brand/MiRelay-brand-kit-v1/wordmark/mirelay-wordmark.png"
OUTPUT = ROOT / "assets/ui/v2/brand/mirelay-wordmark-transparent.png"


def extract_canvas(source: Image.Image) -> Image.Image:
    if source.mode != "RGB":
        raise ValueError("Expected the original opaque RGB artwork")
    red, green, blue = source.split()
    minimum = ImageChops.darker(ImageChops.darker(red, green), blue)
    maximum = ImageChops.lighter(ImageChops.lighter(red, green), blue)
    light = minimum.point(lambda value: 255 if value >= 245 else 0)
    neutral = ImageChops.subtract(maximum, minimum).point(
        lambda value: 255 if value <= 6 else 0
    )
    candidate = ImageChops.multiply(light, neutral)
    # Padding connects every canvas edge without crossing the sticker outline.
    exterior = ImageOps.expand(candidate, border=1, fill=255)
    ImageDraw.floodfill(exterior, (0, 0), 127, thresh=0)
    alpha = exterior.crop((1, 1, source.width + 1, source.height + 1)).point(
        lambda value: 0 if value == 127 else 255
    )
    result = source.copy()
    result.putalpha(alpha)
    return result


def main() -> None:
    with Image.open(SOURCE) as source:
        result = extract_canvas(source)
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    result.save(OUTPUT)
    print(OUTPUT)


if __name__ == "__main__":
    main()

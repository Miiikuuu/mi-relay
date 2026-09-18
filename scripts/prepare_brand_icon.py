"""Derive transparent Linux launcher icons without modifying the brand kit."""

from PIL import Image

from prepare_brand_wordmark import ROOT, extract_canvas

SOURCE = ROOT / "assets/brand/MiRelay-brand-kit-v1/icons/mirelay-icon-original.png"
OUTPUT = ROOT / "assets/ui/v2/brand/icons"
SIZES = (16, 24, 32, 48, 64, 96, 128, 192, 256, 512, 1024)


def main() -> None:
    with Image.open(SOURCE) as source:
        icon = extract_canvas(source)
    OUTPUT.mkdir(parents=True, exist_ok=True)
    icon.save(OUTPUT / "mirelay-original-transparent.png")
    for size in SIZES:
        # Pillow uses premultiplied alpha when resampling RGBA, avoiding white
        # fringes from the now-transparent canvas. Keep the original framing.
        icon.resize((size, size), Image.Resampling.LANCZOS).save(
            OUTPUT / f"mirelay-{size}.png"
        )
    print(f"Generated {len(SIZES)} launcher sizes in {OUTPUT}")


if __name__ == "__main__":
    main()

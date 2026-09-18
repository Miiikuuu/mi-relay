"""Bound Android brand decoding without touching the approved originals."""

import shutil

from PIL import Image, ImageOps

from prepare_brand_wordmark import ROOT

DESTINATION = ROOT / "android/app/src/main/res/drawable-nodpi"
WORDMARK = ROOT / "assets/ui/v2/brand/mirelay-wordmark-transparent.png"
ICON = ROOT / "assets/ui/v2/brand/icons/mirelay-512.png"


def main() -> None:
    with Image.open(WORDMARK) as source:
        scaled = ImageOps.contain(source, (512, 512), Image.Resampling.LANCZOS)
        scaled.save(DESTINATION / "mirelay_wordmark_transparent.png")
    shutil.copyfile(ICON, DESTINATION / "mirelay_brand_icon_transparent.png")
    print(f"Android wordmark: {scaled.width} x {scaled.height}, RGBA; icon: 512 x 512")


if __name__ == "__main__":
    main()

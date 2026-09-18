# Sidebar wordmark

`mirelay-wordmark-transparent.png` is a deterministic derivative of the approved
`assets/brand/MiRelay-brand-kit-v1/wordmark/mirelay-wordmark.png` (SHA-256
`33dff809e9c54e6aea9d4c31faf0dc6cac3ba8180e560f21ff50bfc76a1e6d97`).

The user approved direct background extraction on 2026-09-15. Only near-white,
neutral pixels connected to the outside canvas are made transparent. Dimensions,
all RGB pixels, lettering, green accents and enclosed white sticker outlines are
unchanged. The original remains embedded and is used in About. No generated or
redrawn logo is used, and no image processing runs in the app.

Regenerate with Pillow installed:

```sh
python3 scripts/prepare_brand_wordmark.py
python3 -m unittest discover -s scripts -p test_brand_wordmark.py -v
```

## Linux launcher icon

`icons/` contains the full-resolution transparent master and eleven launcher
sizes (16–1024 px), derived from `icons/mirelay-icon-original.png` in the original
brand kit (SHA-256
`b4b68c3be0580998a878c4c96abf7da16d2bc40b6ef4fe730e523a7c3aacdd10`).
The same canvas-only alpha extraction preserves the master's dimensions and all
RGB pixels. Size variants use alpha-aware Lanczos resampling, with no reframing.
These are used for both Linux GResources and installed hicolor icons. The
original favicon is not changed. Android now reuses the transparent 512 px icon
as its inset adaptive foreground, over its separate pale-sage background.

```sh
python3 scripts/prepare_brand_icon.py
python3 -m unittest discover -s scripts -p 'test_brand_*.py' -v
```

Android's bounded wordmark and foreground copies are generated separately:

```sh
python3 scripts/prepare_android_brand.py
python3 -m unittest discover -s scripts -p test_android_brand.py -v
```

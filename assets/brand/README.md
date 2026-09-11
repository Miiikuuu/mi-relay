# MiRelay brand assets

Project and creator: **MiRelay**, by **MiiiKuuu**.

`MiRelay-brand-kit-v1/` is an unchanged copy of the supplied
`MiRelay-brand-kit-v1.zip`. Its original handoff, metadata and Chinese README are
kept with the artwork. All 18 entries listed in the original `SHA256SUMS.txt`
are preserved byte-for-byte; this wrapper is not part of that manifest.

```bash
cd assets/brand/MiRelay-brand-kit-v1
sha256sum -c SHA256SUMS.txt
```

## Integration

| Surface | Original asset | Rendering |
| --- | --- | --- |
| GitHub README | `wordmark/mirelay-wordmark.png` | Entire canvas, proportional width |
| Linux launcher | `icons/png/mirelay-*.png` | Embedded icon-theme resources; installed PNGs for the application menu |
| Linux header / About | 128 px icon / complete wordmark | White surfaces, original aspect ratio |
| Android launcher | `icons/png/mirelay-512.png` | Adaptive icon with uniform inset |
| Android top-left header / drawer | Complete wordmark | Fit, never crop; white container in both themes; replaces plain app-name text |

The Android wordmark stays in the top-left app bar on both the welcome and Folder
screens, without a second centered welcome logo. Folder navigation remains
available through the menu action on the right.

The Android copies live in `android/app/src/main/res/drawable-nodpi/` so Android
does not reinterpret their source density. They must remain byte-identical to
their originals. Desktop resources are compiled by `build.rs` only with the
`desktop` feature; the executable does not load assets from the checkout at
runtime. See [Linux installation](../../packaging/linux/README.md).

The artwork is **opaque white-background PNG**, not a transparent cutout. Its
white canvas, three-i symbol, signature, lettering and file illustration are
intentional and are not cropped, recolored or redrawn. Black primary controls
in the apps remain unchanged.

Android's adaptive launcher wraps the unmodified square PNG in a white background
and a uniform inset, keeping the actual drawing inside the launcher's safe region.
See [Android's adaptive icon specification](https://developer.android.com/develop/ui/compose/system/icon_design_adaptive).
The pack does not contain independent foreground/background layers, a monochrome
mask, or vector masters. No monochrome/themed-logo resource is fabricated; the
existing `ic_relay` transfer symbol is retained for system notifications.

If those variants are needed later, obtain approved transparent/vector/monochrome
originals from the creator. Do not derive a silhouette by silently removing the
background or turning the colored PNG into a solid square.

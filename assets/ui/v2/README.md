# MiRelay UI v2

Selected implementation assets from the user-supplied `MiRelay-design-kit-v2.zip`.
The archive and the extracted offline prototypes are design input, not application
code. Reference checksums were verified before integration.

- `DESIGN-HANDOFF.md` and `design-tokens.json`: original handoff, unchanged.
- `icons/`: original Lucide SVG geometry; retain `lucide-LICENSE.txt` when distributing.
- `gtk-icons/`: derived symbolics with explicit per-path paint and line-to-path
  conversion, reproducible with `python3 scripts/prepare_ui_icons.py`. See the
  [GTK symbolic format](https://docs.gtk.org/gtk4/icon-format.html).
- Linux embeds these derived icons through GResource. Android ports the same folder,
  images and link paths to vector drawables; no runtime downloads or JavaScript.
- The supplied wordmark is byte-identical to the existing v1 artwork. Both clients
  now use the approved canvas-only transparent derivative in their headers; the
  original remains available and is used in About. See [brand derivation](brand/README.md).
  The unapproved horizontal concept is not substituted.
- Demo photos and fake pairing/transfer data are never loaded into the product.

See `docs/ui-v2.md` for native implementation scope and verification limitations.

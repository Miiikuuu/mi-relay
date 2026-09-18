# Linux desktop integration

Build from the repository root:

```bash
cargo build --locked --release --features desktop --bin mirelay-desktop
```

The binary includes its icon theme and About artwork as a GResource; moving the
binary does not require copying the repository. The following optional commands
install a user-local executable, application-menu entry and transparent PNG icons.
They replace files with the same names, so review any existing installation first.
They do not change Folder configuration, restart a running app or enable autostart.

```bash
install -Dm755 target/release/mirelay-desktop "$HOME/.local/bin/mirelay-desktop"
install -Dm644 packaging/linux/io.mirelay.Desktop.desktop \
  "$HOME/.local/share/applications/io.mirelay.Desktop.desktop"

for size in 16 24 32 48 64 96 128 192 256 512 1024; do
  install -Dm644 "assets/ui/v2/brand/icons/mirelay-${size}.png" \
    "$HOME/.local/share/icons/hicolor/${size}x${size}/apps/io.mirelay.Desktop.png"
done
update-desktop-database "$HOME/.local/share/applications"
gtk-update-icon-cache --force --ignore-theme-index "$HOME/.local/share/icons/hicolor"
```

The transparent launcher sizes are deterministic derivatives of the original
sticker artwork. Only the outside white canvas is removed; the sticker outline,
shadow and original framing remain. Regenerate with
`python3 scripts/prepare_brand_icon.py` (Pillow required). The original brand kit
is unchanged, and these Linux assets do not change Android's adaptive icon.

Ensure `$HOME/.local/bin` is on the graphical session's `PATH`, then reopen the
application menu or log in again if the desktop has cached its entries. The
launcher uses the stable application ID `io.mirelay.Desktop`. It does not carry
tokens, paths to private Folders, or startup arguments.

These commands show the usual XDG locations. If `XDG_DATA_HOME` is customized,
install the desktop entry and icons under that directory instead. For
distribution packaging, install the executable under the package's `bin`
directory and place the entry/icons under its `share` directory.

The resource compiler is a desktop-only build prerequisite (`libglib2.0-bin` on
Debian/Ubuntu). CLI, server and Android-native builds skip resource compilation.

# Frontend playground

Temporary, local-only data for tuning the native Linux frontend. Requires the same Rust, GTK 4.6+ and libadwaita development dependencies as the desktop app.

```bash
./dev/frontend/run.sh
```

The launcher seeds `.local/default/` on first use, compiles current source and opens an independent window. An already-running MiRelay window is left alone. Subsequent launches preserve your configuration, selection and received files. No production configuration is imported. Generated files and screenshots under `.local/` are ignored by Git; this scaffold is tracked.

## Samples

- **Documents**: received text/CSV, a pending acknowledgement, a retry error, and one incoming file waiting for **Receive**.
- **Illustrations**: Photos category, the original supplied icon as a preview sample,
  and a tiny PNG with a failed wallpaper state. Failures are in Transfer activity.
- **Empty Folder**: no transfers.
- **Long Folder name**: long English/Unicode names for truncation and tooltip checks.

All sample sources use local filesystem inboxes, Automatic Receive starts off, and wallpaper commands are empty. No server, token, network connection or wallpaper change is needed. Hashes and stored files come from the real Rust receive pipeline; pending/error display states are synthetic, not evidence of network testing. Clicking **Receive** changes those states and receives the queued local sample.

Folder Settings can be opened for layout checks, but the current editor only saves HTTP configurations: a filesystem sample has an empty Server URL and cannot be saved without entering a URL. Doing so explicitly changes that sample to a network source; use a disposable workspace when testing configuration edits.

## Useful commands

```bash
# Prepare samples without opening a window.
./dev/frontend/run.sh --prepare

# No registered Folders (first-run screen).
./dev/frontend/run.sh --empty

# Fresh disposable workspace without overwriting previous experiments.
./dev/frontend/run.sh --workspace layout-v2

# Screenshots require a working graphical session and exit automatically.
./dev/frontend/run.sh --screenshot "$PWD/dev/frontend/.local/default/screenshots/main.png"
./dev/frontend/run.sh --screenshot "$PWD/dev/frontend/.local/default/screenshots/narrow.png" --screenshot-width 820
./dev/frontend/run.sh --screenshot "$PWD/dev/frontend/.local/default/screenshots/settings.png" --screenshot-settings

# Fixture regression tests (no graphical session required).
cargo test --locked --features desktop --example frontend-fixture

# Exercise real GTK sorting, filtering, selection and new-file notices, then exit.
# Uses a disposable workspace and performs no network transfers.
./dev/frontend/run.sh --workspace sidebar-check --sidebar-smoke-test

# Exercise file filtering/sorting, pagination and synthetic live file events.
./dev/frontend/run.sh --workspace files-check --file-smoke-test

# Opt-in 1k / 10k / 50k history and 100 / 1k Folder stress measurements.
# Synthetic data stays in memory; stdout includes QA_PERF / QA_MEMORY results.
MIRELAY_DESKTOP_STRESS=1 CARGO_BUILD_JOBS=1 ./dev/frontend/run.sh --workspace stress-check --file-smoke-test

# Capture the compact sidebar menus or the no-results state.
./dev/frontend/run.sh --screenshot "$PWD/dev/frontend/.local/default/screenshots/sort.png" --screenshot-sidebar-menu sort
./dev/frontend/run.sh --screenshot "$PWD/dev/frontend/.local/default/screenshots/filter.png" --screenshot-sidebar-menu filter
./dev/frontend/run.sh --screenshot "$PWD/dev/frontend/.local/default/screenshots/no-results.png" --screenshot-folder-filter new-files

# Capture file menus, quiet completed rows, or a synthetic active download.
./dev/frontend/run.sh --screenshot "$PWD/dev/frontend/.local/default/screenshots/file-filter.png" --screenshot-sidebar-menu file-filter
./dev/frontend/run.sh --screenshot "$PWD/dev/frontend/.local/default/screenshots/file-sort.png" --screenshot-sidebar-menu file-sort
./dev/frontend/run.sh --screenshot "$PWD/dev/frontend/.local/default/screenshots/completed.png" --screenshot-file-filter completed
./dev/frontend/run.sh --screenshot "$PWD/dev/frontend/.local/default/screenshots/downloading.png" --screenshot-file-activity
```

Edit `src/desktop.rs` for UI/CSS, `dev/frontend/seed.rs` for samples, then rerun. Sample changes only apply to a fresh `--workspace NAME`; there is intentionally no destructive reset command. Configuration and state paths are absolute, so use a new workspace after moving the checkout. `.local/NAME/folders/<id>/` contains each Folder's config, library, state, samples and inbox; `.local/NAME/screenshots/` is reserved for visual checks.

For directory-mode samples, seed a **new, empty** workspace directly:

```bash
cargo run --locked --features desktop --example frontend-fixture -- /absolute/new/directory-preview --directory
target/debug/mirelay-desktop --new-instance --registry /absolute/new/directory-preview/folders.toml
```

This adds and selects `Shared files`, with nested filenames, a retained conflict copy and a pending receipt. Its server URL is an unused loopback endpoint and Auto is off. File writes/hash/history use the directory core; receipt flags are **display fixtures**, not evidence of network delivery. Do not enter real credentials or enable Receive in this visual workspace. Existing workspaces are reused unchanged; `--directory` is an option for the fixture example, not the desktop binary.

For an album-specific layout workspace, seed a **new, empty** directory:

```bash
cargo run --locked --features desktop --example frontend-fixture -- /absolute/new/album-preview --album
target/debug/mirelay-desktop --new-instance --registry /absolute/new/album-preview/folders.toml
```

This selects Illustrations and adds 120 deterministic gradient PNGs, in landscape
and portrait shapes, through the real local receive pipeline. No photos, account,
network or production credentials are needed. `--album-stress` generates 600
gradient images instead, to exercise pagination and recycled cells. `--album`,
`--album-stress` and `--directory` are mutually exclusive fixture options;
existing workspaces are never reseeded.
See [Linux album checks and measurements](../../docs/linux-album.md).

The album benchmark separates post-scroll animation from true idle: it observes
actual paint signals and scrollbar opacity changes. The application now uses a
regular scrollbar for Cairo, automatically detected when the album maps; other
renderers keep the original overlay preference. `MIRELAY_ALBUM_QA_OVERLAY_SCROLLBARS=1`
forces the old overlay path in the grid benchmark; `MIRELAY_ALBUM_QA_SOLID_SCROLLBARS=1`
forces its regular-scrollbar control. These mutually exclusive overrides affect
only the test widget, including remaps; neither is an application setting.
The standalone `gdk-idle-probe.c` is a negative
control for raw pixel conversion, not a reproduction of the scrollbar animation.
Compilation and comparison commands are in the linked report.

The real-process X11 scrolling diagnostic requires Python 3, libX11 and libXtst,
and **a dedicated Xvfb display**, never your ordinary desktop. It launches its own
desktop child, identifies its mapped window by PID, injects wheel events, verifies
changed window pixels and checks fixture hashes before stopping only that child.
It accepts disposable registries under this checkout's `target/` and refuses
`:0`/`:1`; this guard does not automatically prove other display numbers are safe.
Use the actual display number returned by your isolated Xvfb instance.

```bash
cargo run --locked --features desktop --example frontend-fixture -- "$PWD/target/album-stress-new" --album-stress
# Example only: :2 must belong to your dedicated Xvfb server.
python3 dev/frontend/album-x11-check.py --registry target/album-stress-new/folders.toml --isolated-display :2 --more-clicks 6
```

Six Show More clicks load all 602 records of that fresh fixture (600 gradients
plus two existing samples). The test checks disappearance of the footer via pixel
readback; its click coordinates assume the current compact desktop layout. A
changed layout or display setup can fail the harness and must not be relabeled as
an application failure. Omitting `--more-clicks` tests only the initial page.

See the [fix verification report](../../docs/qa-fixes-2026-09-05.md) for earlier performance numbers, screenshot-error regression coverage, actual process-kill recovery checks, and input guards. The [original QA report](../../docs/qa-2026-09-05.md) is retained as a pre-fix baseline. Graphical diagnostics are opt-in; a passing default test suite does not include those checks. Screenshot write failures now report an error and exit normally with code 1; successful captures exit with code 0.

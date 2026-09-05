# Frontend playground

Temporary, local-only data for tuning the native Linux frontend. Requires the same Rust, GTK 4.6+ and libadwaita development dependencies as the desktop app.

```bash
./dev/frontend/run.sh
```

The launcher seeds `.local/default/` on first use, compiles current source and opens an independent window. An already-running MiRelay window is left alone. Subsequent launches preserve your configuration, selection and received files. No production configuration is imported. Generated files and screenshots under `.local/` are ignored by Git; this scaffold is tracked.

## Samples

- **Documents**: received text/CSV, a pending acknowledgement, a retry error, and one incoming file waiting for **Receive**.
- **Illustrations**: valid tiny PNGs with unconfigured/failed wallpaper states.
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

See the [fix verification report](../../docs/qa-fixes-2026-09-05.md) for current performance numbers, screenshot-error regression coverage, actual process-kill recovery checks, and input guards. The [original QA report](../../docs/qa-2026-09-05.md) is retained as a pre-fix baseline. Graphical diagnostics are opt-in; a passing default test suite does not include those checks. Screenshot write failures now report an error and exit normally with code 1; successful captures exit with code 0.

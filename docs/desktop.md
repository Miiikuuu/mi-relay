# MiRelay for Linux

`mirelay-desktop` is a native GTK 4 frontend built on the existing Rust receive core. Delivery-only Folders share the CLI configuration, state, content-addressed storage, HTTP retry, verification, acknowledgement, and wallpaper state machine. Opt-in **Directory sync** Folders use the shared directory receiver instead; see [setup and validation](desktop-directory-sync.md). Neither mode introduces a separate transfer implementation.

## Build and run

GTK 4.6 or newer and libadwaita development libraries are required. On Debian or Ubuntu:

```bash
sudo apt install libgtk-4-dev libadwaita-1-dev libglib2.0-bin
cargo build --release --features desktop --bin mirelay-desktop
```

Run the development build:

```bash
cargo run --features desktop --bin mirelay-desktop
```

The original MiRelay artwork is embedded in the executable. The small icon in
the header opens **About MiRelay**, showing the complete wordmark and creator
credit. For an application-menu entry and launcher icons, follow the
[Linux desktop integration instructions](../packaging/linux/README.md).

For isolated UI development with sample Folders and transfer states, run `./dev/frontend/run.sh`. See the [frontend playground](../dev/frontend/README.md) for fresh workspaces and screenshots. Its `--new-instance --registry PATH` launch leaves any existing desktop instance alone.

Import and select an existing CLI configuration:

```bash
cargo run --features desktop --bin mirelay-desktop -- \
  --config /path/to/config.toml
```

Use a separate Folder registry for isolated testing:

```bash
cargo run --features desktop --bin mirelay-desktop -- \
  --registry /tmp/mirelay-desktop/folders.toml \
  --config /path/to/config.toml
```

The default Folder registry lives in MiRelay's XDG configuration directory. The previous single default configuration is imported safely on first launch. Removing an imported Folder prevents it from being imported again automatically.

## Interface

The interface uses black primary controls and Folder selection, compact property rows, and a narrow sidebar. Light and dark themes retain readable controls and keyboard focus. Long paths and status labels are abbreviated to fit; hover to read the full value.

- A Folder is the primary managed object and represents one local delivery workflow.
- The sidebar shows single-line Folder names without file counts or permanent status dots. Selecting one shows its properties and activity on the right.
- The sort popover has one row per field (Name / Last received), with paired ▲ / ▼ buttons for ascending and descending order. Folders with no received files stay last in either time direction. The filter menu combines name search with All Folders, Needs Attention, Pending, or New Files. `Ctrl+F` opens filtering. Sorting/filtering never changes the selected Folder; empty results offer Clear Filters.
- A small symbolic icon appears only for an error, pending work, or newly received files in another Folder. Opening a Folder clears its new-file marker, not its pending/error state. An intentionally disabled wallpaper command is not a sidebar warning.
- Folder properties include its local path, source, device ID, file-size limit, and wallpaper workflow.
- The Files list belongs to the selected Folder. Its sort popover uses the same paired triangles, with just three rows: Name, Date received, and Size. Only the selected direction is highlighted; tooltips explain the ordering and keyboard focus follows the current selection when opening the popover. Filtering combines filename search with file type and transfer status. Preferences are remembered separately per Folder during the session.
- Active operations always sort first among matching files. Filtering searches the entire local history, not just recent records; long lists render 100 rows at a time with Show More.
- Completed files show their name, size, and time without a persistent Received label or checkmark. The top-level Receive button remains the action for checking for new deliveries.
- Regular files, wallpaper images, pending acknowledgements, and failures use the same compact status model.
- Receive work runs in the background, with manual refresh and an action to open the selected Folder.
- You can add, rename, switch, and remove multiple Folders. Removing one does not delete its configuration, state, or received files.
- Each Folder has its own managed path and HTTPS or trusted-local-HTTP connection settings.
- Automatic Receive is opt-in per Folder and checks every 60 seconds while the app is open.
- `Ctrl+R` receives files and `Ctrl+,` opens Folder Settings.

Delivery-only Folders continue to use the standard Rust CLI pipeline. Directory Folders explicitly store `directory_sync = true`; the old `sync` and wallpaper retry commands refuse this mode instead of flattening files into the target. The desktop registry maintains Folder registrations, selection and Auto flags. A broken Folder is isolated from healthy ones. MiRelay rejects overlapping managed paths, shared state files (including directory state), reused device identities, inbox conflicts, and Folders that would compete for the same HTTP receive queue.

Network synchronization runs on a worker thread while the GTK main thread handles rendering and input. Actions are disabled until a running task finishes. Cross-process state locks still prevent the CLI and desktop app from updating the same state concurrently.

History snapshots share their records, and loading prepares filename/type/ID indexes on the worker. Sorting selects only the requested page before ordering it; filtering still examines the whole history. Settled files update their indexed record, identical phase events skip redraws, and unchanged visible rows keep their widgets. These optimizations do not change transfer state or file ordering. See the [fix verification report](qa-fixes-2026-09-05.md) for measured costs and remaining limits.

The worker sends real file phases before downloading, verifying existing content, confirming receipt, retrying, or applying wallpaper. These are indeterminate operation indicators, not byte percentages. Waiting acknowledgements are not presented as actively downloading. Settlement and fatal errors stop active indicators; failures without a persisted record remain visible until retry or refresh. File phases are UI-only and do not change the on-disk transfer schema.

The receive action and active file rows share the Dash Ring indicator: a 24-unit viewBox, radius 9.5, rounded 2-unit strokes, and a 10% opacity track. The foreground rotates every 2 seconds, while its dash length (`0 → 42 → 42`, gap 150) and offset (`0 → −16 → −59`) interpolate over a separate 1.5-second cycle. The native renderer preserves the supplied SVG's timing and dash clipping. It follows the theme, stops ticking while hidden, and shows a static visible arc when system animations are disabled. Rebuilding a file row does not restart the animation phase.

Automatic Receive is disabled by default, including for configurations imported from the CLI. Enable it in Folder Settings. Each interval schedules all enabled, healthy Folders in one background worker and isolates a failure in one Folder from the others. If the app is already loading, receiving or editing Folder settings, that interval is skipped rather than starting overlapping work.

Sorting, filters, and new-file markers are currently session-local. Startup establishes a baseline without marking existing files new; subsequent Receive or Refresh detects new delivery IDs, including when the file count stays the same. File counts remain available in the detail pane. Both sort controls share a custom 16px monochrome icon (three tapering lines and paired triangles), following the [GNOME interface guidelines](https://developer.gnome.org/hig/guidelines/ui-icons.html); its vectors inherit theme colors without depending on the installed icon theme.

## Token boundary

For server-isolated Folders, Folder Settings includes invitation creation,
handshake checks and receiver confirmation. Save validates the connection off the
GTK thread before changing the registry. See [Folder pairing](folder-pairing.md).

When creating a Folder, use **Pairing → Administrator credential**, not
**Session Token**. Pasted leading/trailing whitespace is removed; failures appear
at that field and retain its input. Success clears the administrator input and
fills Session Token with the new Folder's receiver credential.

Configuration files only store the environment-variable name, never the bearer token. If Session Token is empty in Folder Settings, the desktop app reads that environment variable. A token entered manually stays in process memory and must be entered again after the app closes. Secret Service or another system keyring can be added after the desktop MVP.

## Build isolation

GUI dependencies are behind the `desktop` feature:

```bash
# Build only the CLI and server; GTK is not required.
cargo build

# Build the desktop app.
cargo build --features desktop --bin mirelay-desktop
```

## Visual checks

The hidden screenshot options run a separate instance and disable automatic receive. Use an isolated registry for previews:

```bash
target/debug/mirelay-desktop --registry /tmp/mirelay-desktop/folders.toml \
  --screenshot /tmp/mirelay-folder.png --screenshot-width 820

target/debug/mirelay-desktop --registry /tmp/mirelay-desktop/folders.toml \
  --screenshot /tmp/mirelay-settings.png --screenshot-settings
```

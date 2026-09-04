# MiRelay for Linux

`mirelay-desktop` is a native GTK 4 frontend built on the existing Rust receive core. The CLI and desktop app share the same configuration, state, content-addressed storage, HTTP retry, verification, acknowledgement, and wallpaper state machine. There is no second transfer implementation.

## Build and run

GTK 4 and libadwaita development libraries are required. On Debian or Ubuntu:

```bash
sudo apt install libgtk-4-dev libadwaita-1-dev
cargo build --release --features desktop --bin mirelay-desktop
```

Run the development build:

```bash
cargo run --features desktop --bin mirelay-desktop
```

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

- A Folder is the primary managed object and represents one local delivery workflow.
- The sidebar lists Folders. Selecting one shows its properties and activity on the right.
- Folder properties include its local path, source, device ID, file-size limit, and wallpaper workflow.
- Recent transfers belong to the selected Folder rather than global navigation.
- Regular files, wallpaper images, pending acknowledgements, and failures use the same compact status model.
- Receive work runs in the background, with manual refresh and an action to open the selected Folder.
- You can add, rename, switch, and remove multiple Folders. Removing one does not delete its configuration, state, or received files.
- Each Folder has its own managed path and HTTPS or trusted-local-HTTP connection settings.
- Automatic Receive is opt-in per Folder and checks every 60 seconds while the app is open.
- `Ctrl+R` receives files and `Ctrl+,` opens Folder Settings.

Each Folder continues to use a standard Rust CLI configuration, so the CLI and desktop app can operate on the same Folder. The desktop registry only maintains the Folder list, display names, and current selection. A broken Folder is isolated from healthy ones. MiRelay rejects overlapping managed paths, shared state files, reused device identities, inbox conflicts, and Folders that would compete for the same HTTP receive queue.

Network synchronization runs on a worker thread while the GTK main thread handles rendering and input. Actions are disabled until a running task finishes. Cross-process state locks still prevent the CLI and desktop app from updating the same state concurrently.

Automatic Receive is disabled by default, including for configurations imported from the CLI. Enable it in Folder Settings. Each interval schedules all enabled, healthy Folders in one background worker and isolates a failure in one Folder from the others. If the app is already loading or receiving, that interval is skipped rather than starting overlapping work.

## Token boundary

Configuration files only store the environment-variable name, never the bearer token. If Session Token is empty in Folder Settings, the desktop app reads that environment variable. A token entered manually stays in process memory and must be entered again after the app closes. Secret Service or another system keyring can be added after the desktop MVP.

## Build isolation

GUI dependencies are behind the `desktop` feature:

```bash
# Build only the CLI and server; GTK is not required.
cargo build

# Build the desktop app.
cargo build --features desktop --bin mirelay-desktop
```

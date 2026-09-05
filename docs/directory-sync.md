# Directory sync — protocol and Rust receiver

Status: experimental. The Rust CLI foundation is now connected to the Android
development app's explicit directory mode; see [Android setup and QA](android-directory-sync.md).
Existing delivery-only Auto is retained. The [Linux desktop Folder UI](desktop-directory-sync.md)
now supports explicit directory initialization and receive. No physical-phone install or VPS deployment was
performed for this work.

## Behavior

- Reuse a confirmed, scoped Folder: sender and receiver credentials stay separate.
- Linux publishes a SHA-256 inventory of its existing destination directory.
- Sender previews identical, missing, different and destination-only files.
  Already-in-flight source versions are included in the comparison.
- Initialization requires explicit confirmation and rechecks both inventories
  and pending versions. A changed preview is rejected without uploading.
- Subsequent sends detect new or changed content. Original UTF-8 filenames and
  subdirectory paths are retained; equal bytes at two paths remain two files.
- Each source update gets a durable increasing version. Staged payloads and tus
  offsets survive process restarts. A late old upload cannot resurrect a newer
  acknowledged version.
- Upload completion means **stored on the relay**, not delivered to Linux.
  Directory receipts are sent only after verified, durable local projection and
  a durable ledger. A missing/changed projection before receipt blocks the ACK.
- No source deletion, destination deletion, or destination-only cleanup occurs.
  Rename is currently a new path; the old destination path remains.

The receiving file is updated to the incoming version. Before replacing an
existing file, an atomic exchange preserves the previous inode in a sibling
`.mirelay-history-<UUID>` file. This also retains the previous copy for ordinary
updates. A differing, untracked or locally edited previous copy is marked as a
conflict. `receive` prints the history path and conflict flag in its result.
There is no automatic merge or conflict-resolution UI yet.

## Isolated CLI workflow

Build with `cargo build --locked --bin mirelay-directory --bin mirelay-server`.
The directory CLI currently requires Linux/glibc and a filesystem supporting
atomic `renameat2` exchange/no-replace. It does not fall back to unsafe overwrite.
The server remains independently buildable, including its existing musl target.

First create, claim and confirm a **test** Folder using the existing pairing
workflow. Keep the directory state outside, and not an ancestor of, the selected
directory. Both selected directories must already exist. Credentials below are
read from environment variables, never from URL parameters or saved state.

On the receiving side, set `MIRELAY_TOKEN` to the Folder receiver credential:

```bash
mirelay-directory --directory /absolute/test-destination \
  --state-dir /absolute/test-receiver-state \
  --server-url https://relay.example/f/FOLDER_UUID receive
```

On the sending side, use the Folder sender credential:

```bash
mirelay-directory --directory /absolute/test-source \
  --state-dir /absolute/test-sender-state \
  --server-url https://relay.example/f/FOLDER_UUID preview

# Review the counts and paths first. This commits consent, but uploads nothing.
mirelay-directory --directory /absolute/test-source \
  --state-dir /absolute/test-sender-state \
  --server-url https://relay.example/f/FOLDER_UUID initialize --confirm

mirelay-directory --directory /absolute/test-source \
  --state-dir /absolute/test-sender-state \
  --server-url https://relay.example/f/FOLDER_UUID send
```

Run receiver `receive` again to download/acknowledge. Sender `status`, using the
same global options, shows `acknowledged` and `conflict`. Run `send` again for
later changes or to resume after a network failure. These commands are one-shot;
there is no new background watcher or scheduler in this stage. Plain HTTP needs
explicit `--allow-insecure-http` and is only for an isolated local test network.

Low-level tus testing also supports `mirelay-upload --relative-path PATH
--source-version NUMBER`. Both options are required together; resumed uploads
cannot change either. The higher-level sender allocates these versions itself.

## Protocol and compatibility

New scoped endpoints, all requiring a ready pairing and protocol header:

| Endpoint | Permission | Purpose |
| --- | --- | --- |
| `GET /f/{id}/api/v1/directory` | sender or receiver | Inventory, latest path versions and receipts |
| `PUT /f/{id}/api/v1/directory/index` | receiver | Compare-and-swap receiver inventory |
| `POST /f/{id}/api/v1/directory/ack` | receiver | Version + SHA-256 receipt, with conflict flag |

Directory tus metadata adds base64-encoded `relative_path` and `source_version`
to the existing filename, media type and SHA-256 fields. Delivery publication
and path/version association commit in the same SQLite transaction. Directory
deliveries are excluded from legacy pending lists; legacy ACK cannot acknowledge
them. Existing uploads and their resume records remain compatible without the
new optional metadata.

The server database migrates from schema 1/2 to **3**. Existing queues and Folder
credentials remain intact. The migration has isolated tests, not a production
upgrade rehearsal. Back up the database, data and old binary before any deployment;
an older server binary cannot read schema 3. The live VPS has not been migrated.

## Limits and safety boundaries

- Files must be nonempty and at most 100 MiB, subject to a smaller server limit.
- At most 5,000 paths in a source ledger/server Folder; historical deleted paths
  count toward this limit. At most 5,000 files in an inventory, 16 directory levels,
  1,024 UTF-8 bytes per relative path and 255 per component.
- Full-content inventory scans are bounded to 4 GiB total file bytes, 10,000
  visited entries and a 90-second checked traversal budget. This is not an
  interruptible deadline for an individual blocked filesystem operation.
- Inventory/metadata state is capped at 8 MiB; exceptionally long names can reach
  this before the file-count limit. Exceeding a limit fails rather than accepting
  a partial inventory.
- No followed symlinks, special files, traversal, file/parent collisions or silent
  path normalization. Colon and backslash components are rejected. `.mirelay*`
  names are reserved and excluded from scans. Empty directories, permissions,
  timestamps, ACLs and links are not synchronized.
- Receiver operations are anchored to open directory descriptors, with parent
  inode checks during recovery. This does not make a user-owned filesystem a
  sandbox against its owner moving directories concurrently.
- Keep state and source data. Do not reassign an existing state directory to a
  different Folder/root. There is no automatic rebind, state migration UI or
  repair of an acknowledged file deleted later on the receiving machine.
- Local verified-object cache and retained history have **no automatic retention
  cleanup yet**. They can grow with updates. Watch disk space during testing;
  do not enable unattended production use.
- Transport protections reuse the existing HTTPS and scoped-token model. This
  change does not add end-to-end encryption or hide filenames from the relay.

## Validation

The results below record the initial Rust-only stage. Subsequent Android directory
workflow tests and current mobile results are in [Android directory sync](android-directory-sync.md#验证).

```bash
cargo test --locked --features desktop --lib --tests -j2 -- --test-threads=2
cargo clippy --locked --features desktop --all-targets -- -D warnings
cargo test --locked --test directory_sync \
  inventory_5000_files_is_bounded_stable_and_rejects_overflow -- --exact --nocapture
bash android/scripts/build.sh :app:assembleDebug :app:testDebugUnitTest :app:lintDebug
```

New coverage: 13 integration tests, five persisted recovery-boundary tests and
one path-validation unit test. Includes real local HTTP/tus and CLI subprocess
round trips, initial consent/stale previews, existing nested directories and a
64 KiB arbitrary binary payload,
same-content/different-path handling, edits/conflicts/deletion non-propagation,
restart catch-up, mutable resume metadata rejection, out-of-order versions,
receipt retries, cross-Folder credentials/roles, >16 KiB valid inventories,
oversize rejection, path/symlink guards and schema 2→3 migration.

Validation on this implementation: Rust desktop-enabled regression **184 passed,
0 failed, 6 ignored**; Clippy with warnings denied passed. The ignored entries are
four graphical-session checks, one explicit JNI smoke test and the process-crash
child helper (the parent recovery tests did execute). Android ARM64/x86_64 native
libraries and debug APK built; lint passed, and a forced rerun of the existing
JVM suite passed **25 tests**. This checks existing Android compatibility, not the
new directory-sync mobile workflow. No new emulator or physical-phone app test
was performed in this Rust-only stage. Android build still reports its existing
SDK XML tools-version warning. The x86_64 musl release server also built
successfully; the resulting binary was not deployed.

Recovery tests reconstruct persisted states before exchange, after exchange,
after archiving, before new-file rename and after new-file rename. They are not
power-cut, disk-full or physical-device fault tests. Existing general process
recovery tests also run in the Rust regression suite.

Local debug-build small-file benchmark: 5,000 × 1 KiB files (4.88 MiB) scanned and
SHA-256 hashed in **172 ms** in one run, with a second identical scan checked for
stable identity. This is a warm/local-filesystem baseline, not an Android, WAN,
large-file throughput or peak-memory result.

## Next stage

Android SAF inventory, explicit preview/confirmation, modification detection and
receipt display are implemented in the development app. Linux Folder settings,
directory receive and retained-copy display are also implemented. A real phone/server
upgrade needs separate deployment approval and backups; the existing installed
phone Auto has **not** been converted into directory synchronization.

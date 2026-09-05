# Android Auto — implementation and verification, 2026-09-05

## Behavior

Each destination Folder now has an English **Auto** entry. Choosing a source directory only fills a draft. Explicit **Enable Auto** obtains a persisted, read-only SAF tree grant, completes a recursive baseline, and schedules work. Existing files are never uploaded by that baseline. Manual choosing/sharing remains available.

Periodic WorkManager checks run approximately every 30 minutes, subject to Android scheduling. The default constraints are unmetered connectivity, battery-not-low and storage-not-low. There is no perpetual watcher, foreground scan service, alarm or gallery-wide permission. An actual upload still uses the existing foreground transfer notification. **Check now** schedules the same constrained scanner; it does not bypass the scheduler.

New document identities must have stable size, modification time and name on two observations separated by at least 10 seconds. One bounded stability follow-up is allowed per periodic/requested check. Metadata is rechecked before and after copying to private, fsynced storage. SHA-256 deduplication, the discovery ledger and the transfer record commit in one SQLite transaction. The SQLite v1 → v2 migration is additive and preserves Folders, encrypted tokens and upload receipts.

**Pause Auto** atomically disables its consent revision and invalidates queued/running automatic upload ownership before cancelling tagged work. It does not stop manual transfers. Re-enabling creates a fresh baseline, skipping files added while paused, and does not silently resume previously paused files. Explicitly resuming a file makes it manual, with normal connectivity settings. Pausing individual files also wins against delayed enqueue/recovery calls.

Revoked source permission is checked during scans, before automatic uploads, and between upload chunks. Detection pauses Auto. A failure from an obsolete scan revision cannot pause a newly configured source. Server-accepted data cannot be recalled; the current HTTP request may finish after a pause/revocation.

## Scope and limits

- A maximum of 5,000 total entries per directory tree; directory depth 16; discovery history 50,000 identities per Folder baseline.
- A scan has a 90-second cancellation deadline and stages at most 20 files / 100 MiB. Each file must be nonempty and at most 100 MiB. Larger backlogs wait for later checks.
- Loading/error/duplicate/cyclic/over-limit listings cannot partially enable Auto. Virtual or missing-size/time documents require manual attention. Files that disappear and return must settle again.
- Deduplication covers automatic content within a destination Folder, not manual sends or the contents of baseline files. Baseline establishment reads metadata, not every historical file's bytes.
- Editing an existing/baseline/sent document is not treated as a new file. No deletion mirroring, directory hierarchy replication, instant synchronization or event-trigger acceleration is implemented.
- Force-stop prevents automatic execution until the app is reopened. Normal process death is different: WorkManager persists tasks, while app startup repairs committed-but-unscheduled uploads.
- The deadline uses cancellation signals and closes active streams; arbitrary remote providers can still ignore or delay cancellation. Stable metadata is not proof against a provider that changes bytes while lying about metadata. Such provider behavior is not certified here.
- Source reads never delete originals. Failed/paused staged payloads are retained for resumption. Process death before queue commit can leave private orphan staging data; automatic reclamation and storage-management UI remain follow-up work.
- Uploaded still means verified by the server, not received by Linux. Sender-side Linux receipts remain outside this change.

## Implementation

`AutoStore.kt` owns consent revisions, baseline/candidate state and atomic deduplication. `DirectorySource.kt` provides bounded SAF traversal and cancellable metadata reads. `AutoCoordinator.kt` handles consent, scheduling and scan/stage orchestration; `AutoScanWorker.kt` is the system-scheduled entry point. Existing importer/upload/queue classes now support automatic provenance, constraints and pause guards. The new controls are in `RelayScreen.kt`, with the system picker wired through `MainActivity.kt` and `RelayViewModel.kt`.

No server/Rust/Linux implementation was changed in this Auto round. No production VPS or real phone/files were used. Existing unrelated worktree changes were preserved; no commit or push was performed.

## Verification

The final full regression run exits **0**. Evidence is retained under **`android/.local/device-qa-tpbdrzai/`**, with the APK hash recorded in `results.json`.

| Verification | Result |
| --- | --- |
| JVM input/SQLite/migration/concurrency | 21 passed (11 new Auto cases) |
| Actual Android runtime/Keystore/JNI | 13 passed |
| Actual SAF provider / Auto / WorkManager | 12 passed |
| Compose UI/share/layout/pixel | 21 passed (3 new Auto cases) |
| Notification denial / foreground upload | 1 passed |
| External SIGKILL during manual 16 MiB upload | Passed |
| External SIGKILL during automatic 16 MiB upload | Passed |
| Android → Rust server → Linux exact bytes, SHA-256 and ACK | 11 deliveries passed |
| Application/test APK build, v2 signature, 16 KiB zip alignment | Passed |
| Lint | 0 errors, 9 dependency-update warnings |
| Shell/Python syntax and source whitespace | Passed |

The **47 device cases** exclude the four setup/verification helper methods around the two host-orchestrated process kills. Both kills occurred after 1 MiB of a 16 MiB transfer; reopening retained the same Android transfer identity and completed all bytes with a durable receipt and payload cleanup. The automatic case additionally retained its read permission, consent revision and one periodic task. Linux independently verified all 11 deliveries; the server ended with `pending: 0`, `acknowledged: 11`.

Final captured logcat contains no Java fatal exception, native fatal signal, JNI abort or ANR matching the checked markers. This is evidence for the tested paths, not a crash-freedom guarantee. Original UI captures, including the Auto dialog and short-window controls, are in the report's `visual/` directory; they are diagnostic captures, not screenshot goldens.

The bounded 5,000-entry synthetic local-provider sample measured **292 ms to establish the baseline** and **235 ms for an unchanged scan**. These metadata-only debug-emulator timings do not represent remote providers, hashing/staging 5,000 files or real-device battery consumption.

A separate final observation is retained at `android/.local/device-qa-xhe4x99e/`: five process-cold starts took **930, 940, 871, 920, 953 ms**, median **930 ms**. A **10.07-second** idle sample consumed **0.05 CPU seconds**, about **0.5% of one core**. This sample used the post-test idle app with Auto paused, warm filesystem caches and software rendering; it is not an Auto-on battery-drain test. Theme/font/rotation settings were restored after diagnostic screenshots. No ANR is recorded in the final `lastanr.txt` snapshot.

An earlier complete run (`android/.local/device-qa-8gjywogj/`) also passed, before the final pause-before-enqueue guard and its test were added. Its 5,000-entry sample was 294/238 ms. The final APK was rebuilt and the entire suite rerun after that guard; the earlier passing APK is not being substituted for final verification.

Test coverage added in this round:

- 11 JVM/database cases: complete baseline, existing-file edits, stability window, backward clock, disappearance, unsupported metadata, concurrent enqueue, per-Folder hash deduplication, pause/manual isolation, stale revisions, bounded baseline, fresh re-enable and v1 migration.
- 12 device SAF/Auto cases exercise a plain-Java DocumentsProvider in a separate test APK: real read-only tree grants, recursive baseline, WorkManager constraints/unique registration, app-background discovery/upload, content deduplication, incomplete listings, depth/entry limits, 5,000-entry timing, source mutation during read, staging cleanup, unsupported metadata, permission revocation at scan/upload, cancellation, out-of-scope/write denial, queue recovery, and pause-before-enqueue/manual-resume.
- Three Compose cases cover the real directory-picker cancel flow, explicit Enable/Pause consent flow, and a short window with 1.5× fonts. Existing UI/share/transfer/pixel regressions remain.
- The host harness now kills and reopens the uploading process for both a manual and an automatic 16 MiB transfer. The automatic case also checks persisted permission/revision and unique periodic registration.
- The Rust server → Linux leg verifies each delivered fixture's exact bytes and SHA-256, then verifies all deliveries were ACKed.

## Artifact

- Debug APK: `android/app/build/outputs/apk/debug/app-debug.apk`
- Size: **23,880,624 bytes**
- SHA-256: **`af89d84e9143c2c186e3150198f9063701305914e8b93dd81f9e0e877a14eecc`**
- ARM64 and x86_64 native libraries are bundled. Runtime tests here use only x86_64.

The throwaway local servers and owned emulator were stopped at handoff to release resources. Toolchains, APKs, the AVD and ignored test reports remain available. No production deployment, commit or push was performed.

## Untested boundaries

This is Android 16/API 36 x86_64 debug-emulator verification, not a real-device guarantee. Natural 30-minute scheduling under Doze, reboot scheduling, extended idle/battery drain, low-battery/storage transitions, Wi-Fi/mobile changes, OEM restrictions, physical ARM64 devices and other Android releases still require device validation. Tests assert the actual periodic job interval/constraints and execute the same scanner using one-time WorkManager requests; they do not simulate a day of autonomous periodic operation.

The Android storage and scheduling choices follow [SAF directory access](https://developer.android.com/training/data-storage/shared/documents-files) and [WorkManager scheduling/constraints](https://developer.android.com/develop/background-work/background-tasks/persistent/getting-started/define-work). Those APIs allow deferred automatic work, not guaranteed instant background delivery.

## Reproduce

Only use the guarded repository-owned QA emulator; the runner clears its MiRelay test data.

```bash
bash android/scripts/emulator.sh start
# Another terminal, after boot:
bash android/scripts/emulator.sh check
bash android/scripts/build.sh :app:testDebugUnitTest :app:assembleDebug :app:assembleDebugAndroidTest :app:lintDebug
python3 android/scripts/device-tests.py
python3 android/scripts/device-tests.py --observe
bash android/scripts/emulator.sh stop
```

See [Android README](../android/README.md) for toolchain setup, user-facing behavior and APK location.

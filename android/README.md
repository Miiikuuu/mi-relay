# MiRelay for Android

Native Android sender MVP: Kotlin/Jetpack Compose for the platform UI and background scheduling, with the existing Rust tus sender shared through JNI. English UI, compact monochrome controls, and multiple **Folders** representing Linux destinations. No WebView, advertising, analytics, or third-party cloud service.

## Scope

The development app now includes opt-in **Directory sync** for paired Folders:
select an existing directory, preview both sides, explicitly initialize, then
system-schedule new and changed files with original relative paths. Linux receipt
and conflict status are displayed. The receiving side is available in both the
Linux GTK app and the experimental Rust directory CLI. A dedicated physical-phone
initialization test is recorded in the [test report](../docs/phone-directory-init-2026-09-05.md).
This remains a development build, not a production-readiness claim. See
[Android directory sync](../docs/android-directory-sync.md) for setup and limits.
The delivery-only workflow below remains available and is not silently converted.

Directory sync requires a complete scan; unsupported files are not silently skipped.
An empty, oversized, virtual or metadata-unavailable file now reports its relative
path, the reason and a recovery action. A source-level error counts toward **need
attention**, separately identified from file issues. Fix the source, then use
**Check now** or allow the existing bounded retry to run. A successful check clears
the source error without resetting pairing or directory history.
See the [diagnostics and database concurrency regression report](../docs/android-source-diagnostics-2026-09-08.md).

- Android 8.0+ (API 26), ARM64 phones and x86_64 emulator builds; compile/target SDK 36.
- Add/select Folders using a Linux-generated pairing code and server URL; the server now isolates each Folder and its sender/receiver permissions. **Choose existing directory** initializes a directory in place during creation. History sending is explicit and unchecked by default; Auto stays off until enabled after pairing. See [Folder setup and pairing](../docs/folder-pairing.md). Legacy device-token connections remain available.
- Choose files with Android's document picker or receive single/multiple file shares. Confirm the destination before a manual upload. Optional, explicitly enabled **Auto** sends new files from a selected directory. Text-only shares are not yet supported.
- Copy selected content into app-private, no-backup storage; preserve originals. Stream the copy with a **100 MiB/file** limit, up to **20 files/selection**. Empty files are rejected by the current server protocol.
- Rust performs content detection, SHA-256, tus creation/HEAD/PATCH, offset/metadata validation, and same-origin checks. The platform-provided MIME/extension is not trusted for protocol metadata.
- WorkManager persists tasks, waits for connectivity, shows a foreground notification, and retries transient failures with exponential backoff (up to five automatic attempts). Pause/Resume retains the same staged payload and tus session. Pause takes effect between HTTP requests, not necessarily immediately; request timeout is 30 seconds.
- Persist a local upload receipt before removing staged bytes or completed tus state. A restart before receipt persistence can replay HEAD without enqueueing another delivery. Stale workers cannot overwrite a replacement task's state.
- Encrypt bearer tokens with Android Keystore AES-GCM, bind ciphertext to the Folder ID, and exclude app data from cloud backup/device transfer. WorkManager input contains only task/Folder IDs, consent revision and scheduling flags, not credentials, paths, or file content.

**Uploaded means the server accepted the complete, verified object, not that Linux received it.** Directory-sync files distinguish **Waiting for Linux**, **Synced**, **Synced · conflict copy kept** and **Superseded** using the scoped receipt API. Delivery-only files still show **Uploaded** without a Linux receipt. Linux independently verifies, stores and ACKs each received version.

Folder server URLs are immutable after creation to prevent retargeting existing tasks. Names and tokens can be updated; leave the new-token field blank to retain the existing one. A different server needs a new Folder.

## Delivery Auto: original new-file sending

The following section describes delivery-only Auto. In a paired Folder's new
directory setup, choose **Use delivery Auto** to select this original behavior.
Directory sync preserves its baseline/version history on resume and also detects
modifications; it does not use delivery-only cross-path content deduplication.

Open a destination Folder → **Auto** → **Choose source directory** → **Enable Auto**. The system picker grants access only to that directory and its descendants; MiRelay retains **read permission only**. Selecting a directory alone does not enable sending. No all-files permission, gallery-wide permission, permanent listener, alarm, or always-on foreground service is added.

- A complete initial scan records a baseline without uploading existing files. An incomplete/loading/error/cyclic/oversized listing fails closed. Only subsequently discovered document identities are candidates; edits to baseline or already-sent files are not synchronized.
- WorkManager checks approximately every **30 minutes**, starting after the baseline. This is not an exact interval: Android may defer execution. **Check now** requests an earlier system-scheduled check, still subject to constraints.
- The default is an **unmetered** connected network, with battery-not-low and storage-not-low requirements. Unmetered is not identical to Wi-Fi. Disabling that checkbox allows metered networks; pause and re-enable to change it.
- A new file needs two observations with unchanged name, size and modification time at least **10 seconds** apart. Each periodic/requested check can schedule at most one delayed stability follow-up; there is no polling loop. Metadata is checked again before and after private staging. Unreliable metadata is a provider limitation, not a filesystem transaction guarantee.
- SHA-256 content deduplication, the discovery ledger and transfer insertion commit atomically in SQLite. Identical automatic content is sent once per destination Folder, including across Auto re-enables. Manual sending is independent and can deliberately resend a file. Baseline files are not read/hashed just to establish the baseline.
- Scan bounds: **5,000 total entries**, directory depth **16**, discovery history **50,000 identities**, a **90-second** cancellation deadline, and at most **20 files / 100 MiB total staged bytes per scan**. Each file must be **nonempty and at most 100 MiB**. More candidates wait for subsequent checks. Virtual, empty, oversized, or unknown-size/time documents need manual attention.
- **Pause Auto** invalidates scan/upload ownership and cancels that Folder's automatic jobs; manual jobs continue. Private paused payloads remain available. Manually resuming one makes it a manual transfer with normal connectivity rules. Pausing cannot undo data already received by the server, and a current HTTP request may finish.
- Re-enabling creates a **fresh baseline**, so files added while paused are skipped. Existing paused transfers are not silently resumed. Revoked directory permission disables Auto when the next scan/upload access check notices it; staged automatic uploads also check permission before starting and between chunks.
- No continuous process is required. During an actual upload, the existing foreground transfer notification may appear. WorkManager persists jobs across process death and normal reboot; opening the app also repairs a queue committed before WorkManager enqueue. **Force-stop blocks automatic execution until the app is reopened.**

The database migration from version 1 to 2 preserves existing Folders, encrypted credentials and transfer receipts. Event-triggered acceleration, deletion mirroring, resending edits, and instant synchronization are intentionally not part of this first Auto version. See the [Auto implementation and verification report](../docs/android-auto-2026-09-05.md).

## Build

The supplied MiRelay icon is used by the adaptive launcher and Folder drawer;
the welcome screen displays the complete wordmark. White containers preserve the
original opaque artwork in both themes. Notifications retain a separate symbolic
transfer icon. Original assets, attribution and integration notes are in
[the brand directory](../assets/brand/README.md).

Linux prerequisites: Rust/rustup, curl, jq, unzip, tar, CMake, and a host C/C++ toolchain. The optional bootstrap downloads a repository-local JDK 17, Gradle 8.13, Android SDK 36/build-tools 35, NDK r27c, platform-tools, and installs the ARM64/x86_64 Rust targets. Generated assets, SDK, caches, and debug keys are ignored by Git. It does not edit shell startup files or the existing desktop environment.

Read the [Android SDK license](https://developer.android.com/studio#terms-and-conditions), then explicitly opt in:

```bash
bash android/scripts/bootstrap.sh --accept-sdk-license

# Optional when Google downloads work better without your configured proxy:
MIRELAY_ANDROID_DIRECT_DOWNLOADS=1 bash android/scripts/bootstrap.sh --accept-sdk-license

bash android/scripts/build.sh :app:assembleDebug :app:testDebugUnitTest :app:lintDebug
```

APK output: `android/app/build/outputs/apk/debug/app-debug.apk`. The build script compiles both native ABIs, sets 16 KiB ELF alignment, and bundles the Kotlin certificate-verifier component matched by `android/native/Cargo.lock`. Android certificate verification is initialized before Rust HTTPS requests; it is never disabled.

The pinned AGP/Gradle/JDK combination follows the [AGP 8.13 compatibility notes](https://developer.android.com/build/releases/agp-8-13-0-release-notes). Open `android/` in Android Studio after running the native build script; rebuilding Kotlin alone does not refresh the Rust libraries. The initial APK is a debug/development build, not a signed distribution release.

Production builds require HTTPS. Only debug builds expose explicit trusted-network HTTP opt-in, enforced again by the native request configuration. Do not use development HTTP over an untrusted network.

## Tests

For the first APK's exact verification results and untested boundaries, see the [2026-09-05 Android MVP report](../docs/android-mvp-2026-09-05.md).

```bash
CARGO_BUILD_JOBS=1 cargo test --locked --features desktop --all-targets

# Actual host JVM → JNI → Rust input/callback checks.
bash android/scripts/test-jni.sh

# Actual host JVM → JNI → Rust → local tus server → Linux receive/ACK.
# Text and unknown binary, pause/resume, receipt replay, callback exception.
CARGO_BUILD_JOBS=1 cargo test --locked --test android_jni -- --ignored --nocapture
```

The JNI test runs on a **Linux JVM**, not Android ART. `tests/upload_api.rs` covers mobile-facing API guards, pause/resume, completed-session replay, source mutation and bad authentication. Android JVM tests cover input rules and SQLite task ownership/persistence. The fake token cipher used by database tests is test-only; it does not certify Android Keystore behavior.

## Local Android emulator and device tests

The repository now has a dedicated Android 16/API 36 x86_64 AVD named `MiRelay_API36_QA`, separate from physical phones and other AVDs. It uses KVM, two virtual CPUs, 2,560 MiB guest RAM, a 720×1280 display, software graphics, and no usage-metrics reporting. Host RSS is higher than guest RAM (about 4 GiB during the first run). SDK/system images, AVD data and test reports stay under ignored `android/.local/`.

The device runner also requires Python 3, OpenSSL, and the host CLI/server binaries. Use `MIRELAY_EMULATOR_GPU=swangle` before the start command to compare the alternate ANGLE/software renderer when diagnosing rendering artifacts; the default is `swiftshader`.

```bash
# Requires the base toolchain above and explicit SDK-license acceptance.
bash android/scripts/emulator.sh setup --accept-sdk-license

# Keep this running in its own terminal. Use --window for an interactive window.
bash android/scripts/emulator.sh start

# In another terminal, once Android has booted:
bash android/scripts/emulator.sh check
CARGO_BUILD_JOBS=1 cargo build --locked --bins
bash android/scripts/build.sh :app:assembleDebug :app:assembleDebugAndroidTest
python3 android/scripts/device-tests.py

# Optional bounded startup/idle samples and dark/landscape/large-font screenshots:
python3 android/scripts/device-tests.py --observe

# Stop only this emulator, preserving its installed apps and disk data:
bash android/scripts/emulator.sh stop
```

**The device test runner clears MiRelay's app data on this dedicated test AVD. Do not configure real Folders there.** It checks the AVD identity before any install/reset operation. Instrumented tests additionally reject physical-device hardware and require an explicit isolation marker. All upload fixtures and credentials are synthetic; local services listen only on loopback ports 18080–18082, reached from Android via `10.0.2.2`. No production server is used. Tests deliberately kill only MiRelay's test process during upload.

The runner prints its report directory, including instrumentation results, HTTP event metadata, screenshots, logcat, SQLite recovery snapshots, and Linux receive/ACK verification. The fake external provider is in the **test APK only**, implemented in plain Java so its separate process does not depend on the target APK's Kotlin runtime. Tests execute real Android Keystore, SQLite, WorkManager and JNI/ART behavior; these are not Robolectric substitutes.

See the [UI fix and retest report](../docs/android-ui-fixes-2026-09-05.md) for the current layout fixes and results; the [initial emulator QA report](../docs/android-emulator-qa-2026-09-05.md) preserves pre-fix findings. The device UI tests include landscape, large fonts, short windows, long Folder/file lists, and actual button-text screen pixels. Original screenshots are collected under each report's `visual/` directory. A nonzero test-runner exit must not be ignored. `--observe` captures diagnostic samples but does not assert visual correctness; inspect original image data when a preview appears suspicious. Run the base suite first, since observation preserves the current test data.

## Remaining device validation / limitations

### Read-only physical-phone monitoring

For a phone the owner has explicitly authorized through USB debugging, use the
separate diagnostic collector, **not** the emulator test/reset runner:

```bash
android/.local/sdk/platform-tools/adb devices -l
python3 android/scripts/device-monitor.py --serial YOUR_AUTHORIZED_SERIAL --minutes 30
```

This collector neither installs nor launches apps. It records only MiRelay's UID
logs (info and above, including process restarts) and samples its main-process
memory/PID every 15 seconds. It stops at the chosen duration, on USB/authorization
loss, when the app identity changes, at a 20 MiB log cap, or with Ctrl+C. Reports
are private, Git-ignored `android/.local/phone-monitor-*` directories. They can
contain personal app log content; inspect/redact before sharing. Log collection
is not a guarantee that every failure is logged, and a surviving process does
not prove the UI or transfer pipeline is healthy. The collector does not upload
diagnostics, read transferred files, change settings or clear phone/app data.

Keep USB/ADB overhead and charging in mind when interpreting measurements. This
is a bounded diagnostic recording, not a battery-drain benchmark or a guarantee
of continuous agent supervision after a conversation ends.

### Validation still needed

- The dedicated emulator now covers installation, selected Compose/share flows, Keystore, foreground WorkManager, untrusted HTTPS certificate rejection, and one upload-process-death scenario. Physical ARM64 devices, other Android versions, successful trusted HTTPS uploads, real Wi-Fi/mobile-network switching, OEM battery restrictions, and broader UI/accessibility coverage still need validation. A passing emulator run is not a real-device certification.
- Android can delay background work. Force-stop prevents automatic execution until the app is reopened. Long-running foreground WorkManager tasks are subject to [Android scheduling limits](https://developer.android.com/develop/background-work/background-tasks/persistent/how-to/long-running); no perpetual-background guarantee is made.
- File providers may revoke or block access during staging. A failed copy does not enqueue partial content. After staging commits, resuming no longer depends on the original provider URI.
- Paused/failed payloads remain private so they can resume. A process killed mid-import can leave unregistered staging data; automatic orphan reclamation, a storage-management screen, and deletion/retention controls remain follow-up work. The server does not advertise tus termination.
- No event-driven folder watching, automatic gallery-wide scanning, text/URL conversion, exhaustive Android fault injection, release performance certification, or 32-bit ABI support yet. Linux receipts are available for directory sync, not delivery-only Folders. Auto currently uses periodic SAF directory scans, not provider change notifications.

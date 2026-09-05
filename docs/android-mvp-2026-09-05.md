# Android sender MVP — implementation and verification, 2026-09-05

This is the initial build/host-test report. Subsequent actual Android emulator testing is recorded in [the emulator QA report](android-emulator-qa-2026-09-05.md); its findings supersede the initial untested-device status below.

## Result

The first Android sender development APK builds successfully. Kotlin/Compose handles the UI, Android file providers, encrypted credentials, and persistent WorkManager scheduling. Uploads use the existing Rust tus implementation through JNI, not a second protocol implementation.

The UI is English, monochrome, and organized by **Folder**. It supports choosing or sharing files, confirming the destination, active-first transfer rows, progress, pause/resume, retry, and persistent upload receipts. Uploaded means **accepted by the server**, not delivered to Linux; sender-side Linux receipts are not implemented.

This is a development handoff, **not a real-device or release certification**. No Android device/emulator was installed or launched for this verification. No production server, real token, or existing user Folder was changed.

## Artifact

- Path: `android/app/build/outputs/apk/debug/app-debug.apk`
- Application: `io.mirelay.android`, version `0.1.0-dev` (1)
- Minimum API 26; target/compile API 36
- Native ABIs: `arm64-v8a`, `x86_64`
- Size: **23,831,494 bytes** (approximately 22.7 MiB)
- SHA-256: `a31796151498e961e2cc5e7d0956c12683abc401037ed67851b2225c7d3d8786`
- Debug APK signature verification passed (v2). This is not a production signing identity.
- `zipalign -c -P 16 4` passed; both Rust shared libraries have ELF LOAD alignment `0x4000`. These are packaging checks, not a 16 KiB-device runtime test.
- Merged APK requests network, notification, foreground data-sync, wake-lock, and boot-completed permissions, plus WorkManager's app-local receiver permission. It does not request gallery access or broad external-storage access.

SDK/NDK, caches, debug keys, generated libraries, and APK outputs are ignored by Git. Bootstrap uses repository-local tooling; Rust targets are installed through rustup. Builds use one Cargo job and one Gradle worker to bound build concurrency.

## Verification

| Check | Result |
| --- | --- |
| Rust regular suite, desktop feature and all targets | 150 passed, 0 failed |
| Host JVM → JNI → Rust → local tus server → Linux receive/ACK | 1 opt-in integration test passed |
| Direct host JNI malformed/oversized input smoke checks | Passed |
| Android JVM tests | 10 passed, 0 failed, 0 skipped |
| Android debug APK assembly | Passed, including both native ABIs |
| Android lint | 0 errors; 7 dependency-update warnings |
| Strict Rust Clippy, main project and native crate | Passed |
| Rust formatting, shell syntax, tracked diff whitespace | Passed |

The regular Rust run excludes four graphical diagnostics, one subprocess helper, and the separately run JNI test. Previous Linux graphical results are documented separately in [Linux QA](qa-fixes-2026-09-05.md); they were not rerun or counted here. JNI smoke assertions are not counted as extra JUnit/Rust test cases.

### Covered behaviors

- Shared Rust upload API: invalid options/names, failed authentication without automatic retry, staged payload mutation, pause at a known tus offset, resuming the same session, completed-session receipt replay without duplicate enqueue, and token-free resume state.
- Host JNI round trip: UTF-8 text with a Chinese filename and unknown binary data; progress callback exception propagation; pause/resume; completion replay; Linux byte-for-byte verification; two successful ACKs and an empty server queue afterward. It uses a temporary **HTTP** loopback server, not Android HTTPS or the remote VPS.
- Android input rules: release HTTPS enforcement, explicit debug HTTP opt-in, rejecting URL credentials/query/fragment/invalid host/port, token bounds/control characters, path/control-character filename sanitization, and UTF-8 byte limits.
- Android SQLite: credential separation from UI models, stale worker ownership, pause/replacement races, completed receipt persistence after reopening, preventing completed-task requeue, immutable server destinations, token replacement, and staging-path traversal guards.

The five SQLite tests run under Robolectric API 35 on JDK 17 with a **test-only fake token cipher**. They do not validate Android Keystore encryption. Robolectric's API 36/JDK 21 warning does not indicate skipped tests: the test XML confirms all ten ran. A first test attempt could not download its runtime because Robolectric did not inherit the Gradle proxy; the download configuration was corrected and tests were rerun successfully. The remaining build-tool SDK XML parser warning is nonfatal.

## Safety boundaries and remaining work

- Files are staged in private no-backup storage before upload; originals are not moved or deleted. Current limits are 100 MiB per file and 20 files per selection; empty files are rejected by the existing protocol.
- Payload bytes are flushed and renamed before the database entry is committed. The server receipt is persisted before deleting staged content or completed tus state. Pausing invalidates worker ownership before cancellation; native requests may take up to the configured 30-second timeout to return.
- Failed/paused files are retained for resume. A process killed during import may leave unregistered staging data; orphan reclamation and user-facing deletion/retention controls remain follow-up work.
- WorkManager retry and startup recovery are implemented, but actual Android process death, network changes, foreground-service restrictions, denied notifications, OEM battery management, and long-duration background behavior are **not yet device-tested**.
- Android ART/JNI loading, Android certificate verification, Keystore, system shares/file providers, installation, Compose appearance/accessibility, and configuration changes still need device/emulator validation. ELF and APK checks do not establish these runtime behaviors.
- No Android performance, memory, disk-full, power-loss, or long-running soak certification was performed. No automatic gallery scanning, folder watching, text/URL shares, Linux sender receipts, or 32-bit ABI is included.

## Reproduce

See [Android build instructions](../android/README.md) for prerequisites and the explicit SDK-license bootstrap step.

```bash
bash android/scripts/build.sh :app:assembleDebug :app:testDebugUnitTest :app:lintDebug
CARGO_BUILD_JOBS=1 cargo test --locked --features desktop --all-targets
bash android/scripts/test-jni.sh
CARGO_BUILD_JOBS=1 cargo test --locked --test android_jni -- --ignored --nocapture
CARGO_BUILD_JOBS=1 cargo clippy --locked --all-targets --features desktop -- -D warnings
CARGO_BUILD_JOBS=1 cargo clippy --locked --manifest-path android/native/Cargo.toml --all-targets -- -D warnings
cargo fmt --all -- --check
cargo fmt --manifest-path android/native/Cargo.toml -- --check
bash -n android/scripts/bootstrap.sh android/scripts/build.sh android/scripts/test-jni.sh
git diff --check
```

Local generated reports are under `android/app/build/test-results/testDebugUnitTest/` and `android/app/build/reports/lint-results-debug.html`.

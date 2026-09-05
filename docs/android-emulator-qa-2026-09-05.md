# Android emulator QA — 2026-09-05

**Follow-up correction:** the suspected missing-button-text observation below was a screenshot-preview interpretation error. Direct inspection of the original PNG pixels shows the complete label; the unchanged APK also passes the new screen-pixel assertions. See [the fix/retest report](android-ui-fixes-2026-09-05.md) for the landscape repair and subsequent results. This document preserves the original pre-fix test counts.

## Result

The local Android test environment is installed, boot-tested, and reusable. Actual Android/ART tests and the Android → Rust server → Linux receive/ACK chain now run from the repository.

**The App is not all-green:** 25 of 26 device test cases pass. The remaining case reproduces a real landscape-layout defect: an existing transfer disappears because the fixed header/footer leave no usable height for the file list. The test is retained and the runner correctly exits **1**. No production App code was changed to hide or fix this failure.

The separate upload-process-kill recovery scenario passes. The final valid run has no unexpected Java fatal exception, native fatal signal, JNI abort, or ANR in its captured logcat. This is a bounded emulator test, not a real-device, exhaustive-fault, or release certification.

## Environment and safety

- Host: Linux, Intel i7-13700H, approximately 14 GiB RAM; KVM version 12 is accessible without system/group changes.
- Android Emulator 37.1.11.0, build 15917651; Android 16 / API 36 default x86_64 image, revision 2. Actual guest page size is **4,096 bytes**; this does not certify 16 KiB runtime compatibility.
- Dedicated AVD: `MiRelay_API36_QA`; serial `emulator-5580`. Two virtual CPUs, 2,560 MiB guest RAM, 720×1280 pixels, density 320, software graphics, no snapshots/audio/cameras/usage metrics. Host emulator RSS was approximately 4 GiB, higher than guest RAM.
- SDK/images, AVD storage, generated APKs, credentials and reports are repository-local and Git-ignored. The Android SDK packages are installed; a full Android Studio IDE is not required.
- KVM, boot completion, ADB, guest storage, screenshot capture, APK installation, and an actual guest-to-host HTTP probe passed. The AVD had approximately 9.2 GiB available in `/data` at the final check.
- Only synthetic fixtures and test tokens were used. Services bind to host loopback ports 18080–18082; Android reaches them through `10.0.2.2`. The existing VPS, production data, real Folders, physical phones, and Linux wallpaper configuration were not touched.
- The runner checks the AVD name before installing or clearing MiRelay data. Instrumented test resets also require emulator hardware and an explicit isolation marker. **Running the suite clears this test AVD's MiRelay data.**
- Application APK SHA-256 remains `a31796151498e961e2cc5e7d0956c12683abc401037ed67851b2225c7d3d8786` (23,831,494 bytes), identical to the initial MVP build. Changes in this round are test infrastructure, test-only dependencies and documentation.

## Verification results

| Check | Final result |
| --- | --- |
| Device runtime/security/native tests | 13 passed |
| Device UI/share/worker tests | 11 passed, 1 failed (landscape) |
| Notifications denied, foreground upload | 1 passed |
| External SIGKILL during a 16 MiB upload, reopen and resume | Passed |
| Android → local Rust server → Linux | 7 deliveries verified and ACKed |
| Rust regular suite, desktop feature, all targets | 150 passed, 6 ignored |
| Opt-in host JVM → JNI → Rust → server → Linux integration | 1 passed separately |
| Android JVM input/SQLite tests | 10 passed, 0 failed/skipped |
| Debug application/test APK build | Passed |
| Android lint, including instrumentation sources | 0 errors, 9 dependency-update warnings |
| Shell syntax, Python compilation, tracked whitespace checks | Passed |

The 26 device cases comprise 13 runtime + 12 UI + 1 notification test. Recovery uses two helper test methods surrounding one externally orchestrated kill; these are not counted as two extra independent scenarios. The six ignored regular Rust tests include four graphical diagnostics, one subprocess helper, and the JNI integration test that was executed separately. Counts are not a claim of exhaustive feature or fault coverage.

### What ran on actual Android

- JNI loading and initialization on ART, malformed/oversized native inputs, Java callback exception propagation without native abort, and continued JNI usability.
- Real Android Keystore AES-GCM: randomized IVs, decrypting from a new vault instance, rejecting the wrong Folder binding and tampered ciphertext; encrypted credentials persisted in real SQLite.
- An external, test-only `ContentProvider` supplies deterministic bytes across Binder. Tests verify exact staged content, Unicode/path-safe names, empty data, denied/revoked access, non-content URIs, oversized declared length, and a provider claiming one byte while streaming 101 MiB. Rejected inputs do not enqueue a transfer.
- Native tus upload, pause at a known offset, resume and receipt replay; bad authentication is non-retryable and does not echo the token; unreachable services fail with bounded retryable errors. A self-signed HTTPS certificate is rejected by Android's actual certificate verifier; no verification bypass is used.
- Real Compose interaction: empty-state validation, creating/renaming Folders while retaining an unchanged token, Folder isolation, share destination confirmation/cancel, multi-file sharing, unsupported text shares, selection-count/busy guards, configuration recreation, and system document-picker open/cancel.
- Actual WorkManager execution and SQLite ownership guards: pause/resume, stale worker protection, durable receipts, staged-payload cleanup after completion, and automatic recovery from an injected first-request HTTP 503. Denied notification permission does not prevent the tested foreground upload.

### Process death and Linux delivery

The host kills only MiRelay's process with SIGKILL after a successful PATCH, while the persisted state is `UPLOADING`, **1,048,576 / 16,777,216 bytes**. Reopening the App resumes the same Android transfer ID, reaches `UPLOADED` at 16,777,216 bytes, retains a delivery receipt, and removes the staged payload. This is process death followed by reopening, not a guarantee of immediate background recovery after Android force-stop.

Linux then receives all seven completed deliveries from the actual Rust server. Every stored payload is compared byte-for-byte against its deterministic source pattern and independently checked with SHA-256. There are **five unique content-addressed objects for seven deliveries**, because two pairs intentionally contain identical bytes. All seven records are acknowledged; server status is `pending: 0`, `acknowledged: 7`. No wallpaper hook runs for these binary fixtures.

The device chain covers synthetic non-image binary data from 17 bytes through 16 MiB; it does not represent every document/media format. The separate host JNI chain also covers UTF-8 text with a Chinese filename and unknown binary data.

## Confirmed defect: landscape transfer list disappears

- Reproduction: create a Folder with an existing file, verify the file is visible in portrait, then rotate to 1280×720 landscape (640×360 dp before insets).
- Expected: the existing transfer remains visible or reachable by scrolling.
- Actual: the fixed header and footer consume the height; the file list has no usable viewport. This affects viewing/controlling transfers in short windows, not the verified payload integrity.
- Source: [`RelayScreen.kt`](../android/app/src/main/java/io/mirelay/android/RelayScreen.kt), the outer Column with fixed header/footer and weighted LazyColumn (around lines 99–125).
- Failing regression: [`DeviceUiTest.landscapeMustKeepExistingTransferVisible`](../android/app/src/androidTest/java/io/mirelay/android/DeviceUiTest.kt), assertion around line 178.
- Evidence: `android/.local/device-qa-cyuym4kp/DeviceUiTest.txt` and `android/.local/device-qa-6mfoujfz/landscape.png`.
- Suggested follow-up: make the complete content scrollable or adapt the header/footer to limited height, then rerun landscape, large-font and multi-window checks. The defect remains unfixed in this testing-only round.

The first suite had 25 passing cases; screenshot review exposed this previously untested defect. Adding its regression test produced the final 25/26 result without changing the production APK. This is improved coverage, not a newly introduced App regression.

## Performance and visual observations

Five process-cold starts (force-stop between starts, warm filesystem caches) were **911, 888, 1,021, 925, 938 ms**: median **925 ms**. A subsequent 10.06-second idle sample consumed 0.05 CPU seconds, approximately **0.5% of one CPU core**. App PSS moved from 70,599 to 69,315 KiB (approximately 69 to 68 MiB); RSS was approximately 193 MiB. No ANR was recorded in the observation sample.

These are debug-APK measurements on a software-rendered x86_64 emulator with a small transfer history. They do not establish release performance, sustained throughput, leak freedom, battery usage, frame-rate budgets, or real-phone startup speed. The captured gfxinfo frame sample is too small to report meaningful jank statistics.

Portrait, dark-theme, landscape and 1.3× font screenshots were captured. Their previews initially appeared to omit some button text/icons. **That interpretation was incorrect:** inspecting the original PNG pixels in the follow-up confirms the complete Choose files label in the previously cited captures. The original label region contains 1,074 bright pixels, with the same readable glyph pattern in both SwiftShader and ANGLE samples; it is not a blank button. Screen-pixel assertions also pass on the unchanged APK. There is no established missing-label App defect or evidence of a renderer fix here. The separate landscape failure is real and independently asserted by the device test.

The alternate-backend diagnostic completed boot/ADB checks and five launches (855, 695, 877, 775, 832 ms), but the full functional suite was run under the default SwiftShader configuration, not repeated under ANGLE. Its timings are a separate sample, not pooled into the baseline above. Theme, rotation and font settings were restored after observation.

## Harness issues resolved during setup

Early diagnostic attempts are retained in ignored local report folders, but are not mixed into the final App test counts:

- The external fixture provider originally used Kotlin in the separate test-APK process, where the runtime was unavailable. It is now plain Java and lives only in the test APK; that earlier crash was a fixture failure, not an App crash.
- `ActivityScenario` lost lifecycle tracking after the App changed its Activity intent for incoming shares. The UI harness now tracks the actual Activity lifecycle directly.
- The first large transfer through `adb reverse` lost the ADB transport while the App remained alive. Direct emulator-to-host networking through `10.0.2.2` completed repeated full runs.
- The network smoke probe now accounts for toybox `nc` exiting on stdin EOF. Free-port probes use `SO_REUSEADDR` so normal TCP TIME_WAIT does not masquerade as an occupied test service.

## Remaining coverage

Physical ARM64 phones, Android versions other than API 36, actual 16 KiB guests, successful trusted HTTPS uploads, real Wi-Fi/mobile switching, Doze/OEM battery restrictions, background-only/locked-screen notification scenarios, real third-party file providers, provider stalls, disk-full/power-loss, import-process-death orphan handling, accessibility/TalkBack, broader adaptive layouts, large histories and long-duration soak tests remain unverified. A generated test provider and loopback fault proxy cannot certify these behaviors.

## Reproduce and evidence

See [Android setup and test commands](../android/README.md#local-android-emulator-and-device-tests). Start the AVD in one terminal; build both the host binaries and Android APKs, then run `python3 android/scripts/device-tests.py` in another. The pre-fix revision tested here exited **1** because of the landscape defect; see the [follow-up report](android-ui-fixes-2026-09-05.md) for the repaired revision. `--observe` preserves data and captures diagnostics; it does not assert screenshots.

- Final functional/fault chain: `android/.local/device-qa-cyuym4kp/` (`results.json`, instrumentation logs, recovery snapshots, HTTP event metadata, logcat, Linux state and server ACK status).
- Default SwiftShader startup/idle/visual sample: `android/.local/device-qa-6mfoujfz/`.
- Alternate ANGLE/software-rendering observation: `android/.local/device-qa-7xk7ud1u/`.
- Earlier complete 25-case run before adding landscape regression: `android/.local/device-qa-bra4p2o6/`.
- JVM XML: `android/app/build/test-results/testDebugUnitTest/`; lint report: `android/app/build/reports/lint-results-debug.html`.

Generated screenshots, logs, test databases, temporary TLS keys and large synthetic payloads remain Git-ignored. Test servers and the owned emulator were stopped at handoff to release host resources; AVD data and installed tooling are preserved. Run `bash android/scripts/emulator.sh start --window` to reopen it interactively. No commit or push was performed.

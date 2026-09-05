# Android UI fixes and retest — 2026-09-05

## Result

The confirmed landscape defect is repaired, and the final complete device run exits **0**: **32/32 device cases pass**, alongside the separately orchestrated upload-process-death recovery and Android → server → Linux chain. The suspected missing-label observation was corrected after checking original pixels; it was not an App defect.

| Verification rerun in this round | Result |
| --- | --- |
| Actual Android runtime/security/JNI tests | 13 passed |
| Compose UI/share/worker/layout/pixel tests | 18 passed |
| Notifications denied, foreground upload | 1 passed |
| External SIGKILL during 16 MiB upload, reopen and resume | Passed |
| Android → local Rust server → Linux, exact bytes + SHA-256 + ACK | 7 deliveries passed |
| Android JVM input/SQLite tests | 10 passed, 0 failed/skipped |
| Debug application and instrumentation APK assembly | Passed |
| Lint | 0 errors; 9 dependency-update warnings |
| APK v2 signature and 16 KiB zip alignment checks | Passed |
| Shell syntax, Python compilation, tracked whitespace | Passed |

Final evidence: **`android/.local/device-qa-_xr9n1gl/`**. The retained 32-case count excludes the two setup/verification helper methods used around the single host-orchestrated process kill. Captured final logcat contains no unexpected Java fatal exception, native fatal signal, JNI abort, or ANR. This does not guarantee crash freedom outside the tested paths.

Linux verified seven deliveries backed by five unique content-addressed objects (duplicate content is intentionally shared). Server status finishes at `pending: 0`, `acknowledged: 7`. Recovery starts from 1 MiB persisted in an active 16 MiB upload and finishes with a durable receipt for that same Android transfer, with staged payload cleanup verified.

The broader Rust suite was not rerun for this Compose-only production change; its prior results are documented in the initial reports. The actual Rust-backed device upload and Linux receive/ACK chain **was** rerun. Physical ARM64 devices, other Android versions, real multi-window lifecycle behavior beyond reduced-window geometry, and release performance remain outside this retest.

## Changes

The confirmed landscape defect is fixed in [`RelayScreen.kt`](../android/app/src/main/java/io/mirelay/android/RelayScreen.kt). Previously, fixed header/footer siblings consumed the available height around a weighted file list. The screen now uses one LazyColumn for the header, transfers, empty state and receipt note. In wide, short windows, Folder details and Choose files share a compact horizontal header. Scrolling remains available when larger text or smaller windows need more space.

The welcome screen and share-confirmation text can scroll. The Folder drawer also uses one scrollable list so its header/footer cannot squeeze out destinations. The main list is keyed by Folder ID: switching from a long list to another Folder does not carry the old scroll offset into that Folder. English labels, monochrome styling, active-first sorting, and upload/receipt behavior are unchanged.

Production changes are limited to this Compose screen. No Rust transfer code, database schema, credentials, server deployment, or Linux desktop code was changed in this round. Test files and the report collector were extended.

## Correction: the missing button label was a preview misinterpretation

The earlier report described a suspected intermittent missing Choose files label. That interpretation was wrong. Reading the original PNG data shows **1,074 bright pixels** in the label region (`x=140..304`, `y=425..454`) in each previously cited image:

- `android/.local/device-qa-cyuym4kp/final-screen.png`
- `android/.local/device-qa-7xk7ud1u/dark.png`
- `android/.local/device-qa-6mfoujfz/dark-confirm.png`

Thresholding those original pixels reveals the complete readable Choose files glyphs in all three, not a blank button. A new device test also passed on the **unchanged original APK**, before deploying the layout fix. It switches light/dark themes and busy/ready states and examines actual screen pixels, not only the Compose semantics tree. There is no established App text-rendering defect to claim as fixed; `BlackButton` and the graphics backend were not changed.

The pixel regression is retained. Its final version asserts a meaningful amount of contrasting white ink (neither a blank dark region nor an all-white background) in the text bounds and saves 12 original screenshots across six theme/state cycles. All 12 final checks measured **1,074 bright pixels / 6,280 sampled pixels**; values are recorded as `MiRelayVisualQA` entries in logcat. This is a targeted visibility check, not a complete screenshot-golden or accessibility audit. Screenshots captured during configuration changes can include system transition frames; they are not approved golden references.

## Added regression coverage

[`DeviceUiTest.kt`](../android/app/src/androidTest/java/io/mirelay/android/DeviceUiTest.kt) now has 18 cases, up from 12. The original landscape visibility assertion is retained and strengthened to check the transfer action as well. Layout-only transfers are paused so Activity recreation does not accidentally enqueue them.

The six added cases cover:

1. Landscape with 1.8× font: file, transfer action, receipt note and Choose files remain reachable.
2. A 720×640-pixel short window with 1.5× font: the welcome screen can reach Add Folder and the editor controls.
3. The same short window with a file: the transfer and receipt note remain scrollable.
4. A short-window drawer with 24 Folders: the last Folder and Add Folder remain reachable.
5. Switching from a 60-row transfer list to another Folder resets the viewport appropriately.
6. Actual button-text pixels survive light/dark and busy/ready changes.

The initial expanded run already passed the original landscape and large-font cases. One new test accidentally selected the drawer's off-screen Add Folder instead of the welcome button because it used a global “last matching text” locator. The locator is now scoped to the welcome container; its isolated rerun passed. This test-harness correction is separate from the production layout fix. The first expanded run is retained in `android/.local/device-qa-ii_na4t1/`.

Display/font/theme changes are confined to the dedicated test AVD and restored by the tests. Original device screenshots are collected in each report's `visual/` directory. No physical phone or production data is used.

## Artifact

- Debug APK: `android/app/build/outputs/apk/debug/app-debug.apk`
- Size: 23,834,936 bytes
- SHA-256: `16157327f8bc781cbf29192af99b3438db8c586faf8f3cff8477a406cf148f93`
- Both ARM64 and x86_64 native libraries remain bundled; runtime verification here uses the Android 16/API 36 x86_64 emulator with 4 KiB pages.

## Post-fix observation

`android/.local/device-qa-08s53b2i/` contains a separate final observation run: five process-cold starts (warm filesystem caches) took **930, 962, 949, 868, 893 ms**, median **930 ms**. A 10.08-second idle sample used 0.06 CPU seconds, approximately **0.6% of one core**. These debug/software-emulator samples are comparable in scale to the earlier baseline, not a release performance guarantee or a statistically significant performance improvement.

Dark-theme, landscape and 1.3× font screenshots were captured after bounded settling delays. Theme, font and rotation settings were restored. Test servers and the owned emulator were stopped at handoff to release memory/CPU; installed tools, the AVD and reports remain available.

## Reproduce

See [the Android README](../android/README.md#local-android-emulator-and-device-tests) for isolated-emulator setup and safety boundaries.

```bash
bash android/scripts/emulator.sh start
# In another terminal, after boot:
bash android/scripts/emulator.sh check
CARGO_BUILD_JOBS=1 cargo build --locked --bins
bash android/scripts/build.sh :app:assembleDebug :app:assembleDebugAndroidTest :app:testDebugUnitTest :app:lintDebug
python3 android/scripts/device-tests.py
bash android/scripts/emulator.sh stop
```

These tests clear MiRelay's data **only on the named, guarded QA emulator**. Reports, synthetic fixtures and test keys remain Git-ignored. Existing unrelated worktree changes were preserved; no commit or push was performed.

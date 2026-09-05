# Brand integration and publication checks — 2026-09-06

## Delivered

- Preserved the supplied MiRelay brand kit, including its original handoff,
  signature, colors, proportions and opaque white/near-white canvases.
- Added embedded Linux icons and an About window with the complete wordmark;
  added a standard application-menu entry and installation instructions.
- Added Android's adaptive launcher icon, drawer icon and complete welcome
  wordmark. Notifications keep the existing monochrome transfer symbol.
- Rewrote the root README in English, covering the current Android/Linux apps,
  pairing, existing-directory initialization, deployment and security limits.
- Kept black functional controls and the existing compact Folder interface.

## Verification

| Check | Result |
| --- | --- |
| Original brand manifest | All 18 SHA-256 entries match |
| Android PNG copies | Byte-identical to the supplied originals |
| Rust all-target regression, desktop enabled | 201 passed, 0 failed, 9 opt-in/helper entries skipped |
| Clippy, all targets/features, warnings denied | Passed |
| Rust formatting and staged whitespace checks | Passed; original kit whitespace deliberately preserved |
| CLI/server build check without desktop features | Passed |
| Linux release build | Passed |
| Linux About graphical test | Passed with Cairo and OpenGL, light and dark; screenshot pixel checks reject an empty wordmark area |
| Additional desktop integration tests | 5 passed: directory Auto receive/restart, corrupt configuration isolation, startup SIGKILL, screenshot I/O errors, bounded idle sampling |
| Deployment helper unit tests | 12 passed |
| Android native builds | ARM64 and x86_64 passed |
| Debug app and instrumentation APK builds | Passed |
| Android JVM tests | 43 passed, including brand resources on API 26 and API 35 |
| Android lint | Passed; existing SDK XML tooling-version warning remains nonfatal |
| Android API 36 emulator suite | 54 tests passed: directory 5, runtime 13, Auto 12, UI 23, notification denial 1 |
| Emulator process-death recovery | Both manual and automatic uploads resumed after SIGKILL; four seed/verification invocations passed |
| Relay → Linux verification | Exact bytes, SHA-256 and ACK verified for 11 legacy files, 1 paired delivery-only file and 49 synchronized paths |
| Focused Android brand UI check | Welcome wordmark verified in dark mode; final test also checks the real package-manager adaptive icon |
| Documentation and desktop metadata | Updated local links resolve; desktop-file-validate passed |

The copied Linux release executable was also launched with an isolated registry
outside the source directory. It does not need a runtime `assets/` directory.
Screenshots were inspected for complete lettering, the creator signature, the
three-i mark and the file illustration. No transparent or monochrome logo was
invented from the opaque artwork.

## Issues found and resolved

- The frontend fixture's three existing test calls had not been updated after
  adding its directory-mode argument. Full `--all-targets` testing caught this;
  the calls now explicitly select delivery-only fixtures.
- A second temporary Cairo renderer could leave the wordmark blank in a later
  screenshot after switching themes. Screenshot capture now uses the window's
  own renderer without unrealizing it. Both renderers and screenshot failure
  paths were retested.
- Initial brand unit tests assumed a pure-white source corner and a real
  PackageManager icon lookup in Robolectric. The originals actually have RGB
  254/254/254 and 253/253/253 corners; they were not altered. Tests now check
  opacity/near-white pixels and inflate the manifest resource, while the actual
  installed-icon lookup is checked on Android ART.
- The new API 26 Robolectric runtime download was slow. Its waiting test process
  was stopped, the download resumed directly, and the JAR/POM SHA-512 digests
  verified before rerunning the complete JVM suite successfully.

## Evidence and limits

Local, ignored emulator report: `android/.local/device-qa-ox0ttnr6/`.
Host screenshots: `/tmp/mirelay-brand-cairo-fixed-i9AHd0/`,
`/tmp/mirelay-brand-gl-fixed-Df3Lqo/` and `/tmp/mirelay-brand-qa-nzesLi/`.
These paths are local evidence, not assets required by the repository.

Built artifact SHA-256:

```text
86e45173c4ef23e0ef04077eab8ae350f4db9302df5e34f114b1ec3230ea3566  mirelay-desktop (release)
a508d83fa73b9eaab2eb1a237c31207451c215eb562eb72fdbd1c39a703e3327  app-debug.apk
```

Only the dedicated MiRelay QA emulator was modified. The physical phone, live
VPS, existing desktop session and real Folder state were not changed. Generated
APKs, native libraries, SDKs, private keys, test databases and credentials are
excluded from Git. Publication candidates were scanned for credential/key
patterns and compared against the known private test credentials without
printing their values; the only token-like literals found were synthetic tests.

This is development validation, not release signing, all-launcher/all-device
certification, an end-to-end encryption claim or a production performance budget.

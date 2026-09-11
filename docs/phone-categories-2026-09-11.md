# Physical phone category acceptance — fixes and regression results

Date: 2026-09-11. Device: vivo V2329A, Android 16 / API 36, connected over USB.
Physical interaction and transfer checks completed after the initial overlay pause.
The initial build failed on completed-upload previews and selected icon contrast.
Both findings have since been fixed and rechecked on the phone as described below.
The initial failing evidence is retained; this is not full-app certification.

## Fix and physical regression follow-up

Updated APK SHA-256:
`ec961ccf2d1ff7436057618d322e21c4f3fddf1ed8f73387422a86db4256c84d`.

- `UploadWorker` prepares a separate, sampled PNG before publishing UPLOADED and
  deleting payload/resume staging. `PhotoPreviewCache` uses atomic writes and a
  disposable private cache limited to 32 MiB / 128 entries. Preparation is
  serialized, and ordinary cache/codec/storage errors do not fail the transfer.
  Reads prefer that preview; switching General → Photos does not require uploading
  the image again. No source file or sync-directory thumbnail is written.
- Drawer selections explicitly set white icon color on the black background.
- **69 JVM tests passed** (including six new cache regressions); Lint reported
  **0 errors, 11 existing warnings**. Tests cover staging removal/new reader,
  pixel and sampling bounds, entry/byte eviction, interrupted temporary writes,
  missing/corrupt images, unavailable storage and invalid IDs/targets.
- Emulator real-worker upload tests verify successful original and corrupt-image
  delivery, staging/resume removal, fresh disk reads, gallery/modal rendering and
  activity recreation. Separate seed/verify instrumentation runs force-stop the
  app between phases and assert a changed PID, valid 256/1024 previews, original
  pixels and no staging payload. Both cold-start phases passed in
  `android/.local/device-qa-3lgewvdh/`.
- The broader run in `android/.local/device-qa-_w897fxt/` passed directory (8),
  runtime (14), Auto (12), notification-denial (1) and 28 of 29 UI tests, plus both
  manual/Auto SIGKILL resumptions. Linux verified 13 legacy, 1 paired delivery-only
  file and 52 directory paths. The remaining failure was an ambiguous locator in
  the new icon test, not a passing full-suite result. A rerun then exposed the test
  toggling an already-restored drawer closed after changing theme. The test now
  checks visibility and waits for the drawer before sampling. The complete
  **29-test UI group then passed** in `android/.local/device-qa-e65cxt1_/`, including
  actual white icon pixels for both General and Photos in light/dark themes and
  the real-upload gallery/modal check. That run also verified 6 legacy and 1 paired
  Linux deliveries. Together the reruns cover all 64 ordinary instrumentation
  cases, plus preview cold-start and both upload SIGKILL scenarios; this is not a
  claim that the earlier failing all-groups run was green.
- The phone received an in-place, signature-checked update after a fresh APK/data
  backup. **4 Folders, 55 transfer rows and 55 private staging files** present at
  that point were retained exactly. No uninstall, data clear, permission reset or
  emulator reset harness was used on the phone.
- Resumed only `qa photos bjia` against the existing loopback USB test relay and
  added `preview-fixed.png` and `preview-invalid.jpg` to its existing SAF directory.
  Auto sent both; Linux verified exact original hashes and acknowledged them.
  All six test records are now Synced and the version counter advanced to 7.
  Both new payloads and resume files were removed after upload completion.
- The valid PNG renders in the real phone gallery and modal. A consistent private
  backup contains its independent derived preview; all 512×512 RGBA pixels match
  the original fixture. The corrupt JPEG transfers unchanged but has no derived
  preview. After force-stop/relaunch, a different app PID still renders the valid
  image. The selected Photos icon visibly renders white on black.
- Existing records and private staging bytes were compared again after testing;
  all seven phone fixture originals and the Linux-only fixture remain unchanged.
  The test source is Images only / paused, without error or active source jobs in
  the WorkManager snapshot. Screen timeout, rotation and font scale are unchanged.
  Collected phone app-PID logs contain no fatal exception, fatal signal, ANR,
  out-of-memory or list-index crash match. This is not a long-duration crash test.

Private evidence is under `target/phone-categories-20260911-B9J6Ia/`:
`fix-upgrade-verified.json`, `fix-final-verified.json`, `fix-final-v2.tar`,
`fix-cold-start.json`, `fix-valid-preview.png`, `fix-cold-grid.png`,
`fix-cold-preview-settled.png`, `fix-corrupt-placeholder-settled.png`,
`fix-selected-icon.png`, and `fix-derived-preview-v2.png`. Original transition-frame
captures are retained separately; settled captures were taken without image edits.
The first final-audit attempt looked for WorkManager under `databases/`; Android
stores it under a different private directory. That audit failed rather than
silently passing. The corrected audit locates the exact database basename,
validates it, and retains both attempts without overwriting evidence.

After the regression, the loopback relay was stopped, its exact port 18093 USB
reverse mapping removed, and the repository-owned emulator stopped. Test originals
and private backups remain available; no production deployment, commit or push
was performed.

Limits: Android/capacity eviction can remove disposable previews. Old completed
records whose original staging was already removed cannot be reconstructed by
this fix; no implicit SAF scan or download is introduced. Unsupported formats
still show placeholders. Large-library performance, extended screen-off/OEM
behavior and multi-device category exchange were not retested here.

## Initial acceptance — verified before the fix

- USB enumerated the phone and its ADB interface, but the existing ADB host process
  initially listed no devices. Restarting that host process restored an authorized
  connection. No device reset was performed.
- Backed up the installed APK and private application data before upgrading.
  The APK signing certificates matched. Installed the development APK in place
  using `adb install -r --no-streaming`, without uninstalling or clearing data.
- Installed APK SHA-256:
  `5e8b2a6dbe95a681eca4da663cc73eb9a9e3347cfb075efcc1cdd732c4bd6d80`.
- SQLite upgraded from version 4 to 5 and passed integrity / foreign-key checks.
  All existing columns and rows were compared: **3 Folders, 51 transfers and
  2 Auto sources** were retained, including encrypted credentials, pairing and
  source configuration. Only existing enabled-source scan timestamps are allowed
  to advance in this comparison.
- Existing Folders defaulted to General. Existing sources retained All files,
  with no new filtering consent or directory initialization.
- **51 original private staging files** were compared across backups and retained
  byte-for-byte. The supplied artistic wordmark rendered in the upper-left header;
  the existing Folder list and transfer records were accessible.

Private backup and test artifacts are under the ignored
`target/phone-categories-20260911-B9J6Ia/` directory. Android Keystore material is
not exportable: these backups are not a promise of recovery after uninstalling.

## Physical interaction and transfer results

The test used a new, isolated Folder (`qa photos bjia`) and the real Android
DocumentsUI picker to grant access only to `Documents/MiRelay-QA-Photos-B9J6Ia`.
A loopback-only Rust relay on the development computer was reached over a dedicated
USB reverse mapping; the production VPS and real Linux synchronization folders
were not used. No emulator-only instrumentation/reset harness ran on the phone.

- Compared the phone's verification code with the receiver handshake before
  confirming pairing. Sending was blocked before confirmation and ready afterward.
- Selected Photos and initialized an **already existing** directory in place.
  Images only preview showed **2 missing, 3 skipped, 1 Linux-only file retained**.
  The relay had no incoming entries before confirmation.
- Explicit initialization uploaded the valid PNG and intentionally invalid JPEG.
  The text, binary and empty temporary-download file were excluded by extension
  policy. This confirms the policy is selection, not content validation.
- The real Linux directory receiver verified the two incoming hashes and receipts.
  Refresh receipts changed the phone's Waiting for Linux state to Synced.
- Paused the test source and requested All files. The empty `.part` file blocked
  the preview with an explicit zero-byte error; no transfer was queued and the
  committed policy remained Images only, with `next_version = 3`.
- Moved only that empty test fixture outside the source, preserving its bytes.
  A new All files preview showed **2 identical, 2 missing**. Explicit confirmation
  sent `notes.txt` and `data.bin`. Linux received matching SHA-256 bytes and all four
  receipts were acknowledged. Earlier records/versions were retained; the next
  version increased to 5 rather than resetting.
- Paused again, restored the empty test fixture to its original path, and previewed
  Images only. It showed **2 identical, 0 missing, 3 skipped** and retained the three
  Linux-only/excluded destination files. Applied the narrower policy and paused.
  No received file was deleted, re-uploaded or reset.
- Changed General → Photos with the source paused. Database comparisons showed
  identical filter, revision, version counter and transfer records; only presentation
  changed. A force-stop/relaunch retained Photos, Images only, the paused state and
  four Synced records. This is a cold-start check, not a mid-upload SIGKILL test.
- Both image modals opened and closed without an observed crash. However, neither
  showed an image; the valid PNG case is a failure, not successful preview coverage.

## Initial finding 1 — completed uploads lose their preview source

The valid `art.PNG` is the supplied, unmodified 512×512 PNG (191,605 bytes).
Its phone original and received Linux copy matched the expected SHA-256. The phone
nevertheless displayed **Preview unavailable** in both the gallery and modal,
including after a cold start.

`PhotoThumbnails.load` in `PhotoPreview.kt` reads `outgoing/<transfer-id>/payload`.
`UploadWorker.kt` lines 75–78 persist UPLOADED and then delete that same payload and
resume state. Private snapshots confirmed all four completed test payloads were
absent. This is a lifecycle mismatch, not transmission corruption. A decode that
wins the cleanup race could temporarily work from memory; it is not reliable.

The earlier `DeviceUiTest.photosRenderOriginalLocalArtAndCategorySwitchDoesNotChangeTransfers`
fixture manually marked a transfer UPLOADED and retained its payload, bypassing the
real worker cleanup. Directory-transfer tests did not assert decoded image pixels.
Their passing results did not cover the combined upload → cleanup → preview path.

Recommended follow-up: separate bounded preview retention from replayable upload
staging, without retaining every original upload indefinitely. Add a real-worker
upload/cleanup/cold-start image regression before calling this feature ready.

## Initial finding 2 — selected category icon contrast

The selected Photos icon renders dark purple on the black drawer selection.
`NavigationDrawerItemDefaults.colors` overrides the selected background and text,
but not the selected icon color. The original `photos-drawer-selected.png` screenshot
confirms the icon is hard to distinguish. Selected icon colors need explicit
contrast plus light/dark visual regression coverage.

## Evidence and cleanup

Private evidence includes `after-images-verified.json`,
`after-empty-guard-verified.json`, `after-all-files-verified.json`,
`after-general-verified.json`, `after-photos-verified.json`, and
`final-verified.json`, plus the original screenshots. No screenshot was edited.

The final consistent backup verified the original **3 Folders, 51 transfers,
2 Auto sources and 51 private staging files** were preserved. Existing enabled
source scan timestamps may advance; original configuration and credentials did
not change. Only one test Folder, one paused test source and four test transfers
were added. All five phone test originals were hashed after restoring the empty
temporary file. The original Linux-only fixture was also unchanged.

The test source has **Images only / paused**, no source error, and no queued,
running or blocked source jobs in the WorkManager snapshot. It remains available
for a follow-up fix/regression. The loopback relay was stopped and its exact port
18093 USB reverse mapping removed. Private fixtures and backups are retained.

The collected app-PID log contained no match for `FATAL EXCEPTION`, `Fatal signal`,
`ANR in`, `OutOfMemoryError` or `IndexOutOfBoundsException`. That does not establish
overnight/OEM crash freedom. A single UI memory sample was 191,356 KiB PSS,
319,552 KiB RSS and 24,209 KiB swap PSS; absent preview payloads mean this is **not**
a successful image-decoding or large-gallery performance benchmark.

No production server, existing Folder policy, default input method, screen timeout,
rotation or font-scale setting was changed. Input simulation was affected by the
phone IME's language/shift behavior; test field values were checked, with a valid
loopback URL and claimed invitation before proceeding. No commit or push occurred.

## Initial interruption (historical)

A calculator floating window repeatedly covered MiRelay and intercepted screen
targets. UI automation was paused rather than proceeding against uncertain targets.
The helper was tightened to only enter fields belonging to the MiRelay package.
The user then closed the overlay and authorized continuation. The resumed results
above supersede the initial partial report. Extended screen-off/background,
network-loss, large-library and multi-device category tests were not rerun here.

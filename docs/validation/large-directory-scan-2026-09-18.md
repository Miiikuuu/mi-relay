# Large-directory scan repair — 2026-09-18

## Scope

Remove the aggregate 4 GiB inventory restriction from Android and Linux. Content
hashing remains sequential and streaming with a 64 KiB buffer; Android reuses the
buffer across files. The 100 MiB individual-file limit, nonempty-file validation,
path/node/metadata bounds, source-change checks and upload batching remain intact.
No protocol, credential, server or database-schema migration is required.

Directory scans now enforce a 90-second inactivity budget, refreshed by successful
reads/traversal, instead of rejecting a progressing scan after 90 seconds total.
Android background directory work also retains an eight-minute absolute budget.
Ordinary Auto/import operations keep their original absolute timeout. Cancellation
closes the tracked stream and an incomplete inventory is never accepted as a
baseline. Android uses fixed-delay timer checks to avoid catch-up bursts after a
cached process resumes.

This is **not incremental inventory scanning**: preview, confirmation and later
checks still hash the selected source files. Slow/large libraries may still
exceed the background budget. Uploads remain batched, not one all-library staging
operation. Linux checks time between filesystem operations; it cannot interrupt
an individual blocked filesystem call.

## Local regression evidence

- `cargo test --features desktop --quiet -- --skip inventory_over_four_gib_streams_all_files`:
  **243 passed, 0 failed, 23 ignored**. The intentionally omitted heavy scan ran
  separately, as below. Ignored graphical/device-specific tests are not passes.
- `cargo test --lib directory::receiver::tests -- --test-threads=1`:
  **8 passed**, including an actual **43 × 100 MiB (4.199 GiB)** inventory scan.
  Sparse fixtures avoid allocating several GiB on disk, but all bytes pass through
  the real file read/hash loop. The suite also checks unchanged per-file/node
  restrictions, timeout rejection and recovery boundaries.
- Android's `DirectorySourceTest` exposes 43 SAF document paths over a sparse
  100 MiB backing file. The real reader hashes all 4.199 GiB, verifies SHA-256,
  preserves the source, rejects unsupported/stale metadata before opening payloads,
  and tests timer-driven and explicit cancellation.
- `ScanDeadlineTest` uses a monotonic fake clock to cover progressing scans well
  beyond 90 seconds, idle expiry, nonrenewable ordinary operations, the absolute
  background deadline, and rejection of late progress after expiry.
- Strict desktop/all-target Clippy, Rust format and whitespace checks passed.
- Release desktop/directory binaries and both Android native ABIs were rebuilt.

- Final Android `testDebugUnitTest`: **116 passed, 0 failed, 1 skipped** (the
  optional external QR fixture was not supplied). `assembleDebug` and `lintDebug`
  passed, with **0 lint errors and 18 existing advisory warnings**. The new
  fixed-rate-timer advisory was corrected and is absent from the final result.

Final build artifacts (subsequent installation is recorded below):

| Artifact | SHA-256 |
| --- | --- |
| `android/app/build/outputs/apk/debug/app-debug.apk` | `ddf331ff98d5d23dfe5fffdc2d00aaa8780572b2e236dd5d871d86fa6cf55b27` |
| `target/release/mirelay-desktop` | `10679471e33db94270c680d0d77bd80d30cdb767bbdb32744b06535090a34355` |
| `target/release/mirelay-directory` | `9a537ef3a9cc72905c4055715046d04f2e35e6661263d878826819c4b4987141` |

## Device and deployment boundary

The user's existing photo directory was inspected read-only before this repair:
1,238 files, 5,175,610,575 bytes, no empty files and no files above 100 MiB. Its old
invitation had expired before it reached initialization preview.

The user chose **local repair/testing now, installation later**. No updated app
has been installed, no invitation was replaced, no bulk upload was started, and
no source file was changed or deleted. A fresh invitation and a real-device
initialization preview remain to be tested after installation. No production
server change, commit or push belongs to this run. This is not full release
qualification or real-device large-library performance evidence.

## Subsequent authorized installation

The user subsequently authorized installation. The desktop executable was
replaced atomically with the exact hash above, retaining its previous binary;
configuration was not modified. The installed executable's `--help` check passed.

Android signing identity was matched to the existing installation, and the app's
private data and old APK were backed up before using overwrite installation.
The first system prompt was dismissed; reconciliation confirmed the old APK
remained, and the user-authorized retry succeeded. The installed APK hash matched
the artifact above and the application UID stayed unchanged. The app started
successfully; post-start backup comparison verified the original database rows,
six tracked private/preference files and schema 7 were preserved. The app had no
saved Folders or transfers before or after the update.

No source files, server configuration or pairing invitations were changed and
no upload was started. The 4.82 GiB physical-directory initialization preview
was still outstanding at this installation stage; the subsequent preview is
recorded below.

## Subsequent physical existing-library preview

The exact installed artifacts above completed pairing through the public HTTPS
relay. Both visible verification codes were compared before Linux confirmation.
The existing empty Linux destination was reviewed and initialized (publishing an
empty inventory), then Android's **All files** preview scanned the real existing
photo library without confirming Android initialization or enabling uploads.

- Source metadata measured **1,238 files / 5,175,610,575 bytes (4.820 GiB)**.
- Native preview: **0 identical, 1,238 missing, 0 different, 0 Linux-only,
  0 skipped**; the UI reached **Initialize sync** with no scan error.
- UI polling saw the operation still busy at +53.6 seconds and complete at
  +103.6 seconds. These are observation bounds, not an exact hashing benchmark
  and not proof that this particular scan exceeded 90 seconds.
- Two-minute app-only monitoring retained the same process, recorded no fatal
  log markers, and sampled peak PSS **290.8 MiB**, RSS **415.5 MiB**. This is
  sampled whole-app memory, not a continuous peak or a cross-device guarantee.
- Android confirmation was **not clicked**. The Linux destination still had
  only its receiver lock and no delivered media. Original media was not changed.

One setup defect remains: the native **Show QR code** control reported expanded
but did not expose a visible popover in this desktop session. A private temporary
standalone QR viewer, using the same invitation and actual server expiry, enabled
optical scanning. The first invitation expired during troubleshooting; the same
Folder's invitation was renewed and rescanned successfully. The temporary viewer
was closed afterwards. This workaround does **not** count as fixing or validating
the native popover. No new server deployment or code push occurred.

Outstanding: native QR-popover repair, Android final initialization confirmation,
bulk transfer/ACK validation for this library, and background operation. This
result establishes real-device large-library **preview**, not complete syncing.

## Subsequent desktop input repair

The diagnostic About window was modal and blocked its parent until closed.
The original process still answered accessibility requests; this alone did not
establish working pointer input. About is now nonmodal, closes on Escape and is
destroyed with its parent. The QR control now toggles an inline 300-pixel image
in the setup window, with no popup surface or reveal animation. Clearing or
expiring an invitation hides the image and resets/disables its toggle.

- Ordinary Rust regression: **243 passed, 0 failed, 24 ignored**, excluding the
  heavy inventory scan already exercised above; strict Clippy, formatting and
  whitespace checks passed.
- Owned headless Wayland/GL: inline QR geometry/native-surface identity,
  repeated show/hide and adjacent-control callbacks passed; replacement/expiry
  also passed. About encountered a compositor connection reset in this kiosk
  environment, so it is **not** recorded as a Wayland pass.
- Independent owned Xvfb/cairo: all **3 targeted GTK tests passed**, including
  About's nonmodal/lifetime assertions, complete artwork in both themes and
  close-button behavior. Xvfb's missing-DRI3 warnings are expected in this
  software-rendered environment.

These component tests exercise GTK state/layout and programmatic actions, not
physical mouse input or a new optical phone scan. They do not constitute full
release acceptance. No Android initialization confirmation or bulk upload was
performed as part of this UI repair.

The installed desktop was subsequently replaced atomically and restarted with
SHA-256 `dba42dabeb8b2f2d995a8eb9e41b6b442c4399bee708731a945d45e2f2ca153f`.
The prior executable was retained privately. Its existing session credential
was handed over in memory via the configured environment variable, without
writing the credential to disk. Registry and Folder configuration were byte-for-
byte unchanged at handoff, and the existing paired Folder returned to **Ready**.
Three installed-desktop settings open/cancel cycles passed. About's accessibility
state confirmed nonmodal; attempting another settings action during that check
encountered a temporarily disabled control during automatic receiving, so that
combined action sequence is not a pass. The About window was closed and only the
main window was left open. User-driven physical pointer operation and another
native optical QR scan are not claimed by these checks.

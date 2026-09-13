# Android album mode

Photos is a browsing surface, not the transfer queue. General retains the file
management view; Linux's Photos view is unchanged by this Android-only revision.

## Browse

- An existing Folder with a configured source displays image files from that
  **already granted source tree**, including images never uploaded or skipped as
  identical at sync initialization. Nested directories are included. No device-wide
  media permission or MediaStore scan is requested.
- Browsing is read-only: it does not initialize sync, hash the library, upload,
  change filters, delete originals, or create files in the source directory.
  Existing, explicitly enabled Auto schedules retain their normal behavior.
  MiRelay makes no album requests to the relay; a cloud-backed Android document
  provider can still fetch its own file data when the granted source is read.
- Photos without a configured source still shows local image transfer records.
  This compatibility view cannot reconstruct discarded upload previews; configuring
  a source is a separate action in Folder setup / Sync settings.
- The grid starts at three columns and uses up to six on wider windows, with
  narrow gaps, no filenames under tiles and no per-image Synced labels. Images are
  ordered by file modification date, newest first; unknown dates sort last. This
  is not EXIF capture-time ordering. Cropping affects grid display only.
- Tap to view full screen, swipe between photos, double-tap or pinch to zoom, and
  pan a zoomed image. Zoom is bounded to 1–4× and panning respects the fitted image
  edges. Previous/Next buttons are available as alternatives to gestures. Tap once
  to hide or show controls; file metadata lives under Photo details.
- Refresh photos and returning to the app refresh the visible library. There is
  no new polling service or background album index. Incomplete provider listings
  and revoked permissions show an error, not a partial successful library.

## Quiet controls and glass

The original, uncropped wordmark stays at the upper left. The compact header
contains the Folder title, a sync/activity summary, refresh, options and Folder
navigation. Transfer activity contains all file rows, errors, retry/pause actions,
pairing information and receipt refresh; non-image attachments remain available
there. Album options provides Sync settings, Folder settings and General view.

The frosted header replays the grid's graphics display list into a header-sized
blur layer; it does not blur text/buttons or take repeated bitmap screenshots.
Images can scroll beneath it. Older Android versions, non-hardware rendering,
low-RAM devices and power saving use a translucent fill. **Reduce transparency**
in Album options forces this fallback and persists across app restarts. The
wordmark's supplied opaque white canvas is intentionally preserved in both themes.

The implementation uses Compose's [graphics layers and drawing modifiers](https://developer.android.com/develop/ui/compose/graphics/draw/modifiers)
and [conditional pan handling](https://developer.android.com/reference/kotlin/androidx/compose/foundation/gestures/transformable.modifier)
to keep ordinary swipes available while the photo is fitted to the screen.

## Resource and format limits

- A complete source metadata scan remains bounded to 5,000 entries / 16 nested
  levels. It uses the existing strict directory traversal and persisted read grant.
- Only visible/prefetched tiles and viewer pages request image data. Source reads
  have two concurrent permits and a 15-second provider deadline, with cancellation
  when requests leave composition or the host lifecycle drops below STARTED.
  Listing scans follow the same lifecycle, restart on return, and fetch no image
  bytes. This does not cancel explicitly enabled background sync/transfer jobs.
- Source previews require a known nonzero size of at most 32 MiB. A bounded private
  scratch copy supports non-seekable SAF providers. The declared length and source
  metadata are checked before/after reading. Scratch copies are removed afterward;
  leftovers from a previous process are removed at the next source decode.
- Decoded images are sampled to 256 pixels for tiles / 1024 for the viewer, with
  a 12 MiB source bitmap cache (8 MiB tiles, 4 MiB viewer). The existing 8 MiB
  transfer-thumbnail cache (4 MiB tiles, 4 MiB viewer) and
  32 MiB / 128-entry durable upload-preview cache remain separately bounded.
- Independent size-class budgets keep viewer paging from evicting grid tiles.
  This reserves viewer capacity, reducing the maximum tile-only working set
  compared with a shared pool; it is a bounded tradeoff, not a universal cache-hit
  improvement. Visible bitmaps are never recycled on cache eviction. These cache
  budgets are not total process-memory limits.
- Identical concurrent preview requests share the first successful result through
  32 fixed mutex stripes. There is no unbounded task map, orphaned application
  scope or retry loop; rare hash collisions can serialize unrelated requests.
  Source cache hits still verify permission, but skip decode permits, scratch
  copies and provider-session/deadline-thread creation. A cancelled decode cannot
  insert its result. Null/failed loads may retry on a later request.
- A refresh creates a new cache generation, so even providers retaining the same
  size/timestamp are reread on refresh. Permission is rechecked on cache hits.
- Unsupported, corrupt, missing, virtual, empty or over-limit images show a
  placeholder; they are not removed. Original resolution is not decoded for zoom.
  Animated formats remain still previews; full EXIF orientation and media editing
  are outside this slice. These are safeguards, not a codec security sandbox.

No database schema, protocol, tus, receipt or deletion behavior changes are needed.
There is no implicit server deployment or Git push with this UI change.

## Verification — 2026-09-11

- Debug APK SHA-256:
  `30f0693d244229765073150ec0862180cc0a79094cd3e121cd68aebee5345bac`.
- 74 JVM tests passed, including album ordering, never-uploaded source images,
  empty-image metadata, 5,000-entry presentation, fitted-image pan bounds and glass
  eligibility. Lint: zero errors, 14 advisory warnings (dependency updates and
  Kotlin/resource style suggestions). Both debug APKs built successfully.
- Dedicated API 36 emulator: all 33 `DeviceUiTest` cases passed in 134.791 seconds.
  New cases cover source browsing without uploads/configuration changes, fullscreen
  double-tap zoom and swipe paging, details, scrolling glass, persistent reduced
  transparency, source deletion/refresh, incomplete listings, revoked grants,
  oversized previews, changed metadata and prompt cancellation of a blocked read.
- Existing UI coverage includes real upload cleanup/retained previews, Folder
  switching, selected icon contrast in both themes, landscape/large text, pairing,
  pause/retry and system sharing. The UI harness independently received and checked
  hashes/bytes/acknowledgements for six legacy files and one paired delivery file
  using the Linux CLI.
- Private local artifacts: `android/.local/device-qa-adxgxd69/`. Original screenshots
  `visual/album-frosted-scroll.png` and `visual/album-reduced-transparency.png` were
  inspected together: the grid scrolls beneath the header, blur affects the image
  backdrop, and controls remain sharp. `visual/photos-real-upload-preview.png`
  confirms the fullscreen viewer. The `album-fullscreen.png` capture caught the
  details dialog's window transition; it is not used as a settled-viewer reference.
- The same APK passed the remaining 35 ordinary device cases: 8 directory-sync,
  14 runtime/JNI/security, 12 Auto and 1 notification-denial test. Together with
  the UI run this is **68 ordinary device cases**, all passing. Preview cold-start
  recovery and both manual/Auto SIGKILL-resume scenarios also passed (six separate
  seed/verify instrumentation phases). This second harness run independently
  verified eight legacy deliveries and 52 synchronized Linux paths, including
  bytes, hashes, receipts and conflict/history expectations. Private artifacts:
  `android/.local/device-qa-46qaas8f/`. No APK/source changes occurred between runs.

This is development regression coverage, not release/performance certification.
The 5,000-entry test exercises metadata presentation, not a 5,000-photo scrolling
benchmark. Physical-phone installation and new album gesture/performance checks
are separate from these emulator results.

## Performance/lifecycle follow-up — 2026-09-12

Album scanning and preview loading now use lifecycle-aware STARTED scopes rather
than composition lifetime alone. Returning to the foreground still refreshes the
library, including unchanged-metadata providers via a new generation. No background
sync consent, WorkManager policy, database schema, JNI or server protocol changed.
The original graphics-layer frosted header is retained.

The shared `PreviewMemoryCache` separates tile/viewer budgets, coalesces requests
and prevents cancelled results from entering memory cache. A source cache hit
rechecks the persisted grant but no longer starts an `AutoSession` or its deadline
thread. Independent provider reads still have the same two-permit bound and
scratch/size/metadata checks. Native bitmap decoding remains non-interruptible;
late results are discarded rather than falsely claiming immediate codec preemption.

Verification includes eight new JVM regressions: viewer-vs-tile eviction, bounded
tile eviction/oversized viewer entries, 40 concurrent identical requests, grant
validation on hits, cancellation during non-cancellable work, null/error retry,
size/generation identity, and invalid target rejection. All **82 JVM tests** pass.
Lint reports **0 errors, 14 advisory warnings**. Both debug APKs build with the
existing native libraries; no Rust/JNI source changed for this optimization.

An initial incremental Kotlin build failed to resolve unchanged project symbols;
a non-incremental rebuild succeeded (`-Pkotlin.incremental=false`, command-local,
no persistent toolchain change). The new instrumentation stats helper also needed
a return-value correction before the test APK compiled.

The first complete 35-case UI run caught an `InterruptedIOException` escaping a
timeout cancellation: the now-IO-dispatched cache path could receive stream-close
failure before reaching another coroutine suspension. `AlbumLibrary.read` now
checks coroutine cancellation before propagating a provider exception, preserving
genuine errors when the coroutine remains active. The blocked-read regression now
repeats cancellation five times. The initial failing run is retained in private
`android/.local/device-qa-o8wm2354/`, not counted as a passing suite.

The corrected build's full **35-case DeviceUiTest group passed** in 143.177 s,
including five blocked-read cancellations, background cancellation with an empty
scratch directory/no subsequent opens, resume refresh without uploads, and twenty
identical concurrent source requests producing exactly one provider open. A new
scan generation forced a second read; revoking the grant blocked its cache hit.
Existing gallery, glass, viewer gestures, upload-cleanup, sharing, pairing, theme,
large-font and short-window cases also passed. The harness independently verified
six legacy and one paired Linux deliveries (bytes, hashes and acknowledgements).
Passing artifacts: `android/.local/device-qa-8cl1hagc/`.

Tested APK SHA-256:
`c1586428e4f24c38916e1c97dff9766e0cd2fd539ec4dd643883c00253211bfe`.
This pass reran the UI group, not all Android runtime/Auto/directory groups or
process-kill recovery phases. It is not a large-library phone FPS, long-duration
battery or release-build certification. Existing physical-phone data was not used
by the emulator reset/instrumentation harness.

### Physical-phone follow-up

The authorized vivo V2329A/API 36 received the same tested APK via in-place
`adb install -r --no-streaming`. Its previous APK and app-private data were backed
up locally; signing certificates matched. The first install was rejected by the
phone's confirmation UI, then a retry succeeded without disabling security
settings. The initial archive included a nonexistent preferences directory and
failed tar validation; it was superseded by validated backups of the directories
actually present. Android Keystore keys are not exported by these backups.

Immediately before/after installation, SHA-256 comparison confirmed **62 protected
database/private files unchanged**, including **4 Folders and 57 transfer rows**;
SQLite integrity passed. No uninstall, data clear, test-provider APK installation,
new source file, sync enablement or system-setting change occurred on the phone.

The existing paused QA source displayed four image entries: two valid PNG previews
and two known corrupt-image placeholders. Grid rendering, opening a photo,
double-tap zoom (checked in before/after screenshots), Next navigation across all
four entries, close, and Home/foreground restoration worked. The phone's previous
APK still used the old list-style Photos screen, so this is also its first physical
check of the newer source-album UI; it is not a matched old/new scrolling benchmark.

Five-second process-CPU samples, on a one-core basis, measured 0% in settled
foreground and 0% in settled background. Immediate Home transitions measured
3.50% and 2.73% in separate samples and are explicitly **not steady idle**. A memory
snapshot reported 175,563 KiB PSS / 297,716 KiB RSS / 29,051 KiB swap PSS. These
small-library, USB-connected debug measurements do not certify large-library FPS,
instant zero work, power consumption or long-duration background stability.
The collected current-process log contained no fatal exception, fatal signal,
ANR or out-of-memory match; this is a bounded observation, not proof no crash can
ever occur. Private artifacts: `target/phone-album-perf-VzUBLZ/`.

The dedicated emulator was stopped after testing. The updated phone app was left
open on the QA album; production sync settings and the existing source pause
state were preserved. No Git commit or push was performed by this follow-up.

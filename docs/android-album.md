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
  when requests leave the screen. No image bytes are fetched by the listing scan.
- Source previews require a known nonzero size of at most 32 MiB. A bounded private
  scratch copy supports non-seekable SAF providers. The declared length and source
  metadata are checked before/after reading. Scratch copies are removed afterward;
  leftovers from a previous process are removed at the next source decode.
- Decoded images are sampled to 256 pixels for tiles / 1024 for the viewer, with
  a 12 MiB source bitmap cache. The existing 8 MiB transfer-thumbnail cache and
  32 MiB / 128-entry durable upload-preview cache remain separately bounded.
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

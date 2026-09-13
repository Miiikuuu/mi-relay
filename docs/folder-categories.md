# Folder categories — General and Photos

The initial [physical-device acceptance](phone-categories-2026-09-11.md) found a
completed-upload preview lifecycle bug and low-contrast selected icons. The
follow-up separates derived previews from upload staging and explicitly sets
white selected icons; the report records regression verification separately from
the original failing build.

Folder identity, credentials and transport are unchanged. This development slice
adds **General** and **Photos** as local presentation choices, plus a separately
consented directory-sync filter. It does not deploy or change the relay server.

## Presentation

- Choose the type while adding a Folder, or tap General / Photos above an existing
  Folder name to switch locally, including offline. Folder settings also exposes
  the choice. The drawer uses the matching monochrome line icon.
- General keeps the transfer list. Android's [album mode](android-album.md) uses
  a compact 3–6-column grid, full-screen paging/zoom and a frosted header. Transfer
  activity, including non-image attachments and retry actions, lives in a separate
  panel. Linux has a [virtualized album and local viewer](linux-album.md).
- Android Photos browses the already configured SAF source read-only, including
  images without transfer records. Without a configured source it falls back to
  local image transfer records. It is not a device-wide media-library scanner.
- Before publishing upload completion and cleaning staging, image transfers get
  a best-effort derived PNG preview, sampled to at most 1024 pixels per edge.
  This works for General too, so switching to Photos later can use the preview.
  Atomic writes keep partial previews invisible. Private disk cache is bounded
  to 32 MiB / 128 entries, oldest-access-first eviction; a write may temporarily
  need up to 5 MiB extra. Interrupted temporary writes are removed on the next
  successful preparation. Cache/storage/codec failure does not fail an upload.
- Transfer-record previews read this cache or a still-existing immutable staging
  payload, never remote URLs. Two concurrent transfer UI decodes, sampled
  256-pixel tiles / 1024-pixel modals, and an 8 MiB RAM cache bound rendering work;
  worker preview preparation is separately serialized. Invalid/unsupported images
  and missing/evicted previews show a placeholder. Images over 100 million
  declared pixels are not decoded. This is not a codec sandbox.
- Preview cache survives ordinary process restart, but Android or capacity
  eviction can remove it. Completed transfers from the old build whose staging
  was already removed cannot be reconstructed from transfer records alone.
  Android's configured-source album can display the original independently;
  no re-download is introduced. New image uploads populate the cache.
- Source originals are never edited, recompressed or deleted. No thumbnail is
  written into a synchronized directory. No new relay request, broad storage
  permission, media scanner, or background service is introduced. A cloud-backed
  source provider may perform its own network reads for album previews.
- Animated formats use a still preview. Decoder/EXIF orientation support is limited
  by this first implementation; full media playback/editing is outside this slice.

## Sync selection and consent

Changing presentation **never** changes which files may be sent. Directory sync
has its own All files / Images only selector:

1. Pause an enabled source before changing its filter.
2. Choose a filter and press **Preview changes**.
3. Review included changes and skipped counts/reasons. Up to 50 changed and
   20 skipped paths are rendered; the complete skipped list remains in the bounded
   persisted preview. All originals and Linux-only files are kept.
4. Press **Initialize sync**, or **Apply filter and resume** on an initialized
   Folder. The app rechecks the selected files, skipped metadata, policy and remote
   inventory. A changed preview requires a new review.

New Photos setups suggest Images only; General suggests All files. Neither
suggestion enables sync. Existing sources retain All files after upgrading.
Resuming an existing source uses its previously confirmed policy.

`images-v1` includes jpg, jpeg, png, webp, gif, bmp, heic, heif, avif, tif and tiff
extensions, case-insensitively. Names ending in `.part`, `.tmp`, `.crdownload` or
`.download` are reported as temporary downloads. Other names are reported as not
included image extensions. The rule does not inspect image contents and is **not
malware detection**; renaming a binary to `.jpg` does not make it a valid image.
Choose All files for sidecars, raw formats, SVG or other attachments. There is no
custom extension editor or automatic deletion in this slice.

Filters apply only to directory sync, not one-off shares or delivery-only Auto.
They are sender-side selection rules, not relay-enforced file-type permissions.
Excluded files are still counted in the bounded metadata traversal, but are not
hashed or staged for upload. The selected file set still receives full SHA-256
checking; this does not introduce a timestamp-only shortcut.
Existing validation still applies to included files: nonempty files, size and
inventory limits, safe paths, complete provider listings and read permissions.
Unsafe paths are rejected before filtering. An empty excluded temporary file does
not prevent image-only initialization; an empty included image still does.

Changing an initialized filter requires all existing queued/paused/failed transfers
to finish first. It retains transfer records, path history and increasing version
numbers. Excluded historical paths are not deleted or automatically removed from
Linux. Widening the filter previews and synchronizes newly included content.
There is no change to tus, SHA-256, delivery receipts or conflict preservation.

## Compatibility and remaining work

Android SQLite added category defaults in schema **5**; the current schema **6**
also prevents unsafe downgrades past the Folder-disconnection state barrier.
Migration from versions 1–4 retains the General / All files defaults.
Encrypted tokens, existing Auto consent, pairing, transfer rows and directory
history are retained. Unknown presentation keys fall back to General; unknown
filter keys fail closed. An old Android binary cannot open the upgraded database:
back up app data before a physical-device upgrade; do not uninstall to downgrade.

Linux receives selected files using the existing directory protocol. Category
exchange during pairing, complete source-library browsing, Music, Videos, Books
and Study remain follow-up work. No physical phone or production server was
changed by these implementations.

## Linux local categories

The selector above the Folder name changes General / Photos offline. New and
imported Folders start as General; choose Photos after adding the Folder. The
sidebar and detail icon follow that choice. This only updates the locked, atomically
written desktop registry: no connection check, config rewrite, Auto change or
transfer filtering is involved. Android and Linux choices are currently independent.
Older registries default to General; unknown category strings display as General.
General is omitted on serialization, preserving the old format. Old desktop
versions reject registries containing Photos rather than interpreting an unknown
field; switch all Folders to General before a desktop downgrade.

Photos gives the image grid its own expanding, virtualized scroll area. Folder
properties move into the information popover; pending, failed and non-image
records remain accessible in a compact Transfer activity expander. Existing
filename/type/status filters and sort directions still apply to the full transfer
history, with active matches first. Show More retains pagination, and additional
completed images remain grid tiles rather than becoming list rows after 100.
This is not a full browser of untracked or initialization-only files.

Tiles open a dark, read-only viewer with previous/next navigation, keyboard
shortcuts, zoom, pan and a fullscreen action. Navigation follows a snapshot of the
currently loaded, filtered photos. This initial Linux decoder explicitly permits
PNG and JPEG only; other image extensions still get a tile and an unavailable
placeholder, not a failed transfer. No external image viewer or URL is launched.
Preview reads refuse symlinks (including parent directories), non-regular files,
paths outside the library, size mismatches and SHA-256 changes. Replaced, missing,
corrupt and unsupported files leave the original and transfer status unchanged.

A single process-wide worker and a 128-request bounded queue keep reading, hashing
and decoding off GTK. Destroyed preview requests are cancelled before decoding;
an in-progress decode is not forcibly interrupted. Unmapped cells do not poll or
start decodes; visible requests retry queue saturation until admitted or unmapped.
The input cap is 32 MiB and the declared dimension cap is 16 million pixels.
PNG/JPEG headers are checked before invoking the native decoder, as some system
backends decode before emitting the size callback; the callback checks again.
Output edges are at most 256 pixels for tiles and 1024 for modal previews.
Unchanged gallery entries retain their objects during transfer activity; recycled
cells reuse their widgets. A 24 MiB process-local LRU pixel cache rechecks safe
paths, size, inode and nanosecond modification/change times before reuse. Visible
images can retain shared pixels beyond cache eviction, so this is not a total RSS
limit. There is no disk cache or thumbnail written to any sync directory. Decoder scaling follows
[GdkPixbuf's size-prepared API](https://docs.gtk.org/gdk-pixbuf/class.PixbufLoader.html).
These are bounded-input safeguards, not a codec sandbox or a peak-RSS guarantee;
native decoders can allocate intermediate buffers. No media conversion, deletion,
new background service or server API was added.

Existing pairing creation requests reject unknown fields. Category exchange needs
an explicit backwards-compatible capability/metadata design; this slice deliberately
does not send category fields to old servers or change pairing confirmation.

## Regression notes

The initial test runs exposed an error-summary text regression, a merged image
semantics test selector, and a real lazy-list index exception while clearing and
repopulating Folder records. The error summary now stays separate from the filter
count; image assertions use the unmerged image node and also check actual decoding.
The drawer captures an immutable list per composition and recreates its positional
layout state when membership changes, while retaining stable per-Folder keys.
A device regression exercises 12 empty/repopulate cycles with the drawer both
visible and off-screen. See Android's [lazy-list identity guidance](https://developer.android.com/develop/ui/compose/lists#item-keys)
for the underlying state/key model; the specific failure was observed in our
device test, not established as a framework-wide bug.

Image sampling limits and cache/concurrency bounds are implementation safeguards,
not a measured peak-memory or large-library performance certification. The
[physical follow-up](phone-categories-2026-09-11.md) covers in-place upgrades,
real directory uploads and cold-start previews. Extended OEM/background behavior,
overnight usage and full source-library rendering remain outside these tests.

### Preview-fix verification — 2026-09-11

The fix APK `ec961ccf2d1ff7436057618d322e21c4f3fddf1ed8f73387422a86db4256c84d`
passed 69 JVM tests and Lint (0 errors / 11 existing warnings). Emulator runs and
corrected UI reruns cover all 64 ordinary cases, plus a separate cross-process
preview regression and both manual/Auto SIGKILL resumptions. The final 29-case UI
group passed in `android/.local/device-qa-e65cxt1_/`, including actual selected-icon
pixels in both themes and real-upload preview cleanup. Earlier failed test-harness
attempts are retained, not relabeled as passing. On the phone, two new image
fixtures reached Linux with matching hashes/receipts; the valid image still renders
after cold start and staging cleanup. Existing data/settings were preserved.
See the physical report for per-run evidence, cleanup and limitations.

### Initial emulator verification — before the physical findings

Final isolated report: `android/.local/device-qa-_dxwgk4l/` (runner exit 0).
APK SHA-256: `5e8b2a6dbe95a681eca4da663cc73eb9a9e3347cfb075efcc1cdd732c4bd6d80`.

- JVM: **63 passed**, including database upgrades from versions 1–4, independent
  presentation/consent, stale filter previews, retained versions, and bounded/native
  bitmap decoding. Lint: **0 errors, 11 existing warnings**.
- API 36 dedicated emulator: **62 passed** — directory sync 8, runtime 14,
  delivery Auto 12, UI 27, notification denial 1. Includes the 12-cycle drawer
  regression, original/corrupt image previews, category switching, landscape,
  large text, and explicit filter widening after image-only initialization.
- Both additional SIGKILL/restart scenarios passed: manual delivery and delivery
  Auto. These are two scenarios (four seed/verification test invocations), not
  physical power-cut tests or a new directory-specific crash matrix.
- Real isolated Linux clients checked **52 synchronized paths**, **11 legacy
  deliveries**, and **1 paired delivery-only file**, with hashes and receipts.
- No `FATAL EXCEPTION`, `Fatal signal`, `ANR in`, or `IndexOutOfBoundsException`
  match was found in the final collected log. Earlier failing reports are not
  counted as passing runs.
- Inspected original `visual/photos-grid.png`, `photos-preview.png`, and
  `photos-filter-preview.png`. Screenshots wait for native dialog fades; no
  screenshot or supplied brand asset was edited to hide rendering defects.
- The emulator was stopped after verification. No phone installation, live-server
  upgrade, Git commit, or push was performed.

The final device command was `python3 android/scripts/device-tests.py`. Gradle ran
`:app:testDebugUnitTest :app:lintDebug :app:assembleDebug :app:assembleDebugAndroidTest`
with one worker and a 768 MiB build heap, before starting the emulator. Rust source,
server protocol, native libraries and production data were unchanged in this slice.

### Linux verification — 2026-09-11

- `CARGO_BUILD_JOBS=1 cargo test --locked --features desktop --all-targets --quiet -- --test-threads=1`:
  **208 passed**, 10 opt-in tests ignored by the default run. This includes PNG/JPEG
  decoding, pre-decoder dimension limits, truncated headers, unsupported content,
  changed/missing files, symlink/path escape rejection, old-registry defaults and
  category persistence without config/Auto changes.
- Separately enabled GTK preview test passed on X11: 160 rapid preview replacements
  followed by a successful asynchronous image decode, with original bytes preserved.
- File and sidebar interaction smoke checks passed. The file check now switches
  General → Photos → General offline, checks persistence, keeps pending images as
  rows, verifies active transfers precede the gallery, and retains thumbnail widgets
  during progress updates.
- Four X11 desktop resilience checks passed: broken-config isolation, screenshot
  error reporting, SIGKILL/reopen at four startup timings, and an idle sample.
  The 8-second debug idle sample (two General Folders, no transfer) measured roughly
  0.375% of one CPU core and 190 MiB final RSS. This is not a Photos-library benchmark.
- The separately enabled real tus → isolated GTK directory Auto test passed,
  including version updates, retained conflict copies and restart recovery.
- `cargo clippy --locked --features desktop --all-targets -- -D warnings` and
  `git diff --check` passed.
- Inspected the original 820-pixel-wide Photos screenshot at
  `/tmp/mirelay-photos-qa-W4ypu9/photos.png`. Temporary filesystem fixtures use no
  production credentials and are separate from the real app registry.

Earlier runs are not counted as passing: a disconnected Broadway display could
not produce screenshot render nodes (X11 reruns passed); an oversized PNG assertion
revealed that the host's Glycin-backed loader could decode before the size callback,
leading to the explicit pre-decoder header checks. One parallel core run reported
`Another receiver owns this directory` in an existing recovery test; its isolated
rerun and both subsequent serial suites passed. The cause of that intermittent
parallel lock failure is not established by these tests.

These changes do not install a phone APK, upgrade a relay, rewrite a live sync
configuration, or constitute a full physical-device / large-library certification.
No commit or push was performed.

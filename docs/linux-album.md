# Linux album

Current behavior: the album automatically uses a regular scrollbar on Cairo to
avoid the delayed fade's repeated photo-wall redraws. GL/Vulkan retain the initial
overlay preference. The check uses the actual native renderer on each map, not
an environment-variable guess, and introduces no polling or system-setting change.

The [scrollbar animation control](#cause-identified-scrollbar-fade-not-steady-idle--2026-09-12)
identifies delayed overlay-scrollbar fading as the trigger for the post-scroll
Cairo CPU spike. The earlier short samples included active painting and were
incorrectly labeled idle. The [Wayland renderer comparison](#real-wayland-renderer-comparison--2026-09-12)
recorded much lower short-window CPU on the default Intel Vulkan path.
Earlier observations are retained below with this correction, not erased.

Photos is a local presentation choice, independent of transfer policy. Choose
Photos above the Folder name. Folder identity, tokens, pairing, tus, SHA-256,
receipts and directory conflict preservation are unchanged.

## Design

The existing Folder sidebar stays in place. A compact photo wall replaces the
long property-first page; Folder details are available through the information
button. Transfer activity has a separate, collapsible panel. Completed photos
have no repeated Receive buttons or permanent filename captions; hover/focus
metadata and accessible labels identify them. Existing search, filters, ascending/
descending sort and Show More pagination remain available.

The visual direction combines [Loupe's image-first viewer](https://apps.gnome.org/Loupe/)
and [viewing interactions](https://help.gnome.org/loupe/viewing-images.html) with
[Immich's Folder organization](https://docs.immich.app/features/folder-view/).
This is a native GTK implementation, not copied upstream code, branding or artwork.
It uses [GtkGridView's recycling model](https://docs.gtk.org/gtk4/class.GridView.html)
while retaining the existing GTK 4.6 API baseline.

Tiles are square, center-cropped previews in a responsive 2–8-column grid.
The dark viewer shows the uncropped preview. Use Left/Right to navigate, +/− or
Ctrl+scroll to zoom, 0 to fit, drag to pan, double-click to toggle zoom, F11 to
toggle fullscreen and Esc to leave fullscreen/close. Pinch zoom is also wired.
Navigation uses a snapshot of the current loaded/filter/sort result, not the
entire filesystem. Viewer zoom is bounded to 1–4× and uses an optimized preview,
not a full-resolution editing surface.

## Work and safety bounds

- Only tracked local delivery/directory records are browsed. Untracked files and
  initialization-only files are not newly indexed. No new automatic scanning,
  network request, service or thumbnail file is introduced.
- One worker, a 128-request queue and a 24 MiB LRU decoded-pixel cache. Unmapped
  cells stop their result timers and abandon queued work. Recycled GTK list items
  reuse their preview widgets. Idle completed thumbnails have no polling timer.
- PNG/JPEG decoding only, with 32 MiB input and 16-million-declared-pixel limits;
  tiles are at most 256 pixels per edge, viewer previews at most 1024. Unsupported
  image formats show a placeholder and are not marked as failed transfers.
- Preview cache misses check SHA-256. Hits still reopen using the safe root/path
  reader and compare size, device/inode, and nanosecond mtime/ctime. Missing,
  changed, corrupt, oversized and symlinked files cannot turn a cached hit into a
  new unsafe read. Existing displayed pixels are not continuously revalidated.
- Native decoding is not a sandbox and an in-flight decode is not forcibly
  interrupted. Cache capacity is not a total memory limit: GTK textures, visible
  shared pixels, model metadata and decoder allocations also consume memory.
- Optional, unconfigured wallpaper commands no longer inflate the error count.
  Actual failed or uncertain wallpaper actions still need attention.

## Verification — 2026-09-12

Tests use disposable data only. No production server, phone, registry, originals
or system desktop settings were changed. Initial graphical runs stopped receiving
desktop frames; rendering checks were continued on an isolated Xvfb display with
the Cairo renderer. This is software-rendered debug testing, not release/GPU FPS
certification. Screenshots were inspected at 1100 and 820 pixels wide.

The ordinary desktop-feature suite passed **212 tests**. Opt-in checks additionally
cover real grid rendering and reuse, 1,000/10,000 model entries, pagination identity,
40 clear/repopulate cycles, missing previews, empty/out-of-range viewer inputs,
repeated disabled navigation, keyboard navigation and zoom controls. The existing
rapid-preview-replacement regression passed. Cache tests cover mutation, deletion,
symlinks and eviction while pixels remain visible; decoder safety tests remain.
The final grid/viewer run also passed with `G_DEBUG=fatal-criticals`; Clippy with
warnings denied and `git diff --check` passed.

The real GTK process passed file/sidebar interactions, 25 invalid targets and stale
selection guards, 1k/10k/50k history stress, 100/1k Folder stress, corrupt config
isolation, screenshot I/O failure, and SIGKILL/reopen at four startup timings.
A real local tus → GTK automatic directory receiver passed version updates,
conflict retention and restart. These checks do not contact the production relay.

Representative measurements (debug, Xvfb/Cairo):

| Scenario | Result | Scope |
| --- | --- | --- |
| 10,000 photo model entries | 27 ms to update model; 257 bound picture widgets, 48 mapped/loaded | 32 distinct small local PNGs reused across records; excludes later paint |
| Normal scrolling, 120 × 48 px | 16.7 ms median frame interval; 33.3 ms p95 | Synthetic 10k-entry grid, not full-resolution photos |
| 80 large scroll jumps | 50.0 ms p95 frame interval | Deliberately cold/recycled views; not sustained 60 FPS |
| Actual album app idle | 0.25% of one core over 8 s; RSS stable at 201,684 KiB | Separate desktop process, 120 PNG records, first page |
| Actual ordinary app idle | 0.13% of one core over 8 s; RSS stable | Two Folders, no active transfer |
| 50,000-record history | 133 ms median name sort; 21.9 ms alternating first-page GTK update | Existing general-list model, not 50k decoded images |

An unresolved diagnostic is explicitly retained: after repeated scrolling in the
in-process 10k grid harness, idle CPU varied substantially and reached about 75%
of one core, despite zero pending visible previews and no remaining preview poll
callbacks. Sampled native thread stacks were in GLib thread-pool waits. This alone
does not establish a GLib defect or explain the cause. The separately launched
120-image app did not reproduce the high idle sample. Large-library post-scroll
idle therefore remains **unverified**, not a passing performance claim.
In the final repeated run, the initial two-second sample was 21.9%; a subsequent
three-second sample after ten additional settling seconds was 0%. Earlier late
samples were not consistently low, so this repeat is not being called a fix.

Full-resolution mixed-format libraries, sustained GPU/Wayland scrolling, actual
window-manager fullscreen transitions, physical touchpad gestures and extended
soak testing are not certified by these checks.

## Reproduce

```bash
CARGO_BUILD_JOBS=1 cargo test --locked --features desktop --all-targets -- --test-threads=1
CARGO_BUILD_JOBS=1 cargo clippy --locked --features desktop --all-targets -- -D warnings

# A working graphical session is required for the following commands.
cargo test --locked --features desktop --lib album_virtualization_viewer_and_performance -- --ignored --test-threads=1 --nocapture
cargo test --locked --features desktop --lib gtk_preview_decodes_and_survives_rapid_replacement -- --ignored --test-threads=1 --nocapture
cargo test --locked --features desktop --test desktop_resilience --test desktop_directory -- --ignored --test-threads=1 --nocapture
```

For screenshots, set `MIRELAY_ALBUM_QA_DIR` to an existing private output directory
when running the grid test; it writes `grid.png` and `viewer.png` there.
For an interactive 120-image playground, see [frontend development](../dev/frontend/README.md).

## Continued testing — 2026-09-12

The follow-up ordinary suite passed **213 tests**, including a new album fixture
test that checks selection, hashes and non-overwrite behavior. Clippy with
warnings denied, formatting and diff checks passed. No production behavior was
changed during this follow-up: additions are test code, fixtures and documentation.

New opt-in lifecycle checks passed with fatal GTK criticals enabled:

- 20 stale-result invalidation cycles cannot overwrite a recycled missing cell.
- Unmapping cancels its result timer; remapping resumes and displays the image.
- 20 open/close cycles release the viewer window, checked with weak references.
- Removing a previously cached original produces the unavailable placeholder.
- The existing 10k-grid viewer test now also asserts release of its closed window.

The local tus → GTK directory/version/conflict/restart test and all five isolated
desktop resilience tests passed again. The ordinary 120-image application sample
was 0.125% of one core over eight seconds with unchanged RSS (201,424 KiB).

The CPU investigation now includes independent controls:

- Holding the GLib main-context ownership guard throughout the 10k test did not
  remove the transient high sample; this experimental change was discarded.
- Decoder-only control, with no GTK window: 32 local PNG requests completed;
  four subsequent two-second CPU samples were approximately 0.50%, 0%, 0%, 0%.
  This does not reproduce the same magnitude of CPU activity without the grid.
- The 10k harness still reproduced transient samples, including 120% of one core
  (multiple native threads can exceed 100%). Later three-second samples in these
  reruns were 0%. These are diagnostics, not performance pass thresholds.
- Real process, initial page of the 120-image fixture: 240 injected wheel events
  changed the photo wall; subsequent CPU samples were 0% and RSS was stable at
  207,064 KiB. This initial page does **not** adequately stress cell recycling.
- Real process, fresh 600-image fixture with all 602 records loaded using six
  Show More clicks: an initial sample after pagination reached **86% of one core**,
  then measured 0.50% just after the first scrolling run and 0% after settling.
  RSS after scrolling/settling was 215,184 KiB. Fixture bytes were unchanged.
- A second 600-image run used 120 consecutive downward wheel events followed by
  120 upward events, traversing beyond the prebound-cell buffer. Samples were
  85.0% after pagination, **62.5% immediately after scrolling**, and 0% after ten
  additional settling seconds. RSS was 259,868 → 259,856 KiB across the final
  samples. Window pixels changed and all protected fixture files remained intact.

Consequently the high transient CPU is **not limited to the in-process test
harness**. The full-pagination real-app run reproduces it. Root cause remains
open; current evidence is insufficient to blame GLib, the decoder or GTK itself.
This is not marked fixed or certified for large-library performance.

The first decoder-only run could not load its image inside the restricted test
environment; the normal-host rerun passed. The first X11 automation attempt
selected a hidden application window and failed when setting focus. The harness
now requires a mapped, correctly sized window with the child's exact PID and
handles X11 errors so it can clean up. These failed setup attempts are retained
as setup failures, not hidden or counted as application passes.

```bash
cargo test --locked --features desktop --lib album_preview_lifecycle_faults -- --ignored --test-threads=1 --nocapture
cargo test --locked --features desktop --lib album_decoder_idle_control -- --ignored --test-threads=1 --nocapture
```

The real-process X11 diagnostic and 600-image fixture instructions are in
[frontend development](../dev/frontend/README.md). This remains software-rendered
debug testing; no physical touchpad/fullscreen/window-manager or long-soak claim
is added.

## Real Wayland renderer comparison — 2026-09-12

At the user's request, tests ran on the **right-hand eDP-1 display**. No system
display/driver/animation setting was changed, and no global mouse or keyboard
events were injected. The test uses GTK's monitor-targeted fullscreen request,
asserts the actual monitor and fullscreen state, and prints the selected GSK
renderer instead of assuming that an environment variable enabled acceleration.
Every test window was closed at the end.

All completed comparison runs used the same 1920×1080 logical surface, reported
scale factor 2, and native Wayland session on this machine. The same 10,000-record
model references 32 small local PNGs: this is a reproducible renderer comparison,
not 10,000 distinct full-resolution photographs. There were 257 bound picture
widgets and 72 mapped/loaded pictures in each run. Inputs, scrolling pattern,
preview-poll assertions and viewer lifecycle checks were unchanged.

Hardware identification was checked locally, not inferred from installed driver
names: a realized Wayland GL context reported `Intel`, `Mesa Intel(R) Iris(R) Xe
Graphics (RPL-P)` and OpenGL ES 3.2 / Mesa 26.0.8. GTK's default-renderer diagnostic
reported `GskVulkanRenderer` and **Using Vulkan device 0**, identified by GTK as
the Intel Iris Xe integrated GPU. Although NVIDIA RTX 4060 and llvmpipe devices
were enumerated too, they were not the selected default Vulkan device.

| Requested / actual renderer | CPU just after scrolling, one-core basis (2 s) | CPU after settling (3 s) | Smooth-scroll frame-clock interval p95 |
| --- | --- | --- | --- |
| OpenGL / GskGLRenderer | 2.50% | 0% | 16.67 ms |
| Cairo / GskCairoRenderer | 102.0% | 0% | 29.85 ms |
| Default / GskVulkanRenderer | 2.50% | 0% | 16.67 ms |
| Default repeat, without driver diagnostic logging | 2.00% | 0.33% | 16.67 ms |

The late sample follows ten additional seconds of settling. CPU can exceed 100%
because several threads contribute; it is not a percentage of the whole machine.
Frame-clock intervals are not an end-to-end presentation-latency or release FPS
guarantee. GPU memory is not included in process RSS. No pending visible previews
or continued preview-result polling were present at the initial idle sample.
All four completed graphical tests passed, including navigation, cache safety,
40 clear/repopulate cycles, viewer release and unchanged source hashes. Formatting,
Clippy with warnings denied and diff checks passed after adding monitor diagnostics.

This comparison **does not support a hardware-fault explanation**. The large
transient CPU activity is associated with the forced Cairo path on this setup;
the default hardware-accelerated path did not reproduce it. This narrows the
previous open question but does not identify the precise Cairo/native-library
cause or certify all GPUs, fallback renderers, release builds or large mixed-photo
libraries. The earlier 600-image real-app test used forced X11/Cairo; the new
matched comparisons use the grid harness under Wayland, not that full-app X11
automation. No application renderer override or production fix was introduced.

The first untargeted Wayland attempt produced initial frames but then no smooth-
scroll callbacks. That run failed and is not part of the passing comparison.
After explicitly targeting the visible right-hand monitor, all four runs received
frames and completed. An initial unsupported `GDK_DEBUG=gl` probe was replaced
with the locally documented `opengl:vulkan` flags for the diagnostic default run;
the final default repeat omitted diagnostic logging entirely.

```bash
# Replace eDP-1 with the connector of a monitor you have agreed to use for testing.
# Only this test window is made fullscreen; no global pointer events are sent.
env GDK_BACKEND=wayland GSK_RENDERER=gl MIRELAY_ALBUM_QA_MONITOR=eDP-1 G_DEBUG=fatal-criticals \
  cargo test --locked --features desktop --lib album_virtualization_viewer_and_performance -- --ignored --test-threads=1 --nocapture
# Repeat with GSK_RENDERER=cairo for the software control.
env -u GSK_RENDERER -u GDK_DEBUG GDK_BACKEND=wayland MIRELAY_ALBUM_QA_MONITOR=eDP-1 G_DEBUG=fatal-criticals \
  cargo test --locked --features desktop --lib album_virtualization_viewer_and_performance -- --ignored --test-threads=1 --nocapture
```

## Cause identified: scrollbar fade, not steady idle — 2026-09-12

The original two-second “idle” measurement checked that MiRelay's preview polls
and the benchmark's own tick callback had stopped. That did **not** establish
that GTK itself had stopped animating. GTK 4.22.4's
[overlay-scrollbar implementation](https://github.com/GNOME/gtk/blob/4.22.4/gtk/gtkscrolledwindow.c)
waits approximately two seconds after scrolling before starting a one-second
opacity fade, checked by a 500 ms conceal timer. The old measurement window could
capture different portions of this delayed work, explaining the variability.

The diagnostic now passively counts the frame clock's `after-paint` signals and
scrollbar opacity notifications. Neither observer requests new frames. It compares
the normal overlay scrollbar with `set_overlay_scrolling(false)` **only on the
test's own ScrolledWindow**; system animations and application defaults remain
unchanged. Both runs used Xvfb/X11, GskCairoRenderer, a 1090×750 surface, 257 bound
picture widgets and 48 mapped/loaded pictures, with the same 10k/32-PNG fixture.

| Four-second post-scroll sample | Overlay scrollbar, normal | Solid scrollbar, test-only control |
| --- | --- | --- |
| CPU, one-core basis | 94.98% | 0% |
| Actual paint signals | 61 | 0 |
| Scrollbar opacity notifications | 49 | 0 |
| Opacity start → end | 1 → 0 | 1 → 1 |
| Later three-second idle CPU | 0% | 0.33% |
| Later paint/opacity changes | 0 / 0 | 0 / 0 |

Both full graphical runs passed. This identifies the delayed scrollbar animation
as the trigger for the measured transient work. Under Cairo, its repainting is
expensive on this scene. The CPU use was real, but it was **not steady idle or
evidence of a perpetually busy background transfer/preview worker**. The earlier
hardware-accelerated runs recorded much lower short-window CPU with normal
scrollbars. Those older two-second samples did not guarantee coverage of the
entire fade either, so they are not an exact full-animation cost comparison.
This does not diagnose every Cairo internal hot function or imply
that software-rendered animation needs no further optimization.

Measurement output is corrected:

- `ALBUM_POST_SCROLL` replaces the misleading early `ALBUM_IDLE` label, extends
  the observation to four seconds to include the complete delayed fade, and
  reports actual paints and opacity changes alongside CPU.
- `ALBUM_SETTLED_IDLE` replaces `ALBUM_IDLE_LATE` and also reports paint/opacity
  counts. A CPU sample with ongoing painting must not be reported as steady idle.
- `ALBUM_ENV` records whether overlay scrolling was enabled.

Negative controls are retained. A standalone C program with **no MiRelay code,
window, image decoder or transfer** downloaded 400 synthetic GDK memory textures.
RGB-to-default-format conversion finished in 76.3 ms, followed by 0.072%, 0.004%,
0.005%, 0.007% CPU samples; a same-format copy control took 6.0 ms and stayed below
0.01% CPU. Merely exercising pixel conversion did not reproduce a stuck pool.
GTK's [parallel task implementation](https://github.com/GNOME/gtk/blob/4.22.4/gdk/gdkparalleltask.c)
and the presence of waiting thread-pool stacks were therefore insufficient to
establish a thread-pool defect. An exploratory `GDK_DISABLE=threads` run reduced
the short sample but did not identify its trigger; no such override was retained.

A late strace attach was rejected by the host's ptrace policy. No kernel policy
was changed; a subsequent trace launched the test as the tracer's child. Its
bounded sample showed native timed waits followed by renewed drawing-related
activity. Traced timings are not used as performance benchmarks.

```bash
# Run only on a dedicated Xvfb display; :2 is the display used in this session.
env DISPLAY=:2 GDK_BACKEND=x11 GSK_RENDERER=cairo G_DEBUG=fatal-criticals \
  cargo test --locked --features desktop --lib album_virtualization_viewer_and_performance -- --ignored --test-threads=1 --nocapture
env DISPLAY=:2 GDK_BACKEND=x11 GSK_RENDERER=cairo MIRELAY_ALBUM_QA_SOLID_SCROLLBARS=1 G_DEBUG=fatal-criticals \
  cargo test --locked --features desktop --lib album_virtualization_viewer_and_performance -- --ignored --test-threads=1 --nocapture

# Standalone negative control, not a reproduction of the full animation issue.
cc -O2 -g -Wall -Wextra -Werror dev/frontend/gdk-idle-probe.c -o target/gdk-idle-probe $(pkg-config --cflags --libs gtk4)
env DISPLAY=:2 GDK_BACKEND=x11 target/gdk-idle-probe convert
env DISPLAY=:2 GDK_BACKEND=x11 target/gdk-idle-probe copy
```

At this diagnostic stage, only diagnostics/documentation changed. No default scrollbar,
renderer, application polling, transfer behavior, system animation setting or
graphics driver was modified.

## Renderer-aware optimization — 2026-09-12

The production album now reads its [native renderer](https://docs.gtk.org/gtk4/method.Native.get_renderer.html)
when mapped and disables [overlay scrolling](https://docs.gtk.org/gtk4/method.ScrolledWindow.set_overlay_scrolling.html)
only for Cairo (or an unavailable renderer, conservatively). This also covers
GTK automatically falling back to Cairo without a `GSK_RENDERER` override.
Non-Cairo renderers retain the widget's original overlay preference. The regular
scrollbar takes a small amount of horizontal space when needed; scrolling and
thumbnail/viewer behavior are unchanged. No global animations, renderer selection,
driver settings, transfer policy, decode safety or cache bounds are changed.

The map handler has no timer, retained parent or per-frame work and re-evaluates
on remap. Regression checks assert the policy after first map and after detach/
reattach, and require no opacity changes and at most two trailing paint signals
in the regular-scrollbar post-scroll sample. CPU measurements remain diagnostic,
not brittle machine-specific thresholds.

The previous section's ordinary run now exercises the optimized product. Use
`MIRELAY_ALBUM_QA_OVERLAY_SCROLLBARS=1` to explicitly reproduce the pre-fix path:

```bash
env DISPLAY=:2 GDK_BACKEND=x11 GSK_RENDERER=cairo G_DEBUG=fatal-criticals \
  cargo test --locked --features desktop --lib album_virtualization_viewer_and_performance -- --ignored --test-threads=1 --nocapture
env DISPLAY=:2 GDK_BACKEND=x11 GSK_RENDERER=cairo G_DEBUG=fatal-criticals MIRELAY_ALBUM_QA_OVERLAY_SCROLLBARS=1 \
  cargo test --locked --features desktop --lib album_virtualization_viewer_and_performance -- --ignored --test-threads=1 --nocapture
```

Post-fix matched results (isolated Xvfb/X11, GTK 4.22.4, Cairo, debug build,
1090×750 at scale 1, 10k records referencing 32 local PNGs):

| Measurement | Explicit old-overlay control | Automatic production policy |
| --- | --- | --- |
| Four-second post-scroll CPU, one-core basis | 45.24% | 0% |
| Post-scroll paint signals / opacity changes | 61 / 49 | 0 / 0 |
| Later three-second idle CPU | 0% | 0% |
| Normal-scroll frame-clock p95 | 33.33 ms | 33.33 ms |

The optimized path passed twice, both with zero post-scroll paints and sampled
CPU. This removes the identified unnecessary redraws; it is not a claim of
literally zero instructions or universally faster active scrolling. Timing varies
with host load, and frame-clock intervals are not measured presentation latency.

Additional verification for this optimization:

- Ordinary desktop-feature suite: **213 passed, 14 ignored**; opt-in graphical
  checks are run separately, not counted as ordinary-suite passes. Formatting,
  Clippy with warnings denied, and whitespace checks passed.
- A separate Xvfb `GskGLRenderer` grid run passed, retained overlays, and passed
  the remap/viewer/40-reset checks. This validates the non-Cairo policy branch,
  not physical GPU acceleration or a new Wayland hardware benchmark.
- The real desktop process loaded all 602 records (600 generated gradients plus
  two fixture images), then received 240 actual X11 wheel events. Window pixels
  changed and protected configuration/state/image hashes stayed unchanged.
  Four seconds immediately after the last wheel event measured **10.25% CPU**;
  after another ten seconds, the three-second idle sample was **0%**. RSS was
  260,624 KiB immediately after scrolling and 260,408 KiB at settled idle.
  Unlike the grid benchmark, the immediate real-app sample does not first wait
  for previews/scrolling to settle. It still contains work and is not a claim
  that all post-input cost has been removed; its remaining work was not profiled
  in this pass. Older two-second real-app samples are not exact comparisons.
- The 820-pixel-wide real-app screenshot retained a usable three-column photo
  wall, scrollbar, pagination and controls; no overlap was observed.
- Recycled-preview stale results, unmap/remap, repeated viewer close and deleted
  cached-source checks were rerun under Cairo with fatal GTK criticals.

These checks used disposable local fixtures and an isolated display. No production
phone/server data, user desktop settings or globally injected input were involved.

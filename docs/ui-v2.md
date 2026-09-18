# Native UI v2 integration

The supplied desktop prototype is the visual reference. Mobile prototypes are
older; the handoff's folder-category icons and three-column seamless grid take
precedence. The applications remain GTK/libadwaita and Jetpack Compose, not WebViews.

## First native pass

- Static sage, pearl and cream light bands under translucent native surfaces.
- Desktop: 208 px initial sidebar, 14 px outer gutter, 17 px panel corners,
  28 px content alignment, original wordmark at the top left, graphite icons,
  restrained selected rows and unboxed file list.
- Folder details move into the existing **Folder details** popover for both
  General and Photos. Source, local path, device, size limit, automatic receive
  and wallpaper status remain available. **Folder settings** is unchanged.
- Both photo grids use exactly three columns and zero gaps. Virtualization,
  bounded decoding/caches, stable item identities, pagination, original-aspect
  viewing, navigation and error/activity access are retained.
- Android uses the original brand artwork and native vector icons, with a quiet
  selected-folder surface. Only a confirmed `ready` connection displays Paired;
  legacy, unchecked, pending and disconnected states do not.
- Existing system dark appearance and Android low-memory/power-save glass
  fallback remain supported. Error and warning colors remain meaningful.

## Linux brand refinement — 2026-09-15

The sidebar now uses an offline, deterministic transparent derivative of the
approved wordmark, without a white button surface. Only border-connected white
canvas pixels change alpha; original dimensions and RGB pixels are unchanged.
The complete original remains in About. Linux application-menu and embedded icons
also use transparent derivatives of the original sticker, in all eleven sizes.
See `assets/ui/v2/brand/README.md` for regeneration and preservation checks.

The workspace header no longer repeats a centered Folder/General/Photos title.
The Folder name and category remain in the content heading, with native window
controls, Receive, Refresh, Folder settings and the compact drawer retained.

## Function preservation

| Capability | Retained entry point / implementation |
| --- | --- |
| Multiple Folders and category changes | Sidebar/drawer and existing category controls |
| Sort, filter, search, active-transfer priority | Existing Linux Folder/File controls and models |
| Create credentials, manual/QR pairing, explicit confirmation | Existing Folder settings panels |
| Local path and existing directory initialization | Existing native pickers and preview/consent flow |
| Receive/send, pause, retry, delivery receipts | Existing toolbar, transfer rows and activity panels |
| Auto and network constraints | Existing Auto/Sync settings; no new background workers |
| Disconnect vs local removal | Existing guarded Folder settings actions |
| File safety, hashes, tus, authorization and conflict history | No transport or state-store changes |

## Android page and brand refinement — 2026-09-15

- General Folders use one rounded content surface, with a compact category/name
  heading and a guarded, expandable **Folder details** region for server and
  selected-directory metadata. Details are scoped to the current Folder.
- Both page types use a 64 dp top bar and a smaller transparent wordmark. The
  album retains its three-column, gapless grid, original viewer and activity menu.
- The Folder drawer has a close action and a quiet selected-row check. Its solid
  surface prevents background file names from ghosting through; no full-panel
  blur or extra animation loop was added.
- User choice: keep the system font, including OEM custom fonts. Font scale,
  Reduce transparency, Background motion, Folder category and Auto preferences
  are not changed by this update. Settings explicitly explains when Reduce
  transparency is suppressing the light bands.
- The transparent wordmark is pre-sized to 512 × 320: about 640 KiB of RGBA
  pixels versus about 6 MiB for the original, excluding renderer/cache overhead.
  The original wordmark/icon resources remain untouched. Regenerate the bounded
  derivatives with `python3 scripts/prepare_android_brand.py` (Pillow required).
- The launcher uses the transparent sticker foreground over an opaque pale-sage
  background, retaining its existing safe-area inset. Android launchers still
  apply their own masks; this is not a promise of a fully transparent home-screen
  icon. See [Android's adaptive icon documentation](https://developer.android.com/develop/ui/compose/system/icon_design_adaptive).

This is a presentation-only update: no database migration, credential/signing
change, new background worker, or transport rewrite.

Validation of the refined debug APK on 2026-09-15:

- Build, alignment and lint passed (0 lint errors, 16 warnings). JVM tests:
  102 passed, 1 conditional fixture skipped. Shared/Android brand checks:
  7 passed, including original-pixel preservation and bounded decoding.
- Dedicated emulator: all 43 UI cases and the appearance frame test passed.
  Seven isolated Android-to-relay-to-Linux deliveries matched bytes, hashes and
  acknowledgements; a disconnected receiver was blocked. This run did not
  repeat directory-sync or process-kill recovery acceptance.
- Software-rendered emulator, debug APK, warm filesystem caches: five process
  starts took 1,045–1,182 ms. The default empty/static page used 0.07 CPU seconds
  during a 10.09-second idle sample. These are not physical-device frame-rate,
  populated-album or battery-life measurements.
- The physical phone received a same-certificate in-place debug update. The
  installed APK hash matched the tested build. Folder/transfer records,
  tracking tables, Auto configuration and appearance preferences were retained;
  only the expected periodic scan timestamp was excluded from equality checks.
  Main-page rendering and details/drawer navigation were checked with the
  user's system font. No test Folder was created on the phone.
- APK SHA-256:
  `d14a64a80257abdca07cc3b9cf364feec3b9e6f0d2c5ffd07017044e70d4a978`.
  Local reports: `target/ci-reports/mobile-refined-final-build-0915/`,
  `android/.local/device-qa-7182k7_9/results.json`, and
  `android/.local/device-qa-ayygw35u/performance.json`. The owned QA emulator
  was stopped after testing. This is not full release or production acceptance.

## Android appearance and transfer layout — 2026-09-15

Folder removal discoverability follow-up: **Remove locally / Remove Folder**
now uses a full-width, minimum-48-dp outlined destructive button with a trash
icon, rather than an unboxed text action. A visible caption explains that
original files stay on the device. Disconnect is also outlined; paired Folders
explain why removal stays disabled until disconnection succeeds. Confirmation,
cancel behavior, file preservation and system fonts are unchanged.

Follow-up validation: 102 JVM tests passed (1 fixture skipped), lint had 0 errors
and 16 warnings. Five targeted emulator cases passed, including cancellation,
file preservation and disconnect-failure guards; three isolated deliveries
matched bytes/hashes/acknowledgements. The sharing test now waits for its
asynchronously opened confirmation window instead of asserting immediately.
Final reports: `target/ci-reports/mobile-remove-button-verified-0915/` and
`android/.local/device-qa-q1zq7jvt/results.json`. Earlier attempts recorded a
test-selection dependency in the harness and the confirmation-window timing
failure; they are not counted as passing runs. This follow-up APK has not been
installed on the physical phone.

- Global **Settings → Appearance** is available from the Folder drawer and
  album options. Existing Reduce transparency preferences keep their original
  storage key; no Folder, credential, transfer or Auto setting is migrated.
- **Background motion** is opt-in and defaults off. Three cached gradient pairs
  move on 12/15/14-second curves with a 22-second palette crossfade. Only the
  separate decoration draw layer reads animation time; lazy file/photo content
  does not read it. Redraws are capped at 30 Hz, not a promised device frame rate.
- Motion stops when the app is not resumed, the window loses focus, battery
  saver is enabled, system animator scale is zero, hardware acceleration is
  unavailable, the device is low-memory, or Reduce transparency is on. Settings,
  lifecycle and power changes are observed without polling or a new service.
- Reduce transparency uses solid surfaces and disables both ambient motion and
  album-header glass. The preference applies to all Folders on this device.
- General transfers are grouped as **In progress**, **Needs attention**,
  **Waiting for Linux**, and **Transfer history**, retaining each record once and
  its existing actions. Grouping is linear, with stable order within each group.
  A server upload is not relabeled as received; delivery-only files do not
  promise unavailable Linux receipts. Empty-file progress is finite.

## Linux appearance — 2026-09-15

- **Settings** at the bottom of the sidebar opens app appearance settings.
  The original **Folder settings** button and Ctrl+, shortcut still open the
  Folder editor; the new window action is `win.appearance`.
- Background motion defaults off. An independent native GTK backdrop reuses
  six radial-gradient nodes, changing only transforms/opacity on the frame clock
  with a maximum 30 redraws per second. It does not reload CSS, rebuild file rows
  or take framebuffer screenshots during animation. The three motion periods
  are 12/15/14 seconds, with a 22-second palette crossfade.
- Unmapped/hidden windows, inactive windows, disabled GTK animations, high
  contrast and Reduce transparency remove the animation callback. High contrast
  and Reduce transparency also use opaque panels. Global settings notifications
  are disconnected with the backdrop widget. Complete compositor occlusion
  while the window remains active is not detected or claimed as tested.
- A read-only power monitor supports both the modern
  `org.freedesktop.UPower.PowerProfiles` and legacy `net.hadess.PowerProfiles`
  interfaces. Power saver, initial detection, unavailable services and unknown
  profiles pause motion. Balanced/performance permit it only when all other
  appearance gates allow it; the saved motion preference is never changed.
  Either interface reporting saver takes priority; otherwise the modern
  interface is preferred, falling back to legacy only when modern is absent.
- Power monitoring subscribes to D-Bus changes without periodic polling,
  activating services or changing system policy. Startup has a three-second
  deadline; explicit invalidated-property reads have a one-second timeout and
  discard stale replies. Service owner loss/reappearance is observed automatically.
  A private connection prevents bus loss from exiting the app. After a whole-bus
  disconnect, motion stays paused; **Settings → Refresh power status** reconnects
  on demand. Systems without either service retain the static background.
- Settings use `appearance.json` next to the selected registry, covering all
  Folders in that local profile. No file is created just by starting the app.
  **Save** writes atomically on a short-lived background task; it does not block
  the GTK thread on filesystem sync. Duplicate saves and closing mid-save are
  guarded. Invalid, oversized, unsupported-version and non-regular files are
  rejected, not replaced with defaults. Unknown JSON fields are preserved.
- This does not change credentials, Folder registration, automatic receive,
  wallpaper behavior, pairing or transfer state. On save failure, the dialog
  shows an error and permits retry; a post-rename durability failure can mean
  that the file changed even though saving could not be confirmed.

## Deliberately not yet claimed complete

GTK does not perform a full-window framebuffer blur. Both clients use native
material approximations, not pixel-identical browser backdrop-filter implementations.

Remaining mobile prototype refinements, physical-display frame pacing and
power profiling remain follow-up work. The isolated GPU sample below measures
backdrop rendering plus readback, not full-application display performance. This visual change
does not authorize installing over a phone, touching production data, deploying
the server, signing a release or pushing to GitHub.

## Local preview

Use a disposable fixture as described in `dev/frontend/README.md`; never point
UI smoke checks at production folders. The full supplied offline prototypes can
be extracted into `dev/frontend/MiRelay-design-kit-v2/` (ignored).

For isolated Android QA, the emulator must reach loopback services through
`10.0.2.2`. On a host with proxy environment variables, start the repository AVD
without inheriting them (this does not change host proxy configuration):

```sh
env -u http_proxy -u https_proxy -u HTTP_PROXY -u HTTPS_PROXY \
    -u ALL_PROXY -u all_proxy bash android/scripts/emulator.sh start
```

If the AVD is already running, stop only the repository-owned AVD using
`android/scripts/emulator.sh stop` before restarting. The device test runner
resets this dedicated AVD's MiRelay data; never configure real Folders there.

## Verification — 2026-09-14

- Desktop-enabled Rust suite: **222 passed, 18 opt-in/helper tests ignored**.
  Includes real local server/client delivery, tus continuation, binary/non-image
  files, retry handling, directory locks and process-kill recovery. This is not
  a repeat of phone-to-production-server acceptance.
- Fixture suite: **4 passed**; native Folder sidebar and file interaction smoke
  checks passed using disposable local-only registries.
- Separately opted into native layout/category-switch tests (1000 and 820 px),
  complete-artwork About checks in both themes, and album virtualization/viewer
  checks. Source/path/device/limits/Auto/wallpaper properties remain mapped in
  the details popover after repeated General/Photos switches.
- Album stress uses 1,000 and 10,000 records referring to 32 distinct generated
  PNGs, not 10,000 unique real photos. It retained **97 picture widgets**, checked
  actual square cell allocation, and passed 40 clear/repopulate cycles. In the
  final debug Broadway sample, settled idle observed **zero paints and zero
  sampled CPU ticks over about 3 seconds**. This is not a real desktop GPU FPS,
  battery-use or release-build performance guarantee. The independent test
  process excludes browser CPU from its measurement.
- Android debug APK and instrumentation APK build passed. JVM/Robolectric:
  **92 passed, 1 conditional skip** (Linux-generated QR fixture not supplied).
  Includes native rasterization of all three new vector drawables, confirming
  visible outlines rather than solid/missing icons. Lint: **0 errors, 17 existing
  warnings**. Instrumentation tests were compiled, not run on a device this turn.
- Debug APK inspection retains package `io.mirelay.android`, the existing debug
  certificate, and 16 KB alignment. No private release key was created or used.
- Icon generation checks: **3 passed**. Rust formatting and diff checks passed.

Visual review used actual GTK screenshots of the file page at 820 px and the
seamless album at 1000 px. Source screenshots and command logs remain in ignored
`dev/frontend/.local/ui-v2-*` and `target/ci-reports/ui-v2-*` directories. See
`docs/validation/ui-v2-native-2026-09-14.json` for the evidence index.

Issues caught and fixed during this pass: a stale brand-test constant, an unused
Compose measurement wrapper, GTK symbolic paint/line conversion, and fixed-height
photo cells that prevented a seamless grid. A native layout-test assertion also
mistook the returned error-close button for Add Folder; it now checks the actual
Folder action. Initial graphical runs forced Cairo on Broadway and encountered
Broadway JavaScript/node-serialization failures; restarting the isolated display
with its native Broadway renderer allowed screenshot and interaction checks.
Those failed attempts are retained and are not counted as passing checks.

## Verification — 2026-09-15

- Final Android debug and instrumentation APK builds passed. JVM/Robolectric:
  **100 passed, 1 conditional skip** (external Linux QR fixture not supplied).
  Lint: **0 errors, 16 warnings**. No release key was created; the original debug
  certificate and 16 KB APK/native-library alignment were verified again.
- Dedicated Android 16/API 36 x86_64 AVD: **26 selected instrumented tests passed**
  (one real-frame appearance test, eleven UI tests, fourteen runtime tests).
  Covers settings persistence across Activity recreation, existing preference
  preservation, both icon themes, Folder switching, landscape/large text/small
  windows, album glass fallback, original preview/category changes, zoom/paging,
  corrupt images, and deleted/revoked directory sources.
- The real-frame test observed changing background pixels with motion enabled,
  then zero changed sampled pixels after disabling system animations at runtime
  and after disabling motion in Settings. It restores the system setting in
  `finally`. Background/focus/power/low-memory gates also have pure policy tests;
  these are not physical-device power measurements or proof of a GPU frame rate.
- Runtime tests exercised JNI callback failures, Keystore/SQLite safety, bounded
  provider input, rejected TLS, authentication failures and tus pause/resume.
  **One synthetic Android → loopback server → Linux delivery** passed exact byte,
  hash and acknowledgement checks. Pairing, bidirectional directory sync, Auto
  process-death recovery and production networking were not rerun in this slice.
- No fatal exception, fatal signal or MiRelay ANR marker appeared in the final
  captured logcat. This is a bounded regression run, not a crash-free guarantee.
- Reviewed final native screenshots of Settings, album glass and ambient bands.
  Some older screenshot calls can capture platform dialog-window fades after
  Compose reports idle; the reduced-transparency screenshot is one such transient
  capture and is not treated as pixel-level visual acceptance. The fallback and
  persistence assertions themselves passed. The new Settings capture explicitly
  waits for the window fade.

Three initial attempts stopped at network preflight before installing the new
APK; their archived screenshots may be leftovers from earlier AVD runs and are
not current evidence. Restarting the owned emulator without inherited proxy
variables restored `10.0.2.2` access. The first connected run caught an ambiguous
test selector matching Settings in both the hidden drawer and visible album
menu. A dedicated album-menu tag fixed it; the final run passed all selected
tests. No host proxy settings or USB phone data were changed.

Evidence: `docs/validation/ui-v2-mobile-2026-09-15.json`, final logs under
`target/ci-reports/ui-v2-mobile-acceptance-*`, and screenshots under
`android/.local/device-qa-cy197nhk/visual`. The owned emulator and isolated servers
were stopped after testing. Linux code was not changed or retested in this slice.

## Linux appearance verification — 2026-09-15

- Desktop-enabled Rust suite: **227 passed, 19 opt-in/helper tests ignored**.
  The five added pure tests cover preference defaults/round trips, preservation
  of unknown fields, corrupt/future/oversized files, symlink/directory rejection,
  all motion gates, and finite periodic curves at extreme timestamps.
- Opt-in native appearance test passed on an owned Xvfb display with Cairo
  rendering, then passed two fresh-process repeats. Real focus events exercise
  active motion, system animation disable, hide/unmap, opening a modal Settings
  window, opaque surfaces, asynchronous Save, recoverable Save failure, and
  unchanged neighboring Folder configuration bytes. No critical GTK diagnostic
  occurred with `G_DEBUG=fatal-criticals`; expected DRI3 warnings reflect the
  software-only test display, not hardware GPU acceptance.
- Initial timed sample: **32 backdrop snapshots / 1.535 seconds**, **zero
  foreground-probe redraws**, **six cached gradient nodes**. Repeat samples were
  32 / 1.550 and 32 / 1.547 seconds. Default-off and system-animation-off sampling
  observed zero backdrop redraws and no installed tick callback. The foreground
  probe overlays the existing empty-state action; this is not a measurement of
  a populated 10,000-photo album, overall idle CPU, release GPU FPS or battery use.
- Separately reran the native layout/category-switch test at 1000 and 820 px:
  icons, Folder settings, Receive and all Folder-detail properties remain mapped.
  The original three-column album grid and repeated General/Photos switches
  passed. Broadway layout rendering reported no JavaScript exceptions.
- Reviewed native Settings, motion/static, opaque-panel and Save-error snapshots.
  Rust formatting, diff whitespace and both QA-driver syntax checks passed.
  No Android rebuild/install, production access, release signing or push occurred.

The first new builds caught Rust visibility and test-helper path errors; both
were fixed. Headless Broadway focus checks were unreliable (one early pass and
later focus failures), so they are not the acceptance evidence for lifecycle
gates. The final motion driver owns an independent Xvfb and focuses only windows
whose PID matches its exact test child. An initial software-rendering assertion
mistook a nominal one-second event loop for one second of wall time; the corrected
check measures actual elapsed time before asserting the redraw-rate bound.
Failed attempts remain in `target/ci-reports/ui-v2-linux-appearance-*`.

Reproduce using the test binary printed by `cargo test --features desktop --lib
--no-run`. For motion/lifecycle checks, run `dev/frontend/appearance-x11-check.py`
with `--test-binary` and `--xvfb` pointing to the repository's isolated QA tools
under `target/`. For layout only, use `node dev/frontend/appearance-check.mjs
<test-binary>`. Neither driver uses the real desktop. Reports and screenshots are
kept under `target/`; owned display/browser/test processes are terminated on exit.
See `docs/validation/ui-v2-linux-appearance-2026-09-15.json` for the evidence index.

## Linux power integration verification — 2026-09-15

- Desktop-enabled Rust suite: **230 passed, 21 opt-in/helper tests ignored**.
  Added pure checks cover known/unknown profile values, alias conflicts and
  missing bus addresses. Existing transfer and filesystem regressions passed.
- Separately opted into a private-bus integration test covering absent services,
  legacy fallback, saver transitions, property invalidation, modern precedence,
  unknown profiles, conflicting aliases, service loss/reappearance, whole-bus
  disconnection, failed reconnection and cancellation during startup. The fake
  provider observed zero writes and no extra property reads during a 200 ms idle
  sample. This is not a battery-use measurement or a delayed-reply race test.
- The native Xvfb appearance test now receives actual callbacks from that fake
  service: saver removes the GTK tick, balanced resumes it, and unknown values
  stop it again without changing the saved preference. Existing Settings save,
  error recovery and neighboring Folder-file preservation checks remain in it.
- A separate read-only probe successfully detected this host's normal power
  profile. The real system profile was not changed. Real GPU/power profiling,
  physical power-mode switching and full compositor occlusion remain untested.

Commands, final native repeats and screenshot evidence are indexed in
`docs/validation/ui-v2-linux-power-2026-09-15.json`. Earlier appearance reports
remain historical snapshots; their then-pending power integration is covered
by this follow-up. No Android rebuild/install, production access, release
signing, commit or push occurred in this slice.

## Adaptive Linux navigation — 2026-09-15

- The minimum main-window width is now 560 px. A native adaptive Folder drawer
  replaces the always-visible splitter at narrow widths; the header's **Show
  Folders** button opens it, and **Hide Folders** or activating a Folder closes
  it. Wide windows show the same sidebar alongside the content. There is only
  one Folder list, so resize does not copy, reload or reset navigation state.
- Wide-mode sidebar resizing remains available through the gutter: drag, or
  focus it and use Left/Right, Home/End. Width is bounded to 208–320 px while
  reserving content space. Folding follows native minimum-size negotiation,
  including the chosen sidebar width, rather than polling the window size.
- Folder sort/filter, Add Folder, app Settings, Folder settings, Receive and
  General/Photos detail properties remain accessible. Refresh/filter-driven
  selection changes do not unexpectedly dismiss an open drawer. The drawer
  uses a solid surface to keep overlaid content readable, and explicit closing
  returns focus to the header toggle.
- Uses the adaptive widget available at the existing libadwaita baseline; no
  dependency upgrade, new background worker or settings schema is required.

Verification and GPU sampling evidence are indexed in
`docs/validation/ui-v2-linux-adaptive-2026-09-15.json`. The opt-in native layout
test exercises 1000, 820, 640 and 560 px, repeat folding, retained search state,
drawer actions, bounded resize handlers and repeated General/Photos changes.
The historical 820 px minimum in earlier evidence is superseded by this pass.

Final regression: **231 Rust tests passed, 22 opt-in/helpers ignored**; two
fresh-process layout runs and one native Xvfb motion/settings/power run passed.
Final layout runs reported no GTK warnings/criticals or browser exceptions.

## Isolated hardware rendering sample

`dev/frontend/appearance-wayland-check.py` runs a private headless Weston with
hardware OpenGL, a temporary private runtime directory and no connection to the
user's display. Weston packages are extracted under `target/`, not installed
system-wide. The helper matches GTK's reported EGL render device to Weston's
hardware device and rejects missing proof or software-renderer diagnostics.
Its local module mapping follows the [upstream Weston implementation](https://cgit.freedesktop.org/wayland/weston/tree/libweston/compositor.c).

The test uses the production native backdrop snapshot implementation and six
cached gradients, at 1000×680 and 560×680. After five warmups it measures sixty
changing frames per size, including snapshot construction, GPU rendering and
explicit texture readback. Nonempty and changing pixels are asserted. Timings
include readback overhead and are neither screen FPS nor GPU-only execution
time. The static-window idle sample runs an event-driven main loop without
test polling; CPU ticks refer only to this fixture process, excluding Weston
and other applications. No transfer worker is installed in this UI fixture.

The headless display cannot supply real keyboard focus, so it is not used to
bypass or certify the production foreground-animation gate. Native focus,
modal Settings, saver transitions and save/error regressions are checked
separately with the existing exact-PID Xvfb driver. No physical GPU power-policy
change, battery draw measurement or live desktop interaction is performed.

Three final debug-profile GPU runs matched GTK and Weston to
`/dev/dri/renderD129` (NVIDIA RTX 4060 Laptop GPU). The 1000×680 per-frame median
was 1.487–1.588 ms (P95 1.774–2.230 ms); 560×680 was 1.085–1.901 ms
(P95 1.353–3.263 ms). Each one-second static sample had zero backdrop redraws;
fixture CPU used 0, 1 and 0 ticks respectively (100 ticks/second on this host).
This short sample does not establish an overall application idle-CPU budget.

Issues found and fixed: sidebar expansion under native size negotiation, an
unavailable close icon, and snapshot capture before popover close had settled.
The first GPU lifecycle attempt had no real focus; a separate offscreen sample
now states that limitation. A draft FFI probe was rejected by the crate's
`forbid(unsafe_code)` rule and removed; the final test uses safe `/proc` reads
and GTK diagnostics without weakening the rule or adding a dependency.

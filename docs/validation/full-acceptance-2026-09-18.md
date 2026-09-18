# Development acceptance — September 18, 2026

**Verdict: not accepted yet.** Most functional paths passed, but the full Android
sequence and quality gates did not. Production and physical-device acceptance
also remain incomplete. No release candidate is approved.

Subsequent authorized local repairs resolved AC-01 through AC-03 and passed
the full 91-case emulator sequence; see the [repair/reacceptance report](local-repair-acceptance-2026-09-18.md).
The failures below remain the historical result of this earlier source state,
not the current local repair status. Production/release gates are still open.

This run used the dirty checkout based on
`fb539cf65cea6e978baf0ad0d0157660954fefe7`, not a frozen commit or clean checkout.
All original input hashes remained unchanged throughout testing. Only private
test helpers/fixtures and these acceptance records were added; application code
was not changed. The baseline input-index SHA-256 is
`5177b036ebfa2d96f056af4a72e4b7849ff4117b6723d11357d40fb78f7866a4`.

[Machine-readable evidence](full-acceptance-2026-09-18.json) contains commands'
report locations, exit codes, source-index identities, log hashes and artifact
identities. Raw diagnostics and synthetic credentials remain private.

## Executed scopes

Counts below overlap across configurations and reruns; do not add them together.

| Scope | Result |
| --- | --- |
| Rust 1.98.1 desktop, all targets | 244 passed, 0 failed, 23 ignored |
| Rust 1.98.1 default, all targets | 135 passed, 0 failed, 2 ignored |
| Rust 1.88.0 desktop, fresh build directory | 244 passed, 0 failed, 23 ignored |
| Concurrent directory ownership | Four cases passed all 25 repetitions |
| Explicit environment-specific Rust tests | 21 ignored tests executed successfully in appropriate separate processes; includes diagnostics/QR fixture export, not 21 ordinary functional cases |
| Installed Linux release | Sidebar/file interaction guards and 1k/10k/50k history, 100/1k Folder stress passed |
| Android build/JVM | Fresh debug/native/test builds passed; 110 JVM tests passed with the explicit Linux QR fixture, no skips |
| Android lint / APK inspection | 0 errors, 10 warnings; debug signature, APK ZIP and four native-library 16 KiB alignment checks passed |
| Android API 36 x86_64 broad sequence | **87 passed, 2 failed**, 89 instrumented invocations total |
| Fresh Android exit/directory control | 10 passed; does not replace the failed broad sequence |
| Ordered Android contamination reproduction | Photo fixture passed, subsequent peer-exit case failed, directory control passed |
| Host JVM/JNI | Input smoke and actual tus → Linux hash/ACK chain passed |
| Python / workflow | 39 script tests, 13 deployment tests and checksum-pinned actionlint passed |
| Rust quality | **Root/native formatting and strict Clippy failed** |
| Production server | Read-only health, trusted HTTPS and SQLite quick check passed; still schema 4, not exit-capable |

The two Rust entries not directly invoked were the cooperating physical optical
QR check and the subprocess crash helper. The latter was exercised by its normal
parent recovery tests; running it directly would not be valid acceptance.

The broad Android sequence verified 14 legacy deliveries, one paired delivery
and 52 synchronized paths through real Linux receivers, with exact bytes/hashes
and receipts. Manual and automatic upload SIGKILL/resume and cold-process image
preview recovery passed. These are emulator results, not physical-phone results.

## Open findings

### AC-01 — quality gates fail

`cargo fmt --all -- --check` and native `cargo fmt --check` report formatting
differences. Strict Clippy rejects the nested conditional in
[power.rs](../../src/desktop/appearance/power.rs) at line 210 (`collapsible_if`).
No formatting or implementation fix was folded into this testing-only run.

### AC-02 — Android test fixture contaminates later clean-exit tests

The broad `DeviceUiTest` sequence ran 46 cases: 44 passed and two failed:

- `cleanExitCancelsSafelyCleansStagingAndWaitsForLinuxReceipt` timed out waiting
  for local cleanup.
- `peerRequestedExitKeepsDirectoryGrantUsedByAnotherFolder` was refused with
  `Old unowned staging data requires review before clean exit can finish.`

[DeviceUiTest.kt](../../android/app/src/androidTest/java/io/mirelay/android/DeviceUiTest.kt)
line 366 directly creates a photo payload without `folder-owner`.
[DeviceSupport.reset](../../android/app/src/androidTest/java/io/mirelay/android/DeviceSupport.kt)
deletes the database rows but leaves that payload behind. The cleanup safety
check then correctly refuses to guess its ownership.

Both exit cases passed in a fresh app-data control. Explicitly running the photo
fixture followed by peer exit reproduced the failure, and an archived staging
inspection confirmed one unowned payload. Fix fixture ownership/isolation and
rerun the entire suite. **Do not weaken production cleanup protection or replace
the full-suite failure with the isolated passing result.**

Private runs: `device-qa-gb1iqswl` (broad), `device-qa-uq4e5fox` (clean control),
and `device-qa-obqocc08` (ordered reproduction), under `android/.local/`.

### AC-03 — upgrade rehearsal entry point is behind the current schema

[rehearse-upgrade.py](../../deploy/rehearse-upgrade.py) rejects
`--old-schema 4 --new-schema 5`. The supplied deployment rehearsal therefore
cannot certify the required upgrade yet.

A separate local test used the exact schema-4 binary hash installed on the VPS
and the current schema-5 debug server. It passed exact old-row/credential/partial
upload preservation, resumed upload, old-binary rejection of schema 5, and
independent restoration/resume using the matching old binary and backup.
This does **not** validate `upgrade --apply`, real systemd or a fresh VM install.

### AC-04 — production lacks the new exit API

The VPS still runs the September 12 binary, SHA-256
`cd1351dce893b9f32c1f9bd75791d0b1250c949dcfca5383996fd918a9305334`.
It has schema 4, no `folder_exits` table, and the new exit route returns HTTP 404.
Service/HTTPS health and database integrity are good; service restart count is 0.
No production write, fault injection, deployment or service restart was performed.
An authorized, backed-up upgrade is required before production exit acceptance.

### AC-05 — physical phone disconnected

The V2329A was initially reachable but disconnected before phone testing began.
A reconnection request was sent; the device remained absent at the end. No phone
data was cleared, no APK was installed on it, and no physical test was claimed.
The [earlier installed-desktop/phone run](installed-desktop-phone-exit-2026-09-18.json)
remains separate evidence, not a substitute for fresh full acceptance.

#### Physical-device continuation after reconnection

The phone subsequently reconnected and the user approved an overwrite install.
The new debug APK SHA-256 is
`798258d4d89661ae88b6d3b644dd4028345a058eb8279b0401c315ecd41f9b31`.
The installed APK, signing certificate and unchanged package UID were verified.
Stopped before/after backups confirmed schema 7, the original empty Folder and
transfer tables, and all six protected private files/preferences unchanged.
The application launched successfully and opened the Android directory picker.

Fresh native end-to-end acceptance was initially blocked. An isolated loopback
relay, USB reverse route and installed-desktop test registry were prepared.
Desktop accessibility toggle calls returned success without the HTTP/directory
switches changing state; invitation creation was correctly refused with
`HTTPS is required.` No server Folder was created. Window visibility/frame
delivery is a hypothesis, not a diagnosed application defect; the user was
asked to bring the test Add Folder window into view before further testing.
No application code or production service was changed. Private continuation
evidence is in `android/.local/phone-acceptance-resume-20260918-ANRxq9/`.

After the user made the window visible, both switches completed and the fresh
scoped chain passed: existing-directory initialization, six native receiver
acknowledgements including an 8 MiB file, empty-source rejection/recovery,
phone-initiated three-party clean exit and removal, and exact original/private
data preservation. See the [physical continuation report](phone-acceptance-continuation-2026-09-18.md)
for artifacts, evidence and limitations. AC-01 through AC-04 remain open; this
does not turn the failed full sequence into a passing release acceptance.

## Transport boundaries and performance

A disposable real CLI/server chain verified **59 files / 115,891,951 bytes**,
including PNG, JPEG, GIF, WebP, BMP/TIFF byte transport, SVG, PDF, ZIP, WAV, text,
Unicode/space/emoji names, opaque/no-extension data and a misleading image
extension. An EPUB ZIP container was transported, not reader-validated. This
does not add preview/codec support or claim every format is covered; local AVIF
fixture generation was unavailable.

The chain passed one-byte and 100 MiB files, empty/100 MiB+1 rejection, 40 distinct
uploads over four concurrent clients, paused upload across server shutdown and
restart, all receiver hashes/ACKs, database integrity and a no-op second receive.
An initial artificial three-second timeout expired at 100 MiB finalization;
the complete rerun with the CLI's normal timeout passed. Both logs remain.

Representative measurements, not universal performance budgets:

- Debug Xvfb/Cairo album: 10,000 model entries reference **32 small PNGs**;
  model update 20.55 ms, smooth frame-clock p95 16.67 ms, settled idle 0% of one
  core. The separate 120-photo app measured 0.125% over eight seconds with
  unchanged 185,576 KiB RSS.
- Installed release: 50k-record name-sort median 57.37 ms; 1,000-Folder widget
  render median 140.85 ms. These exclude subsequent painting and were sampled
  with other host tests/build work, not on a quiescent benchmark machine.
- Owned hardware-GL compositor: 1000×680 snapshot/GPU/readback p95 1.639 ms over
  60 samples; this is not display FPS.
- Empty debug app on the software-rendered emulator: five cold-process starts
  971–1,091 ms; 0.06 CPU seconds over 10.07 seconds (~0.60% of one core).
  No ANR was recorded since that emulator boot. USB/OEM/battery performance and
  long-duration behavior are not inferred from these samples.

Selected original Android screenshots and native Linux layout snapshots were
visually inspected. A blank browser snapshot taken after the short-lived state
test closed is explicitly excluded from visual evidence; native state assertions
passed independently.

## Remaining conditions and teardown

Fresh physical installation/upgrade, optical QR, natural unplugged/screen-off Auto,
real network transitions, long mixed/high-resolution album use, oldest supported
Linux/Android environments and an actual 16 KiB Android runtime remain open.
Formal signing/debug migration, frozen-SHA clean CI, clean VM/systemd installation,
licensing/notices and dependency vulnerability review are separate release gates.

The initial emulator network failures were caused by inherited proxy settings;
restarting only the owned emulator without proxy environment fixed the preflight.
Host proxy settings were not changed. Synthetic AVD staging was archived before
resetting the disposable app for empty-state performance sampling. Test servers,
owned graphical sessions and the owned emulator were stopped. Original user
Folders/files, the phone and production state were untouched. No commit, push,
tag, release signing or publication occurred.

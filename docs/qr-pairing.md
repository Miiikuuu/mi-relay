# QR invitations

QR pairing is a convenience layer over the existing Folder invitation protocol.
It does not require another relay upgrade, change pairing authority, enable Auto,
or initialize any existing directory by itself.

## Use

1. On Linux, create a Folder with **Create on server**, then select **Show QR code**
   beside the invitation. This opens a compact black-on-white popover.
2. On Android, choose **Add Folder → Scan QR code** and grant camera permission
   if desired. Scan the invitation displayed by Linux.
3. Review the filled server address, enter the Folder's local name and optionally
   choose an existing directory. Press **Save** to claim the invitation.
4. Compare the verification codes on both devices. Linux **Check pairing →
   Codes match — confirm** is still required before transferring.

Camera permission denial, cancellation, invalid scans or unavailable cameras leave
manual entry available. Invalid scans do not replace existing input. A valid scan
replaces only the server/invitation fields, selects pairing mode and turns off the
debug HTTP option; it does not change the source directory or history consent.
The app does not save invitation form fields in Activity saved state; if Android destroys the
creation screen, scan again. This is intentional, not automatic restoration of
secret input.

QR codes require HTTPS. Development HTTP setups continue to use manual input.
An invitation expires after the server's ten-minute validity window. Linux disables
the QR code at expiry, and clears it when the URL changes, an invitation is
replaced, or a handshake reports a claimed/closed Folder. Android checks expiry
both when parsing and before saving an unchanged scanned invitation. The server
remains authoritative about expiry, reuse and revocation.

**Replace invitation** revokes any previous sender. It is not a harmless QR refresh
for an already connected Folder; read its confirmation before proceeding.

## Security and resource boundaries

- QR content contains only `kind`, `version`, `server_url`, `pairing_code` and
  `expires_at_unix`. It contains no administrator or sender/receiver credential.
- The invitation itself grants the right to claim a sender: treat the QR image
  as a secret, not a public device-discovery code. Do not post screenshots.
- No public URI handler, automatic URL opener, third-party scanner app, cloud
  decoding service or automatic claim is registered. Save is explicit, and
  matching-code confirmation remains separate.
- The scanner is a private, non-exported Activity with secure-window protection.
  Capture images and beeps are disabled. Camera capture is user-triggered,
  bounded by a 60-second scan timeout and released when capture pauses.
- The JSON reader rejects duplicate/missing/unknown keys, coercions, unsupported
  versions, non-QR text, invalid IDs, credential-bearing URLs, fragments, queries,
  escaped URLs, non-HTTPS schemes and stale/far-future expiry. Errors never echo
  the decoded input. Payload input is bounded to 1,536 UTF-8 bytes and URLs to 512
  ASCII characters. Expiry more than 15 minutes ahead is rejected to catch invalid
  clocks/payloads; check device clocks if a fresh invitation fails.
- Linux encodes the matrix once per invitation, uses integer-sized black modules
  with a four-module white quiet zone, and draws cached modules without continuous
  animation. Expiry uses a one-shot weak-reference timer, not periodic polling.

Implementation references: [qrcode 0.14.1 encoder API](https://docs.rs/qrcode/0.14.1/qrcode/struct.QrCode.html)
and [ZXing Android Embedded ScanContract and permission handling](https://github.com/journeyapps/zxing-android-embedded/blob/master/README.md).
Only the optional desktop build includes the Rust QR encoder; Android bundles the
decoder and does not require Google Play services.

## Validation

Tests cover malformed/oversized invitations, URL and expiry guards, exact envelope
fields, a local encoder/decoder round trip, and Android decoding a synthetic image
produced by the actual Linux encoder. A GTK test checks expiry, invitation
replacement and invalid-input clearing. Emulator UI tests exercise scan-result
handling without a claim, cancellation, malformed text, permission fallback and
the system permission-denial dialog. Scanner result injection is not evidence of
a physical camera reading a desktop screen.

The automated tests use isolated fixtures. Subsequent
[physical-phone acceptance](phone-qr-pairing-2026-09-12.md) passed a real camera
scan, explicit claim, matching-code confirmation and cleanup using one separate
production-relay diagnostic Folder. Original phone Folders/files were preserved.
Optical reliability across other cameras and lighting still requires further tests.

### Build and regression record — 2026-09-12

- Rust locked desktop library/integration suite: **217 passed, 16 explicitly
  ignored**, run serially. Strict all-target desktop Clippy, rustfmt and whitespace
  checks passed. Both the existing GTK creation test and the new GTK QR
  expiry/replacement test passed separately on a loopback-only Broadway display.
- One earlier parallel run hit a pre-existing receiver recovery file-lock failure.
  The full serial rerun and a separate parallel run of all five receiver recovery
  tests passed. No unrelated receiver logic was changed; these results do not
  establish the root cause of that intermittent lock failure.
- Android JVM: **90 passed**, including the explicitly supplied Linux-generated
  QR fixture. Lint: **0 errors, 17 existing warnings**. The nullable fixture-path
  warning in the test code was corrected and the final build rerun.
- The first full emulator pass completed all three new QR-result UI tests and the
  real camera-permission-denial test. An existing recreation test asserted after
  `onActivityCreated`, before the resumed dialog was visible; its standalone rerun
  passed. The test now waits for RESUMED and visible dialog, and additionally
  checks the retained pending-share count. The standalone wrapper expected a
  file transfer and therefore rejected this UI-only run; that harness failure is
  not counted as a failed app recreation assertion.
- Initial emulator reports are retained in `android/.local/device-qa-ly6zd0f5/`
  and `android/.local/device-qa-2uxetso_/`. Synthetic cross-decoder fixture:
  `target/qr-qa-vWYcwr9w/linux-qr.pgm`.
- Final emulator rerun: **41 UI + 1 system camera-permission-denial tests passed**,
  with no skipped tests. Report: `android/.local/device-qa-6fotui3s/`. The isolated
  Linux receiver also verified 6 legacy files, 1 paired file and rejection of a
  disconnected Folder. These transfers came from existing UI regression cases,
  not a physical-camera QR pairing. The test relay, emulator and temporary
  Broadway display were stopped afterwards.
- Linux release binary: `target/release/mirelay-desktop`, SHA-256
  `10cf3b41f9892a90de78b1885af54d480f5348f7e28c68ad573e33f5a93c5dbe`.
- Android APK: `android/app/build/outputs/apk/debug/app-debug.apk`, SHA-256
  `5ed5850a94dfbf0a52c8f1d7ab2c20256ff10bc289932af400f57ccd9ac99340`.
  Built for the existing arm64/x86_64 JNI payloads; installed only on the isolated
  emulator during the initial implementation task; the follow-up physical
  acceptance above installed the same APK in place. No Git commit or push was performed.

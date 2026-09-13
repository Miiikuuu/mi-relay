# Physical-phone QR pairing acceptance — 2026-09-12

Device: vivo V2329A, Android 16 / API 36, USB serial ending `01YB`.
Installed APK SHA-256:
`5ed5850a94dfbf0a52c8f1d7ab2c20256ff10bc289932af400f57ccd9ac99340`.

## Update and data preservation

The old APK and stopped app-private data were backed up before installation.
Signing certificates matched, and the user approved the system's in-place update
prompt. The installed hash and unchanged package UID were verified. No uninstall,
clear-data operation, emulator test harness or database modification was used on
the real phone. SQLite remained schema 6, with integrity and foreign-key checks
passing.

Immediately after update, all original rows were identical: **4 Folders and 57
transfer records**. All **57 private outgoing files** retained their original
hashes. The final audit after test cleanup preserved these same records/files;
the only permitted runtime difference was a forward-moving `auto_sources.last_scan`
on an already-enabled original source. No pairing, credential, source selection,
Auto option, transfer history or existing directory was changed.

## Actual optical scan and pairing

1. Created one independent **Phone QR acceptance 2026-09-12** Folder on the
   existing relay using its normal authenticated HTTPS API. No real queue was
   downloaded or acknowledged. No server deployment or service restart occurred.
2. Displayed its short-lived invitation on the right-hand `eDP-1` screen in a
   dedicated GTK test window using the **actual `InvitationQr` widget**. This was
   not an image-generator mockup or an injected Android scan result.
3. Opened **Add Folder → Scan QR code** on the phone. The user allowed camera
   access and physically scanned the computer screen, then left Save untouched.
4. Verified the scanned-invitation message, exact HTTPS server address, pairing
   mode enabled and HTTP disabled. The relay still reported `awaiting_peer`,
   proving scanning did not automatically claim the invitation.
5. Entered the synthetic local name **20260912** and used the actual phone Save
   button. No source directory was selected and no history/Auto option was enabled.
   The relay moved to `awaiting_confirmation`.
6. Compared the phone's visible verification code against the independent receiver
   handshake. A pending receiver request was rejected with HTTP 409. Only after
   comparison did the Linux-side verifier confirm through the normal pairing API.
7. Used the phone's **Check pairing** action. It reported **Paired · ready to send**.
   A stopped backup followed by reopening verified the ready state and retained
   encrypted credential. Original records/files were preserved; no new transfer
   or automatic source had been created.

Android's camera service recorded MiRelay opening camera 0 at **16:34:30** and
releasing it at **16:34:34**. The later service snapshot had an empty active-client
list. This establishes release after this scan, not a general camera-performance
or battery-use benchmark.

The first confirmation attempt deliberately stopped at its UI guard because the
Folder drawer still covered the verification code. The test then selected only
the new Folder and compared the visible code before sending confirmation. No
unchecked confirmation was sent. The first strict preservation comparison flagged
the normal Auto scan timestamp; column-level inspection identified only
`last_scan`, and the existing forward-only runtime-change validator was applied.
No app fix or database rollback was needed for these test-driver observations.

## Cleanup and final checks

- Used the real phone's separate **Disconnect Folder** and **Remove Folder**
  confirmations for the exact synthetic scoped URL, never an original Folder.
- The original four entries, 57 transfer records and 57 private files remained.
- The diagnostic relay Folder is permanently disconnected. Its old receiver
  credential gets HTTP 410 `folder_disconnected` from delivery and directory
  endpoints; repeated disconnect returns the same terminal state.
- The diagnostic server metadata and private local evidence are retained. No
  synthetic source file was uploaded during this QR test, and no server GC ran.
- Closed the owned QR test window. No relay daemon, emulator, USB reverse mapping,
  IME configuration, screen timeout or global desktop setting was created/changed
  during this physical QR acceptance. Camera permission was granted by the user
  for the new scanner feature; it was not silently revoked afterward.

Private ignored evidence:
`target/phone-qr-20260912-d4joCInq/`, including APK/data backups,
`upgrade-verified.json`, `scan-verified.json`, `confirmed.json`,
`ready-verified.json`, `ready-runtime-changes.json`, and
`phone-final-verified.json`. The invitation file contains diagnostic credentials;
do not publish the directory or reuse it for a real Folder.

This completes one **real optical scan → phone claim → matching verification →
receiver confirmation → phone ready → disconnect/remove** path. The invitation
creation and receiver confirmation were performed by Linux-side test helpers, not
by clicking the full desktop settings workflow. This is not a new phone-to-Linux
file-transfer, long-running, permission-denial, arbitrary-camera/lighting or full
performance test; automated failure-path coverage is recorded in
[QR pairing](qr-pairing.md). No Git commit or push was performed.

# Physical-phone Folder disconnection acceptance

Device: vivo V2329A, Android 16 / API 36, USB serial ending `01YB`.
Date: 2026-09-12. APK SHA-256:
`109e3619bf70ff8471dae768782416c3ee3c357370cf44fc77d05d30094702ae`.

## Update and preservation

- USB remained visible when ADB briefly lost the device. Restarting the host ADB
  service restored the authorized connection; no phone reset was used.
- Backed up the existing APK and private app data, verified matching signing
  certificates, and checked that no queued/uploading transfers were present.
- The user approved the phone's system installation prompt. The new APK was
  installed in place, with package UID and signature preserved. No uninstall,
  clear-data operation, or emulator reset harness was run on this phone.
- SQLite migrated from schema 5 to 6 and passed integrity/foreign-key checks.
  All existing database records remained identical: **4 Folders, 57 transfers**,
  including pairing, credentials, source settings, tracking and history.
- All **57 original private outgoing files** matched their pre-update hashes,
  both after migration and after the complete test.

## Isolated real-UI checks

A new `qa disconnect test` Folder used a loopback-only relay on the development
computer and a dedicated USB reverse mapping on port 18096. The production VPS
and all four original Folders were left unchanged.

1. Created the temporary Folder through the actual phone UI, matched its displayed
   verification code with the isolated receiver, and confirmed pairing.
2. Verified Remove was disabled before server disconnection. Opening Disconnect
   and then cancelling left the durable pairing state `ready`.
3. Removed only the test reverse mapping to simulate offline operation. Confirming
   Disconnect resulted in `disconnect_pending`, with the credential retained.
4. Force-stopped/reopened the app and verified the pending state survived, original
   records stayed identical, and premature removal remained disabled.
5. Restored the test mapping and used Retry disconnect. The server and phone both
   reported `disconnected`; the phone's stored credential was cleared.
6. Reopened the app, checked the terminal state, and removed only the temporary
   Folder through its separate confirmation dialog. Original records and private
   file hashes were unchanged afterward.
7. Verified the old receiver credential received HTTP 410 `folder_disconnected`
   for delivery listing and directory state. Repeating disconnect returned the
   same terminal receipt.

The phone's Chinese IME initially rewrote synthetic ASCII input. It was temporarily
switched to English, the test fields were corrected, and Chinese mode was restored
visually before cleanup. An accessibility test guard was corrected to honor a
disabled button ancestor rather than its enabled text child. The final HTTP
assertion was corrected to read the protocol's top-level `code` field; the first
attempt already returned the expected 410. These were test-driver issues; no app
code changes were needed during phone acceptance.

## Completion and limits

The temporary Folder entry and its local credential were removed. Its isolated
server database/receipts and private backups were retained. The temporary relay
was stopped and its exact USB mapping removed. MiRelay was left open; screen
timeout, font, rotation, source permissions and original Auto settings were not
changed. Current buffered app-UID logs had no matching fatal exception, fatal
signal, ANR or out-of-memory marker.

Evidence is private and ignored under
`target/phone-disconnect-20260912-I9SktT/`, including `upgrade-verified.json`,
`cancelled-verified.json`, `pending-verified.json`, `pending-after-restart.png`,
`disconnected-verified.json`, `remove-dialog.png`, `phone-final-verified.json`,
and `log-check.json`. `confirm-disconnect.png` is an earlier settings capture,
not the confirmation dialog; the actual dialog is `disconnect-dialog.png`.

This was scoped real-device disconnection acceptance, not a full transfer,
mid-upload kill, long screen-off, or performance rerun. No new phone source files
were sent in this test. Android Keystore keys are not exportable, so these backups
do not promise credential recovery after uninstalling. The production relay has
**not** been upgraded; normal production Folders still need that server rollout
before server-confirmed disconnection can be used there. No Git commit or push
was performed.

# Disconnecting and removing Folders

This feature disconnects a MiRelay Folder, not a USB cable or a source directory
permission. Upgrade the relay and both apps before using it. The relay migrates
its SQLite database from schema 3 to 4; take a stopped-server backup first.
Older server binaries cannot read schema 4. Do not downgrade the database version
manually: that would bypass the revocation state.

Android also upgrades its local database to schema 6 without replacing existing
rows. This semantic version barrier prevents older apps from reopening a stopped
Folder and resetting its state during a handshake. Downgrading the app/database
is unsupported; keep backups and do not edit database version numbers.

## User workflow

1. Open **Folder Settings** on Linux or **Folder settings** on Android.
2. Choose **Disconnect** / **Disconnect Folder** and confirm. Linux needs the
   saved Folder's session token, or its configured token environment variable.
3. Once the server confirms, choose **Remove** / **Remove Folder** if you also
   want to remove the entry from this app.

Disconnect permanently closes that relay Folder for both devices. A new Folder
and a new pairing are required to reconnect. The other device's local entry is
not remotely deleted; its next transfer is rejected by the relay. Checking
pairing on Android also records remote disconnection and stops local work.

If the network fails, the credential is invalid, the server is outdated, or the
response is lost, the app keeps **Disconnect pending**. Local sends/receives and
Auto cannot be resumed; removal of paired entries remains blocked. Restore the
connection or upgrade the relay, then **Retry disconnect**. Linux session tokens
are not saved to disk, so re-enter one after restarting the app. Repeating a
successful disconnect is safe and returns the same terminal state.

Legacy shared-token connections cannot be revoked per Folder. Their explicitly
local Remove action stops management only on that app; it does not revoke the
shared token or stop another device. Ask the administrator to rotate the shared
credential if server revocation is needed (this also affects other legacy users).

## Files and safety boundaries

- Original source files and received files are not deleted, moved, or renamed.
- Android removal deletes the Folder's local credential, source tracking and
  transfer-history records. Existing private staged files/previews are retained;
  there is no automatic cleanup in this operation. SAF permissions are retained
  because another Folder may share them.
- Linux removal deletes only the registry entry and in-memory session credential.
  The stopped configuration, state/history, and received files remain on disk.
- Relay queues, object bytes and history are not deleted by disconnect. Normal
  independent server retention/GC rules still apply.
- Already-authorized requests may finish. This is not a rollback or a guarantee
  that no final file can arrive around the moment you click Disconnect.
- App-managed workers are stopped/invalidated locally before the network request.
  Independently launched CLI processes are not killed. New Config-based receives
  reject a stopped config; the standalone directory CLI is blocked by the relay
  after server confirmation, not by desktop app settings.
- The server keeps only the existing credential hashes for authenticated
  handshake/disconnect retries. These credentials cannot transfer, claim,
  confirm pairing, or issue replacement invitations on the closed Folder.

## Implementation and checks

`POST /f/{folder_id}/api/v1/pairing/disconnect` authenticates the current sender or
receiver and commits terminal revocation in one SQLite transaction. Other Folder
credentials, legacy credentials and the administrator credential do not grant
access to this endpoint. Transfers and directory operations return HTTP 410
`folder_disconnected` after revocation. The client checks the returned Folder
identity, protocol and terminal state before declaring success.

Regression tests cover either-device revocation, authentication/isolation,
idempotent retries across server reinitialization, pre-claim disconnection,
schema upgrades, renewal races, malformed receipts, Linux durable failure/retry,
Android confirmation/cancel, stale worker writes, retry/Auto barriers and local
metadata removal with file retention. Real-device and production-server behavior
must be verified separately; automated disconnect tests use isolated fixtures.

### Validation — 2026-09-12

- Rust `cargo test --locked --features desktop --lib --tests`: 215 passed,
  14 environment-dependent tests ignored. Strict all-target desktop Clippy,
  rustfmt and diff whitespace checks passed. Both Android JNI ABIs rebuilt.
- Android JVM: 83 tests passed, including v1–v5 migration coverage and reopening
  a stopped database. Lint: no errors, 17 warnings (dependency/resource/style
  suggestions; not a claim of zero warnings).
- Isolated API 36 emulator: 38 UI + 12 Auto + 8 directory tests passed in
  `android/.local/device-qa-x5in1a6n/`. The Linux harness verified 9 legacy files,
  1 paired delivery, 52 directory paths, and rejection of 1 disconnected Folder.
- After serializing Folder cancellation with Auto scheduling, the final APK
  (`109e3619bf70ff8471dae768782416c3ee3c357370cf44fc77d05d30094702ae`)
  passed a targeted 16-test rerun: the three new disconnect/removal UI tests,
  existing-directory pairing, and all 12 Auto tests. Report:
  `android/.local/device-qa-lzukt7_t/`. Linux verified 4 legacy files, 1 paired
  delivery, and rejection of the disconnected Folder. The full 58-test run above
  preceded this final scheduling-lock adjustment.
- The first run's app tests passed, but its old harness incorrectly expected
  disconnected pairs to receive successfully. The harness now checks the actual
  terminal database state, HTTP rejection and absence of delivered records.
  The failed report is retained in `android/.local/device-qa-cvu778bo/`.

This is scoped regression testing, not a new full-suite certification. Subsequent
[physical-phone acceptance](phone-disconnection-2026-09-12.md) passed signed
in-place migration and isolated real-UI offline/restart/retry/removal checks.
The [production relay rollout](server-disconnection-2026-09-12.md) also passed
data retention and isolated public-HTTPS transfer/disconnection acceptance.
Real GTK interaction and the separate long-running/process-kill recovery suites
were not rerun for this change.

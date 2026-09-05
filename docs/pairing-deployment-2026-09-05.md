# Pairing deployment follow-up — 2026-09-05

The user explicitly authorized upgrade verification, then updating the existing
VPS and physical Android phone while preserving their data. This report supersedes
the local-only deployment status in the earlier pairing QA report.

## Server: upgraded and accepted

- Existing Ubuntu 22.04 x86_64 service upgraded at 12:24 UTC. systemd reports no
  automatic restarts; stop/start occurred within the same logged second.
- Static musl release SHA-256:
  `5ae342568ee874f1cf9714d6ca9c1540d67a092bd3706cf3abdb0deda8fc209f`.
- Private, durable recovery backup:
  `/var/backups/mirelay/upgrade-_9i_vv05` on the VPS. It contains the old binary,
  environment and entire pre-upgrade data directory. Do not publish this backup.
- SQLite schema 1 → 2; integrity and foreign-key checks passed. All seven old
  delivery rows were compared by hashes and remained identical (pending 0,
  acknowledged 7). Existing legacy token preserved; separate random admin added.
- Caddy, SSH, Hysteria and Shadowsocks process identities/start times unchanged.
  Caddy binary/config and both existing MiRelay systemd units were unchanged.
  No firewall, SSH or phone proxy settings changed. Backend remains loopback-only.
- Public HTTPS certificate validation, health and unauthenticated 401 passed.
  Existing credential successfully accessed the legacy index without consuming it.

## Rehearsal and post-deployment transfer

Before touching production, the actual upgrade filesystem logic and both release
binaries were exercised in isolated bubblewrap namespaces. Systemctl alone was
replaced by a child-process controller. Four complete runs passed, covering:

1. Wrong checksum leaves the running service untouched.
2. Copy failure and fsync failure each restart the unchanged old backend.
3. Durable backup, schema migration and exact old-row/token retention.
4. A pre-upgrade incomplete tus upload retains its 65,536-byte offset and resumes.
5. Pairing confirmation gates transfer, and sender/receiver roles stay isolated.
6. Post-start health failure does not roll back new data or rotate credentials.
7. Old binary plus backup recover schema-1 state and upload offset in a different
   disposable directory, without overwriting upgraded state.

The initial rehearsal observer incorrectly retained Python SQLite connections
across service restarts and reported a malformed-image error. The observer now
closes connections explicitly; the four subsequent complete runs passed. Initial
content probes were also corrected to supply the required strong If-Match ETag.
These were test-harness corrections, not production data repairs.

Seven deployment helper guard tests and all thirteen Rust pairing integration
tests also passed. The backup helper now fsyncs the backup directory's parent;
new tests cover this and reject special files without blocking or following links.

The real upgraded VPS was then tested over public trusted HTTPS using a newly
created, isolated **Upgrade QA 2026-09-05** Folder, not the user's legacy queue:

- Pending transfer rejection, matching two-party verification, confirmation and
  wrong-role rejection all passed.
- A synthetic 262,144-byte non-image paused after 65,536 bytes and resumed.
- The Rust Linux receiver verified exact bytes/SHA-256, ACKed the file and received
  nothing on a second sync. No production restart was needed for this smoke test.
- Test sender was revoked afterwards. One empty diagnostic Folder and its one
  acknowledged synthetic delivery remain as metadata; no user file was consumed.
- Private local evidence: `target/deployment/paired-https-ndo0w5ug/summary.json`.
  The report directory also contains a private diagnostic receiver credential;
  keep it outside Git and do not share the entire directory.

An initial public setup request lost its connection before receiving a response.
The request was not blindly retried: a read-only server check first confirmed that
no Folder had been created. The subsequent complete run passed. The network cause
of that disconnect was not established.

## Physical phone: in-place installation and migration passed

- Exact authorized phone selected; no emulator reset/test runner was used on it.
- Existing APK backed up and signing certificate matched to the new APK.
- App-private data backed up after stopping MiRelay. Schema 2, one Folder, one
  transfer and zero automatic sources were observed. No original photo was opened.
- Initial private backup: `android/.local/phone-upgrade-o3ffp_3u/`.
- Android rejected `adb install -r` with
  `INSTALL_FAILED_ABORTED: User rejected permissions`. The second attempt returned
  the same explicit failure. No uninstall, data clearing or security-setting bypass
  was attempted. Work paused for the user to unlock the phone and permit installation.
- After the user authorized retry, a fresh backup was taken and in-place installation
  succeeded. The installed APK's hash and signing certificate were verified.
- Schema 2 → 3 and SQLite integrity checks passed. The original Folder identity,
  connection settings and encrypted credential were exactly preserved; the original
  transfer record was retained. Zero automatic sources remained zero; no directory
  permission, source selection or Auto setting was changed.
- MiRelay launched successfully, was restarted after the migration inspection and
  was verified as the foreground activity. No new user file was uploaded in this
  installation check; real-phone transfer/source scheduling remains the next test.
- Fresh private pre/post-install backups and migration evidence:
  `android/.local/phone-upgrade-l969as2o/summary.json`.
- A bounded 60-second MiRelay-UID log/memory observation completed and stopped:
  `android/.local/phone-monitor-u_ld5hzh/summary.json`. It found zero Java/native/JNI
  fatal markers; the three post-launch process samples retained the same PID.
  This is a short startup observation, not a long-running background or load test.
- Installed APK:
  `android/app/build/outputs/apk/debug/app-debug.apk`, SHA-256
  `954be0e3c2fb3c2454d5dcbc3ed4a33390cbd39f5c38443f48d3084fcba1fd20`.

Encrypted app data is **not** an exportable Android Keystore backup. Never uninstall
as an update workaround: reinstalling/removing the app can lose its non-exportable
key and persisted directory grants. Archive alone cannot restore credentials onto
another phone. Matching encrypted database values proves preservation, not a separate
test of credential decryption or authenticated phone traffic after this upgrade.

## Remaining limits

- Fresh root installation remains untested end-to-end; this accepted the upgrade
  of one known existing VPS, not every Linux deployment layout.
- Linux receiver credentials remain session-only unless supplied through their
  configured environment variable. No keyring persistence or new desktop install
  was included in this server/phone update.
- Delivery is still Android → server → Linux, not bidirectional synchronization.
- Actual phone directory/provider/background scheduling tests require the user's
  selection of a source directory and explicit Auto activation. Installation is done.
- Certificate renewal, long-term scheduling, quota/retention and new Folder
  lifecycle administration are not proven by this short acceptance run.

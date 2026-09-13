# Folder disconnection — production server upgrade

The user authorized upgrading the existing `relay-vps` MiRelay backend. The upgrade
completed on **2026-09-12 at 07:22:02 UTC / 15:22:02 Asia/Shanghai**. No phone or
Linux application was installed during this server-only rollout.

## Artifact and recovery backup

- Static x86_64 musl release built with locked dependencies from the current
  working tree, not a new Git commit. SHA-256:
  `cd1351dce893b9f32c1f9bd75791d0b1250c949dcfca5383996fd918a9305334`.
- Previous installed SHA-256:
  `548c778c5017fd98fa4c341659d94aab0793576e8a1d8e106b97e9c15bd4c4c9`.
- Private VPS recovery backup: **`/var/backups/mirelay/upgrade-60axob1y`**.
  It contains the stopped backend's full data directory, old binary and original
  environment file. Its contents were checked against the stopped-state snapshot;
  the backup was fsynced before binary replacement. Do not publish it.
- SQLite schema **3 → 4**. Never run the old binary against schema 4 or manually
  lower `user_version`. Recovery needs the matching old binary and pre-upgrade
  data together. Restoring that snapshot after new traffic would lose later writes;
  no automatic rollback was performed or scheduled.

Only `mirelay-server.service` was stopped and started. Systemd recorded both in
the same second; the upgrade helper completed in about 1.92 seconds. These are
server-side timings, not an end-to-end zero-downtime measurement.

## Existing installation retained

The stopped-state and immediate post-upgrade snapshots matched every old table
column and row, object file, tus state file and environment-file fingerprint.
The new `folders.disconnected` field was zero for every old Folder.

- All **4 existing Folders** and **62 delivery records** remained: 2 pending,
  60 acknowledged. No real Folder was disconnected, re-paired or initialized.
- All 67 existing content/upload files were retained byte-for-byte.
- SQLite integrity and foreign-key checks passed, including on the backup.
- Caddy binary/config, both MiRelay systemd units and the original SSH, Caddy,
  Hysteria, Shadowsocks and existing Gunicorn process identities were unchanged.
  No firewall, credentials, certificates or proxy settings were edited.
- Backend still binds only `127.0.0.1:8080`. Public HTTPS health passed with normal
  certificate validation. Service PID 57710 remained active, `NRestarts=0`.

## Tests performed

Before rollout, the actual old/new release binaries and upgrade filesystem logic
passed a disposable namespace **3 → 4** rehearsal. A separate **2 → 4** rehearsal
also passed. Together with 13 deployment-helper unit tests, these checked:

- Bad artifact checksum, copy failure and backup-fsync failure handling.
- Durable backup, exact old-data/credential retention and isolated old-version
  recovery using the matching backup.
- Legacy and scoped tus uploads resuming at their retained 65,536-byte offsets.
- Pairing gates, directory inventory, Unicode paths, byte/hash checks, receipt
  validation and isolation from other queues sharing the same content.
- Disconnection idempotence, both-role rejection, persistence across restart and
  unaffected neighboring Folders.
- Post-start health failure without rolling back accepted data or credentials.

The first namespace launch was denied by the local sandbox; the authorized
isolated rerun passed. No production service was involved in either rehearsal.

After rollout, a new **Disconnection upgrade QA 2026-09-12** Folder was created
over verified public HTTPS. Its synthetic **256 KiB non-image file** traversed
Linux → VPS → Linux, paused after 64 KiB, resumed and arrived with identical bytes
and SHA-256. The real Rust receiver persisted it and acknowledged it; a second
receive found nothing pending.

Only this synthetic Folder was then disconnected. Both credentials received
HTTP 410 `folder_disconnected` for new upload, delivery and directory operations.
Repeated disconnect calls succeeded, unrelated credentials were rejected, and
confirmation/renewal/reusing the invitation could not reopen the Folder. The
already received Linux file remained intact.

Final audit again verified all original rows/files and protected services. The
server now has **5 Folders, 2 pending and 61 acknowledged deliveries**: the only
additions are the disconnected diagnostic Folder, its acknowledged synthetic
delivery and one retained upload-state file. They are deliberately retained;
no production cleanup/GC was run. No authenticated root-queue request was made
by the HTTPS test.

## Evidence and limits

Private local evidence and fixed artifact:
`target/deployment/disconnect-rollout-wAUDEYl5/`.
Private server audit scripts/fingerprints:
`/root/mirelay-disconnect-upgrade-JaA37CEz/`.
The HTTPS report includes diagnostic credentials: keep the entire directory out
of Git and shared logs. No Git commit or push was performed.

This is one existing-server upgrade and scoped acceptance, not a full new
performance, long-running, certificate-renewal, phone-to-production or GTK UI
test. Phone UI acceptance is documented separately in
[physical-phone validation](phone-disconnection-2026-09-12.md).

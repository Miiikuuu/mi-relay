# Coordinated clean exit — existing production server upgrade

The user authorized upgrading the existing MiRelay backend, validating the
production path, then committing and pushing the development work. The server
upgrade and scoped public-HTTPS acceptance completed on **September 18, 2026**.
This is not a release-candidate approval or a fresh-host installation test.

## Artifacts and recovery

- Old schema-4 binary: `cd1351dce893b9f32c1f9bd75791d0b1250c949dcfca5383996fd918a9305334`.
- Initial schema-5 binary: `3492d981142d6e2112ca7d6e471f305ee5d18d8a294c9c1a26cc0fdf6f51d8a9`.
- Final schema-5 binary: `c898d0a550741d0c70fc74d4e35991967ac6b3e7f3a50341c5fb6be799edfb78`.
- Both new binaries are locked-dependency Rust 1.98.1 optimized x86_64 static-musl
  builds from the working tree. They are not signed distribution releases.
- Initial upgrade verified at 06:22:23 UTC; final correction verified at
  06:31:08 UTC (14:31:08 Asia/Shanghai).
- Private original schema-4 recovery backup: `/var/backups/mirelay/upgrade-ntpd7olc`.
- Private pre-correction schema-5 backup: `/var/backups/mirelay/upgrade-puiq8kjx`.

Each operation stopped only `mirelay-server.service`, durably backed up the full
data directory, binary and environment, then replaced the binary and restarted
the service. Backup contents were compared against the stopped-state snapshot.
Each helper took about 1.65 seconds; these are server-side helper timings, not an
end-to-end availability guarantee. No automatic database rollback was attempted.
Recover with a matching binary/data backup, never by downgrading `user_version`;
restoring a backup after new traffic would discard later writes.

## Preservation and final state

Every pre-existing table column/row, payload/upload file and credential fingerprint
matched immediately after both upgrades. The final audit also compared all
original rows/files after the synthetic test (allowing the SQLite sequence to
advance). SQLite integrity and foreign-key checks passed.

- All **6 original Folders**, **2 pending** and **61 acknowledged** delivery
  records and **68 original content/upload files** were preserved.
- No real Folder was disconnected, retired, initialized or re-paired. No
  authenticated root-queue read, receive, ACK or garbage collection was used.
- The final server has 7 Folder rows: the original six plus one small retired
  diagnostic Folder tombstone with completed protocol receipts. Its transfer
  records, content and incomplete uploads were removed by the actual exit API.
- SSH, Caddy, Hysteria, Shadowsocks and the existing Gunicorn listener process
  identities were unchanged. Caddy binary/config, both MiRelay units and the
  credential file were unchanged. No firewall, proxy or certificate edit occurred.
- Backend remains loopback-only at `127.0.0.1:8080`; trusted public HTTPS health
  succeeds. Final backend PID was 82687, active with `NRestarts=0` at audit time.

## Production checks and discovered fix

One newly created, independently paired synthetic Folder was tested through
public HTTPS with normal certificate verification and redirects disabled.

1. Missing authentication, pairing gates, sender/receiver isolation and
   non-mutating clean-exit status behaved as expected.
2. A unique 256 KiB non-image file paused at a persisted 64 KiB tus offset,
   resumed, and was received by the actual Rust Linux CLI with identical bytes
   and SHA-256. Its real receiver ACK and an empty repeat receive were verified.
3. A second complete pending object and a third upload paused at 64 KiB were
   created only in that test Folder for cleanup verification.
4. A negative test exposed HTTP 500 for an otherwise valid exit ACK sent before
   server cleanup. This was a state-conflict mapping bug, not a service crash or
   data loss. The failing local regression was retained. The fix returns HTTP
   409 `exit_cleanup_pending` without advancing a receipt, while genuine storage
   errors remain server errors and credentials are still checked inside the
   write transaction. Tests cover both roles, repeated early ACKs, no exit yet,
   cleanup still pending, unauthorized access and payload preservation.
5. The corrected binary was rebuilt, rehearsed and deployed with a second full
   backup. The same production Folder then returned the expected 409 and passed
   clean exit, role-specific ACKs, idempotent retries, transfer revocation and
   rejection of re-pairing. Server-side read-only audits confirmed zero scoped
   deliveries/versions/indexes, removal of all three tus records/parts, and
   removal of their unshared content. Local source and received files survived.

The exit ACKs in this server smoke test are **synthetic protocol receipts**, not
claims that Android/GTK local cleanup ran against production. Actual native-app
cleanup was verified separately in the preceding
[physical acceptance](validation/phone-repair-acceptance-2026-09-18.md).

The initial smoke harness incorrectly expected completed upload resume files to
remain. MiRelay correctly removes them on completion. The continuation instead
captured ownership-verified server metadata; it reused the same test Folder and
did not repeat setup mutations. The first error and the early-ACK product failure
are not hidden as passing runs.

## Verification and evidence

- Actual original/final release binaries passed isolated schema 4 → 5 upgrade,
  backup failure, interrupted tus preservation, clean exit and old-backup recovery
  rehearsal. The real host subsequently used its actual systemd service.
- Final Rust 1.98.1 desktop/all-target tests: **245 passed, 0 failed, 23 ignored**.
- Rust 1.88.0 minimum-toolchain repeat: **245 passed, 0 failed, 23 ignored**.
- Focused pairing/exit suite: **21 passed** (overlaps the full suite).
- Strict all-target/all-feature Clippy and root/native formatting passed.
- Deployment helper tests: **14 passed**; repository Python helper tests:
  **39 passed**. These are different scopes, not a combined unique case count.

Private local evidence: `target/deployment/clean-exit-rollout-un23sf/`.
Private VPS audits: `/root/mirelay-clean-exit-upgrade-cFnw03hj/`.
Credentials, backups and test sources remain private and are not committed.

Evidence SHA-256 values:

| Evidence | SHA-256 |
| --- | --- |
| Initial upgrade audit | `2bb2aca7197b05d99afe407d6093353c3f05f8cf2d91af5f5d9cd0eac9b6eff9` |
| Corrected upgrade audit | `24dbc243246d0f9c8c3f84f66a70af1c5db7de0eee162be39b759d11f968cbbe` |
| Final original-data audit | `739dd6193d99642338db8f67bf9a94ec2a215551e79f7083575d3d470564e398` |
| HTTPS continuation summary | `a4449a79244e09e58e2cbbeba2ad99f6fc826603099b99420a48fdcce0e6343b` |

CI-run reports are under `target/ci-reports/clean-exit-*-0918/` with source indexes,
commands, log hashes and exit codes. No phone APK or installed desktop executable
was replaced in this server-only continuation. Long screen-off, unplugged/mobile
network handover, optical QR, fresh installer, formal Android release signing and
other release gates remain separate and unqualified by this run.

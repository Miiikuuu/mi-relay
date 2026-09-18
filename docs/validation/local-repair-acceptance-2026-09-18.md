# Local repair acceptance — September 18, 2026

Status: **authorized local repairs and scoped reacceptance passed; not release acceptance**.

Follow-up: the same repaired APK subsequently passed a
[scoped physical-phone chain](phone-repair-acceptance-2026-09-18.md).
The results below retain the original local run's scope.

The user authorized local formatting/Clippy fixes, Android test isolation and
upgrade-rehearsal repair, followed by another acceptance run. Production upgrade,
commit and push were explicitly excluded. The checkout remains dirty, based on
`fb539cf65cea6e978baf0ad0d0157660954fefe7`; this is not a frozen release candidate.

[Machine-readable evidence](local-repair-acceptance-2026-09-18.json) records the
25 command reports, retained failures, artifact identities and final counts.

## Repairs

- AC-01: formatted both Rust packages, collapsed the power-observer conditional
  without changing its logic, and updated a GPU test's fixed-size chunk iterator
  to satisfy strict Clippy. Rust 1.88 compatibility was checked, not assumed.
- AC-02: all three handcrafted image fixtures now use a shared helper that writes
  the Folder ownership marker before payload bytes and queue records. Test reset
  cancels work and drains file operations before dropping records. Two regression
  cases verify ownership survives queue reset and truly unowned payloads still
  block cleanup without losing their bytes. Production ownership protection was
  not weakened; no blanket staging deletion was added to hide contamination.
- AC-03: rehearsal accepts/defaults to schema 4 → 5, requires an initially empty
  exit-receipt table, and preserves open, unclaimed and already-disconnected
  Folders. The actual isolated `upgrade --apply` path now exercises clean-exit
  authorization, scoped payload/tus cleanup, shared-content retention and
  idempotent receipts after restart, alongside backup/failure/recovery checks.

## Verification

Counts are separate, overlapping scopes; do not sum them as unique cases.

| Scope | Result |
| --- | --- |
| Root/native rustfmt and diff whitespace | Passed |
| Strict all-target/all-feature Clippy | Passed |
| Rust 1.98.1 desktop all targets | 244 passed, 0 failed, 23 ignored |
| Rust 1.88.0 desktop all targets | 244 passed, 0 failed, 23 ignored |
| Explicit private D-Bus power regression | 1 passed |
| Android native/debug/instrumentation build | Passed |
| Android JVM, explicit Linux QR fixture | 110 passed, 0 failed, 0 skipped |
| Android lint | 0 errors, 18 warnings |
| Ordered photo → exit and safety regressions | 6 passed |
| Broad emulator sequence including recovery | 91 passed, 0 failed |
| Deployment script unit tests | 14 passed |
| Isolated real-binary schema 4 → 5 upgrade/recovery | Passed |
| Debug APK signature / ZIP alignment | Passed; debug certificate, not release signing |

All four packaged native libraries were also checked for 16 KiB-compatible ELF
LOAD alignment. The API 36 x86_64 emulator uses 4 KiB pages; this is not a 16 KiB
physical-device certification. Lint warnings consist of dependency-update
advisories, KTX suggestions and one obsolete resource qualifier; dependencies
were not upgraded as part of this repair.

The pre-repair ordered reproduction failed again with the original unowned
staging error (`device-qa-y51sc6kn`). The repaired ordered run passed all six
cases (`device-qa-r7xymk79`). Raw private runs remain under `android/.local/`.
Reproduction failures were retained, not replaced with passing logs.

The broad run (`device-qa-3e768cwk`) passed directory 8, runtime 16, Auto 12,
UI 46, notification denial 1, camera denial 1, appearance 1 and recovery 6.
Both originally failing clean-exit cases passed in the full UI sequence, not
only in isolation. Manual and automatic upload SIGKILL/resume and cold-process
image preview passed. Actual Linux receivers verified 14 legacy deliveries,
one paired delivery and 52 synchronized paths, including exact bytes/hashes and
acknowledgements. These counts are not additional unique test cases.

Tested debug APK SHA-256:
`d2a571a61bedf3b89fea23ca203c6622698363cb9529b4cbf357819b0a6bbc7b`.
All originally indexed inputs remained unchanged during the broad run; only
validation documentation was finalized afterward. The owned emulator was
stopped and all three local test ports were verified released. Evidence was kept.

Harness notes: the first Rust rerun selected `--lib --tests` but required the
244-test **all-targets** count; its 240 tests passed, and the report correctly
failed the scope guard. The corrected all-targets run passed 244. Fixing the
first Clippy error exposed the GPU-test iterator warning; that intermediate
failure is retained separately from the final passing run.

## Boundaries still open

Production was checked read-only: service active, zero restarts, trusted HTTPS
healthy, SQLite integrity good, schema 4, no exit-receipt table. Its binary hash
remains `cd1351dce893b9f32c1f9bd75791d0b1250c949dcfca5383996fd918a9305334`.
No service, credentials, firewall or production data were changed.

The upgrade rehearsal uses disposable namespaces and real files/SQLite/HTTP,
with a child-process controller replacing systemctl. It does not certify real
systemd, a fresh installer or an actual production rollout.

This run did not touch the physical phone or installed desktop. The new APK is
emulator-tested only; the [earlier physical chain](phone-acceptance-continuation-2026-09-18.md)
has its own artifact identity. Optical QR, natural long screen-off/unplugged
scheduling, real network switching and signed/frozen release gates remain
separate. The 23 normally ignored Rust tests were not all rerun in this scope;
only the relevant private-bus test was explicitly enabled. No new performance
qualification is claimed.

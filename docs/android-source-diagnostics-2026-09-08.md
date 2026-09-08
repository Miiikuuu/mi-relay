# Android source diagnostics and database deadlock fixes — 2026-09-08

## Scope

Fixes the actionable-diagnostics and attention-count finding from the
[physical-phone acceptance test](phone-acceptance-2026-09-08.md#finding-an-empty-file-is-difficult-to-diagnose).
Also fixes a database lock inversion reproduced during the batch-upload regression.
This does **not** add support for zero-byte transfers or change background scheduling.

Directory sync still fails closed when any file prevents a complete supported
inventory. It does not silently omit files or commit a partial initialization.
Previously, the worker replaced the validation error with a generic scan message,
and the editor could simultaneously show `0 need attention`.

## Changes

- Empty, oversized, virtual/unknown-size and unknown-modification-time files report
  the first offending relative path, a specific reason and a suggested action.
  Additional unsupported files are counted. The 4 GiB scan limit also has a
  specific background error.
- Only a dedicated app-owned exception exposes these diagnostics to the worker.
  Provider exception messages, document IDs and URIs are not copied into them.
  Display paths are bounded by Unicode code points; control characters, direction
  overrides, line separators and quotes are sanitized without changing real paths.
- Source errors contribute one explicitly identified issue to `need attention` in
  both editors. The file issue count remains intact. No database migration is needed.
- Existing bounded retries, cancellation and revision ownership are unchanged.
  A successful scan clears the persisted source error without resetting pairing,
  directory history or user data.
- `RelayStore.refresh()` no longer holds the `SQLiteOpenHelper` monitor while
  waiting for a database connection. A separate lock still serializes UI snapshots.
  Previously, a writer holding the connection could block obtaining the helper to
  end its transaction, while refresh held that helper and waited for the writer.

## Deadlock evidence

The initial device run at `android/.local/device-qa-qgchtu19/` timed out in the
45-file batch test. The captured thread stacks in `deadlock-excerpt.txt` show the
scan worker blocked on the `RelayStore` monitor and the refresh caller holding
that monitor while waiting in `SQLiteConnectionPool.waitForConnection`.

A deterministic regression first ran against the still-installed pre-deadlock-fix
APK (`547ab47b757a44882476e7aefbfc095253991f97be99186370345519b1baf731`).
It failed at the expected two-second helper-acquisition timeout, then released
the cached transaction handle and exited cleanly. Evidence:
`android/.local/device-qa-qgchtu19/deadlock-negative-control.txt`.
The test forces the same lock ordering five times on the fixed APK and checks
database integrity afterward; it does not rely on a chance batch-test race.

## Verification

Commands:

```bash
bash android/scripts/build.sh :app:assembleDebug :app:assembleDebugAndroidTest :app:testDebugUnitTest :app:lintDebug
python3 android/scripts/device-tests.py
```

- ARM64/x86_64 JNI, development APK and test APK built successfully.
- JVM regression: **51 tests passed, 0 failures**, including 8 new tests covering
  metadata boundaries, multiple invalid files, the exact 4 GiB limit, safe Unicode
  diagnostics, persistent error counts and successful-scan clearing.
- Android lint: **0 errors, 11 warnings** in unchanged dependency declarations,
  resource qualifiers and an existing KTX suggestion. No dependency upgrades.
- API 36 emulator regression: **57 tests passed, 0 failures**: directory sync 7,
  runtime 14, delivery Auto 12, UI 23 and denied-notification handling 1.
- Two additional process-interruption scenarios passed (four seed/verify
  instrumentation invocations): ordinary delivery and delivery Auto retained their
  tus state and recovered after SIGKILL. These are not new directory-specific
  process-death tests.
- Real Linux receivers verified **50 synchronized paths**, including the recovered
  valid neighbor and the 45-file backlog, plus **11 legacy and 1 paired delivery-only
  files**. Exact bytes, SHA-256 and acknowledgements passed.
- The new worker/UI test proves no partial upload while the empty file exists;
  reopening the editor retains the error and shows one source issue. Removing only
  the empty fixture lets the existing retry upload its valid neighbor without
  another manual check. The error clears and the Linux receipt is verified.
- The deterministic database lock-order test passed all five iterations. The
  previously hung batch test completed. No crash, ANR or connection-pool starvation
  markers were found in the final captured app log; this is not an exhaustive
  concurrency or long-duration performance certification.
- Inspected `visual/directory-empty-preview.png`, `directory-empty-worker.png` and
  `directory-empty-recovered.png`: the relative path/reason is readable, the failure
  counts as attention, and the recovered view returns to zero issues.

Final device report: `android/.local/device-qa-czcjo69m/`, harness exit **0**.
The failed first run is retained separately as evidence, not counted as passing.

Development APK SHA-256:
`6b37120ebfecabd377b4b283797d40d281e4133e44eda8ebed613afd51a9c74f`.

Tests use only the repository-owned API 36 emulator and an isolated local relay
with real Linux receivers. No physical-phone installation/data change or VPS
deployment is part of this fix. The prior physical-phone screen-off Auto result
remains **inconclusive**; this diagnostic fix does not change that result.

Follow-up: the owner subsequently authorized a backed-up phone installation and
focused [physical-phone empty-file regression](phone-empty-file-2026-09-08.md).
Both correction/retry scenarios passed on that installed build. The original
screen-off scheduling limitation above remains unchanged.

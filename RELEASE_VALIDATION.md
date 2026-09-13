# Release validation status

No release candidate is approved. This file tracks evidence without combining
historical results into a claim of one full passing run.

## A1 — minimum Rust toolchain (2026-09-13)

- Base commit: `e50afbd28400b674f374cb29012de3f03bde955e`.
- Validated MSRV: Rust 1.88.0, declared as `rust-version = "1.88"` in both packages.
- Pinned development/candidate compiler: Rust 1.98.1.
- Source export overlays only the two package manifests and toolchain pin;
  this is **not** the frozen candidate required by A3.
- Matrix and reproducible entry point: [toolchain validation](docs/toolchains.md).
- Execution status: **Rust 1.88.0: 4/4 builds passed; Rust 1.98.1: 4/4 builds passed**.
  These are compiler/linker results, not runtime test passes.
- Private raw evidence: `target/toolchain-validation-4lhpc240/`. Keep this separate
  from historical phone and release-artifact acceptance.
- [Machine-readable result](docs/validation/toolchains-2026-09-13.json) records all
  eight exit codes/log hashes, 12 artifact hashes, both lockfile hashes and the
  exact source/metadata identity. Raw `results.json` SHA-256:
  `d66abae69286ba4b879857ce8ef0f6a89f43b395c673be1b94ba3b961b7c770e`.
- Environment: Linux x86_64, glibc 2.43, GTK 4.22.4, libadwaita 1.9.1,
  Android NDK r27c / API 26 linkers. Each compiler started with an empty build
  directory; dependency sources were cached. This was not a clean VM or a test
  of the oldest advertised Linux distribution/GTK version.
- All 222 exported input files and both original lockfiles remained unchanged.
  All four JNI shared libraries have the expected ELF machine and 16 KiB LOAD
  alignment; this is not APK ZIP-alignment or 16 KiB-device acceptance.
- The run retained its original runner snapshot with matching hash. The current
  runner additionally records timeouts/build settings, snapshots itself, preserves
  custom Cargo/rustup cache locations and hashes the upload CLI too. Its **5 fake
  compiler tests pass**, separately from the real build matrix. Root/native fmt
  checks and `git diff --check` also pass.

### Separate serial runtime regressions

The same source export passed the following separate runs (both exit 0):

| Compiler | Passed | Failed | Ignored |
| --- | --- | --- | --- |
| 1.88.0 | 217 | 0 | 17 |
| 1.98.1 | 217 | 0 | 17 |

Do not add the counts together as unique cases. These are serial runtime runs,
not the eight compilation jobs or a replay of historical 1.98.0 results.

```bash
# From the evidence directory's source/ export:
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 \
  CARGO_TARGET_DIR=/absolute/path/to/evidence/build-1.88.0 \
  cargo +1.88.0 test --offline --locked --features desktop --lib --tests --quiet -- --test-threads=1
```

For the second run, replace both `1.88.0` occurrences with `1.98.1`.
Raw outputs: `1.88.0-runtime-serial.log` and `1.98.1-runtime-serial.log` in the
evidence directory; their hashes are in the machine-readable result. The 17 ignored
tests were not enabled indiscriminately. This run does **not** close the parallel
recovery-lock investigation or replace GTK/JNI/emulator/phone acceptance.

## A3 preparation — CI and directory ownership (2026-09-13)

This patch adds [ordinary CI](docs/ci.md) and a
[directory lock lifecycle fix](docs/directory-lock-lifecycle.md). The following
results were collected before committing/pushing the patch, from the local dirty
working tree, not a frozen candidate SHA. Subsequent GitHub checks must be read
against their actual tested commit; they are not represented by these local totals.

- Before fix: deterministic real fork/pre-exec tests failed for **both** Sender
  and Receiver with EAGAIN; old binary, test source and complete log retained.
- Unchanged baseline: 50 eight-thread desktop-library runs passed. The original
  unrecorded intermittent failure was not reproduced by that repeat.
- After fix: four lock integration tests passed 50 times with four threads;
  one additional unit test protects explicit unlock with duplicate descriptors.
  This is a confirmed product defect, without claiming unique historical causality.
- Workflow syntax validated with checksum-pinned actionlint 1.7.12. Reporter
  Python tests: 6 passed; existing A1 reporter tests: 5 passed.
- Final Rust 1.98.1 desktop suite: **226 passed, 0 failed, 17 ignored**, with
  eight test threads. This includes four fixture tests omitted by the earlier
  `--lib --tests` command and five new lock regressions.
- Final Rust 1.98.1 default suite: **131 passed, 0 failed, 2 ignored**, with
  eight test threads. These overlap the desktop cases; do not add the totals.
- Four separately selected GTK tests passed: QR expiry/replacement, Create
  validation, preview replacement and album preview lifecycle faults. Local
  backend: Broadway/Cairo, loopback only; this is not the CI Xvfb environment,
  a real desktop performance test or physical QR scanning acceptance.
- Final strict Clippy, root/native fmt, actionlint, 11 script-driver tests and
  13 deployment Python tests all passed. No deployment was performed.
- Rust 1.88.0 desktop rerun on the modified product code: **226 passed, 0 failed,
  17 ignored**, with eight test threads. Product-source input hashes match the
  final 1.98.1 desktop run; this does not reuse A1's old-code result.
- [Machine-readable evidence index](docs/validation/ci-lock-2026-09-13.json)
  records the before-fix failure and individual final report hashes, commands,
  source-index identities and results. Raw reports remain private under
  `target/ci-reports/`; the local GTK test daemon has been stopped.

The higher MSRV enables Clippy recommendations previously suppressed by the
incorrect 1.85 declaration. These were addressed without suppressing the strict
job; failure logs remain under their original report labels. The GTK preview
change retains a short RefCell borrow before installing a replacement job.

## Still required before evaluating a beta

- A2: maintainer-approved signing custody, release APK, version policy and safe
  debug-install migration; release install/upgrade and final JNI/APK checks.
- A3: a pushed, passing CI run on a frozen candidate SHA, remaining environment
  suites and release-artifact smoke. The deterministic lock defect is fixed;
  the original unrecorded intermittent event's unique cause remains unproven.
- A4: disposable VM installation/recovery acceptance, or explicitly restricted
  deployment claims.
- A5: maintainer-selected licensing/brand terms, third-party notices and the
  release artifact/documentation checklist.

No tag, Release, production-service change, phone uninstall or data clearing is
authorized by this checklist. A1 compiler builds alone do not close these items.

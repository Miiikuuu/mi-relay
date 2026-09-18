# Release validation status

No release candidate is approved. This file tracks evidence without combining
historical results into a claim of one full passing run.

Source-only public-repository preparation is tracked separately in
[the publication checklist](docs/publication.md). Licensing and documentation
work does not approve a binary release or change GitHub repository visibility.

## Large-directory scan repair — 2026-09-18

The aggregate 4 GiB cap has been removed on Android and Linux, with streaming
hashes and progress-aware timeouts. Local real-byte scans above 4 GiB and scoped
regressions are recorded in [the repair report](docs/validation/large-directory-scan-2026-09-18.md).
After initially deferring, the user authorized installation: exact-artifact
desktop replacement and Android overwrite/startup/data-preservation checks passed.
The physical phone then completed an **All files** preview of the existing
**1,238-file / 4.820 GiB** library: 1,238 missing, zero skipped or different.
Android final confirmation and bulk transfer remain untested. The native QR
popover originally did not display and was bypassed with a temporary private QR
viewer. A subsequent installed repair replaces it with an inline QR image and
makes About nonmodal. Targeted GTK tests and three installed-desktop settings
open/cancel cycles passed; physical pointer/optical rescanning remain unverified.
This is not a new full acceptance or server deployment.

## Latest existing-server rollout — 2026-09-18

**The production schema-4 gap is closed.** The authorized backend upgrade and
public-HTTPS scoped verification passed with original data, credentials and
unrelated services preserved. A newly discovered premature-exit-ACK HTTP 500 was
reproduced, corrected to HTTP 409 and revalidated on the actual server. Final
Rust 1.98.1 and 1.88.0 tests each pass 245/0/23; strict Clippy passes. The actual final static
binary passed upgrade/recovery rehearsal before installation.

See [rollout, backups, artifact identities and limits](docs/server-clean-exit-2026-09-18.md).
This does not qualify a frozen release candidate, fresh installer or the deferred
screen-off/network-handover tests. The server smoke uses synthetic protocol exit
receipts; native phone/desktop cleanup evidence remains the separate run below.

## Latest physical continuation after repairs — 2026-09-18

**The repaired APK's scoped native chain passed on the connected phone.**
User-approved overwrite installation preserved original data and signing
identity. Native existing-directory initialization, empty-file rejection and
recovery, 8 MiB transfer, six Linux acknowledgements and three-party clean exit
passed. Final phone state matched its baseline; both test Folder receipts and
the source grant were removed while original files and conflict history stayed
intact. Test services and the USB route were stopped; no production upgrade,
commit or push occurred. See [physical repair acceptance](docs/validation/phone-repair-acceptance-2026-09-18.md).

This local USB-routed test does not qualify natural long screen-off operation,
unplugged/network handover, optical QR, sustained performance or a release
candidate. Production remains outside this run's scope.

## Latest authorized local repair acceptance — 2026-09-18

**Local AC-01, AC-02 and AC-03 repairs passed reacceptance.** Root/native format
and strict Clippy pass; Rust 1.98.1 and 1.88.0 each pass 244 ordinary tests with
23 ignored. The repaired ordered Android regression passes 6/6, the full emulator
sequence passes **91/91** (including two added ownership regressions), and JVM
tests pass 110/110 with the explicit Linux QR fixture. Lint has 0 errors and
18 advisory warnings. Deployment unit tests pass 14/14, and the actual isolated
schema-4-to-5 `upgrade --apply`/backup/recovery rehearsal passes.

At the end of that local run, the new debug APK had been tested on the dedicated
emulator but not yet on the phone; the subsequent physical run is recorded above.
Production was still at schema 4 in that local run; the later upgrade is recorded
above. No release candidate,
real systemd/fresh-install rollout, optical QR or long screen-off gate is approved.
No commit or push was made. See [repair evidence](docs/validation/local-repair-acceptance-2026-09-18.md)
and [machine-readable results](docs/validation/local-repair-acceptance-2026-09-18.json).

## Earlier development acceptance — 2026-09-18

**Not accepted yet.** The current dirty checkout passed Rust desktop 244/0/23
(passed/failed/ignored) on both 1.98.1 and 1.88.0, Android JVM 110/0/0, explicit
GTK/JNI checks, installed Linux stress, local transport boundaries and direct
schema-4-to-5 migration/recovery. These overlapping scopes are not one summed
test count or acceptance of a frozen candidate SHA.

The full Android emulator sequence recorded **87 passed / 2 failed**. Fresh
exit controls passed, and an ordered photo-fixture → exit reproduction confirmed
unowned test-staging contamination. Root/native formatting and strict Clippy
also fail. The deployment rehearsal does not accept schema 4 → 5, production
still runs schema 4 without clean exit, and the physical phone disconnected
before fresh acceptance. No application fix or production upgrade was made.

After reconnection, the physical phone's user-approved overwrite installation,
native existing-directory chain, empty-source rejection/recovery, six receiver
acknowledgements and three-party clean exit/removal passed with original-data
preservation. See the [scoped physical continuation](docs/validation/phone-acceptance-continuation-2026-09-18.md).
This local USB-routed run does not close the other failures, production upgrade,
optical QR or long screen-off gates.

See the [acceptance report](docs/validation/full-acceptance-2026-09-18.md) and
[machine-readable evidence](docs/validation/full-acceptance-2026-09-18.json).
The remaining release gates below are not closed by this run.

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

## Subsequent validation and remaining gates

### Subsequent ordinary CI (prior baseline only)

[GitHub run 34746258569](https://github.com/Miiikuuu/mi-relay/actions/runs/34746258569)
passed all five jobs on `fb539cf65cea6e978baf0ad0d0157660954fefe7` from clean checkouts.
Both Rust toolchains passed default 131/0/2 and desktop 226/0/17
(passed/failed/ignored); each ran four lock cases 25 times. GTK/Xvfb: 3 passed;
Python: 24 passed. Android JVM: **89 passed, 0 failed, 1 conditionally skipped**
(the explicitly provided Linux QR fixture was absent); lint: 0 errors/17 warnings;
debug build and host JNI smoke passed. All five artifact ZIP digests and 23 report
input/log hashes were verified. See [summary](docs/validation/github-ci-fb539cf.json).
The raw ZIPs are retained locally under `target/github-ci-34746258569-K7hOdY2n/`.
This is not acceptance of the subsequent local-signing preparation changes.

### A2 local-signing preparation (2026-09-13)

The maintainer selected **local signing, no release private key in CI**.
[Local signing and migration boundaries](docs/android-release.md) describe key
creation by the maintainer in a private terminal, encrypted offline backup,
version/identity checks, unsigned preparation and read-only artifact verification.
No production key has been generated/read, and no APK has been release-signed,
installed or published by this preparation work. The existing debug phone is
unchanged; safe migration of its credentials/grants/history is **not implemented**.

Local validation passed:

- Real unsigned release APK, versionName 0.1.0 / code 2, with no signing material
  and `debuggable=false`; APK ZIP alignment and all four packaged native libraries'
  ELF machine/16 KiB LOAD alignment passed. Its SHA-256 is
  `0632ab5c008938bf1c5a59fbee7deaa7949155fbb6173a2aecbe510c06e90970`.
  This is a dirty development build, not a selected stable release.
- 18 new release-guard tests passed; all Python driver tests: 29 passed.
- Three real Gradle negative cases rejected missing metadata, mismatched Cargo
  version and a too-low versionCode. Three CLI negatives rejected a debug APK,
  an unsigned APK presented for release acceptance and noninteractive signing.
  Their expected nonzero exits/logs are retained, not rewritten as successful commands.
- Debug build and JVM tests: 89 passed / 0 failed / 1 conditional Linux-QR-fixture
  skip; lint: 0 errors / 17 warnings. A standalone Gradle invocation initially
  selected another debug certificate because its Android user home differed.
  That APK was never installed. Rebuilding via `build.sh` restored the original
  local debug certificate and retained versionName 0.1.0-dev / code 1. Its final
  JVM/lint tasks reused this turn's results; they are not counted as another run.
- Workflow syntax and `git diff --check` passed. This patch has not been pushed
  or checked by remote CI; the prior `fb539cf` result is separate.

[Machine-readable results](docs/validation/android-release-preparation-2026-09-13.json)
record each command, source-index/log/report hash, expected exit code and APK/native
hash. Signing orchestration tests use fake tools/material and do not certify
cryptographic signing or a real APK upgrade. Actual maintainer key creation,
offline-backup verification, signing and release install/upgrade remain pending.

### Remaining gates

- A2: actual key creation/backup verification, release APK and safe
  debug-install migration; release install/upgrade and final JNI/APK checks.
- A3: repeat CI on the final frozen candidate SHA, remaining environment
  suites and release-artifact smoke. The deterministic lock defect is fixed;
  the original unrecorded intermittent event's unique cause remains unproven.
- A4: disposable VM installation/recovery acceptance, or explicitly restricted
  deployment claims.
- A5: maintainer-selected licensing/brand terms, third-party notices and the
  release artifact/documentation checklist.

No tag, Release, production-service change, phone uninstall or data clearing is
authorized by this checklist. A1 compiler builds alone do not close these items.

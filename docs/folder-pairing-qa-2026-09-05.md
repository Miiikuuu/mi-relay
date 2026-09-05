# Folder pairing QA — 2026-09-05

Local development acceptance only. No user VPS upgrade, physical-phone install,
real file transfer or Git push was performed in this change. The dedicated
`MiRelay_API36_QA` emulator and temporary loopback servers used synthetic files.

Subsequent authorized VPS deployment and the phone-installation status are recorded
separately in [deployment follow-up](pairing-deployment-2026-09-05.md).

## Implemented

- Per-Folder sender/receiver identities and isolated queues; administrator authority
  is separate from the backward-compatible legacy device token.
- Ten-minute single-device invitations, idempotent claims, verification and explicit
  receiver confirmation. Pending, wrong-role and foreign-Folder requests fail closed.
- Hash-only server credential storage; bounded concurrent Folder database work,
  bounded setup response sizes, no redirects, no blind setup-mutation retries.
- Linux path selection plus invitation creation/check/confirm/replace controls.
  Save checks the connection on a worker before changing the local registry.
- Android existing-directory initialization during creation, opt-in history,
  prepared baselines and explicit Auto activation. Encrypted draft credentials
  precede claims, so retries/process recovery retain the same identity.
- Plan-by-default install/upgrade helpers, validated release hashes, managed-path
  guards, private pre-upgrade backups and read-only service/HTTPS checks.

## Results

| Verification | Result |
| --- | --- |
| Rust core, server and desktop unit/integration suite | 165 passed, 0 failed |
| Separately enabled graphical resilience tests | 4 passed: corrupt config, 4 SIGKILL timings, screenshot failure exit status, idle sample |
| Rust Clippy, all targets with desktop, warnings denied | Passed |
| Android Rust JNI host compilation | Passed |
| Android debug APK, ARM64 and x86_64 JNI builds | Passed |
| Android JVM tests | 25 passed, including schema 1/2 → 3 migration and prepared baseline/history consent |
| Android lintDebug | Passed |
| Full emulator runtime / Auto / UI / notification suite | 49 passed (13 + 12 + 23 + 1) |
| Manual and Auto uploads, external process SIGKILL/resume | Both passed; 11 legacy deliveries received/hashed/ACKed by Linux |
| New paired Folder, real Android UI → JNI → server → Linux | Passed separately; one 17,321-byte new file, exact bytes/hash and ACK |
| Deployment helper guard/unit tests | 5 passed; root installation/upgrade not executed |

The general Rust run reported six ignored cases: four GUI tests were then run
separately and passed; the host-JVM JNI integration was not re-run in this pass,
and the crash-worker helper is invoked indirectly by the crash tests.

New pairing verification includes cross-Folder and role isolation, missing admin,
reserved queue IDs, version negotiation, duplicate auth, oversized/malformed setup,
expiry, simultaneous claims, same-credential retries, hash-only persistence,
revocation/stale confirmation, restart, schema-1 preservation and scoped tus resume
(paused after 65,536 bytes, then completed a 262,144-byte payload). Client tests
reject redirected credentials, wrong identity/role/state, oversized responses and
credential-echoing error bodies.

The paired Android test selects an existing directory with historical content,
leaves history consent off, verifies no initial transfer/Auto, compares and confirms
the pairing, adds a new document before first Auto activation, and observes a real
WorkManager upload. The corresponding scoped Linux receiver verifies bytes/hash
and acknowledges it. This uses the activation-triggered scan, **not** a claim that
a full 30-minute periodic/Doze/OEM background cycle has been observed.

GTK idle sample: 8 seconds, two Folders, no transfer; sampled CPU 0.0% of one core
and RSS stable at 185,564 KiB (~181 MiB). This is a local debug-build observation,
not a hardware-independent performance guarantee or an active-transfer benchmark.
No Java/native fatal or MiRelay ANR markers were found in the full emulator log.

## Reproducible evidence (ignored local artifacts)

- Rust: `target/folder-pairing-tests.log`, `target/folder-pairing-desktop-qa.log`.
- Full emulator suite: `android/.local/device-qa-xjnp83st/results.json`.
- UI rerun/settled screenshot: `android/.local/device-qa-fc0ywmlx/`.
- Paired end-to-end receiver: `android/.local/device-qa-6iax3qfr/results.json`,
  `paired-*-sync.txt`, and the scoped receiver's state/content under that report.
- JVM XML: `android/app/build/test-results/testDebugUnitTest/`.
- APK: `android/app/build/outputs/apk/debug/app-debug.apk`;
  SHA-256 `954be0e3c2fb3c2454d5dcbc3ed4a33390cbd39f5c38443f48d3084fcba1fd20`.

## Remaining release work

- Root installer/upgrade acceptance and recovery rehearsal on a disposable VPS.
- Linux secure credential persistence: receiver tokens remain session-only; keep
  a private copy or provide the configured environment variable after restart.
- QR scanning (copy/paste invitation is implemented), receiver-credential recovery
  and full server Folder lifecycle/quota administration.
- Physical-phone source-provider/background scheduling checks after an explicitly
  approved server/phone upgrade. Current personal deployment still uses the old
  protocol/legacy credentials and was not automatically migrated.
- Bidirectional sync, Android receiving and Linux automatic sending are outside
  this change. See [setup flow and protocol](folder-pairing.md).

# CI and environment-specific acceptance

The repository workflow is ordinary validation, **not a release/signing pipeline**.
It runs on `push` to `main`, pull requests and manual dispatch, on disposable
GitHub-hosted Ubuntu 24.04 machines. Creating this file locally does not establish
that GitHub checks have run or passed; the pushed candidate must receive checks.

## Jobs

| Job | Entry points | Scope |
| --- | --- | --- |
| Rust 1.88.0 / 1.98.1 | Default and desktop `cargo test --locked --all-targets`, eight test threads | Independent toolchain baselines; no blanket ignored-test execution |
| Same two Rust jobs | `directory_locking`, four threads, 25 repetitions | Fork/pre-exec ownership and same-state handoff; repeats are not unique cases |
| Quality | Root/native fmt, strict all-feature Clippy, deployment/script Python tests, actionlint, publication hygiene | Static/build/test-driver checks; hygiene is not a complete secret audit |
| GTK | Five exact ignored tests under separate Xvfb/D-Bus sessions | QR expiry/inline interaction, nonmodal About, Create validation and thumbnail lifetime; not a performance benchmark |
| Android | Explicit SDK bootstrap, both JNI ABIs, debug APK, JVM tests, lint and host JNI smoke | No phone/emulator installation, release key or release upgrade acceptance |

The Android job deliberately uses the existing `--accept-sdk-license` bootstrap
on its disposable runner. Android SDK license prerequisites remain documented in
the Android README. It does not invoke a physical-device harness. The runner's
debug key is disposable and never included in uploaded artifacts. It is not an
upgrade identity for an existing developer or future release installation.

`scripts/ci-run.py` records the actual checkout commit, dirty state, input hashes,
selected environment, commands, exit codes, log hashes and per-run Rust totals in
`target/ci-reports/<label>`. Timeout/start failures remain failures. Exact test
filters have minimum executed-test guards, so a renamed/missing test cannot
silently pass by executing zero cases. Repetitions remain separate records.

Each label must be unused; the reporter refuses to overwrite previous evidence.
For example, from the repository root:

```bash
python3 scripts/ci-run.py --label local-locks --repeat 25 --min-tests 4 --timeout 90 -- \
  cargo test --locked --test directory_locking -- --test-threads=4
```

Only the report directory and selected Android JUnit/lint reports are uploaded,
with 14-day retention. Save needed candidate evidence before it expires. Do not
upload `target/`, `android/.local/`, app databases, SDK/debug keys or phone backups.
Do not pass real credentials as reporter command arguments.

## Trust boundary

- Top-level token permissions are `contents: read`; checkout does not persist
  credentials. There are no production, signing or private-device secrets.
- No `pull_request_target`, privileged follow-up workflow, self-hosted runner,
  deployment, release/tag creation or write-permission job is included.
- Actions are pinned to full commits. The actionlint download is versioned and
  checksum-verified. No shared executable build cache is restored from PR jobs.
- Distribution signing is local only. Signing private keys and passwords must
  not enter CI; ordinary PR jobs must not receive production or signing secrets.

These choices follow GitHub's [secure-use guidance](https://docs.github.com/en/actions/reference/security/secure-use).
Branch protection, required-check configuration and a passing frozen-candidate
run are still separate maintainer actions; they are not changed by this patch.

## Ignored-test classification

Ignored Rust tests are deliberately not passed to a blanket `--include-ignored`.
The table groups representative entry points; consult the current test output
for the exact total. Run each GTK test in its own process; a single test thread
does not make GTK initialization across multiple libtest worker threads safe.

| Category | Tests / entry | Treatment |
| --- | --- | --- |
| Parent-driven subprocess helper (1) | `tests/process_recovery.rs::crash_worker` | Invoked by the ordinary crash-recovery parents with synthetic arguments; never directly enumerate/run it |
| JNI environment (1) | `tests/android_jni.rs::java_rust_tus_server_linux_receiver_round_trip` | Separate JDK/JNI/full-chain entry in Android README; host JNI smoke alone is not this test |
| GTK CI subset (5) | QR expiry, inline QR interaction, nonmodal About/artwork, Create validation, preview replacement | Exact allowlist in CI, one process per test |
| Additional GTK UI | Album virtualization/viewer/performance; album preview lifecycle faults; appearance checks | Dedicated graphical acceptance; hardware-dependent metrics must stay separate |
| Desktop application integration (6) | `tests/desktop_directory.rs` (1), `tests/desktop_resilience.rs` ignored tests (5) | Isolated real app/relay and graphical session; startup kills, config fault, screenshot negative and performance diagnostics |
| Decoder CPU diagnostic (1) | `album_decoder_idle_control` | Explicit diagnostic, no GTK window; do not count as ordinary functional coverage |
| Fixture export (1) | `export_linux_qr_fixture` | Requires a fresh synthetic output path; fixture generation is not optical scan acceptance |
| Physical optical acceptance (1) | `show_private_qr_for_phone_acceptance` | Requires private invitation, agreed monitor and a cooperating user/phone; never CI |

The Android emulator suites, physical phone Auto/pairing/upgrade, real deployment
VM/systemd and final release artifacts still need their documented acceptance
environments. CI success alone does not close the beta checklist.

# Directory ownership lifecycle investigation — 2026-09-13

## Evidence and attribution

The historical QR validation described one intermittent parallel receiver
recovery lock failure, but no corresponding raw failure log was found in the
QR evidence directory during this investigation. Do not invent its exact stack
or claim a unique historical cause from a successful serial rerun.

The unchanged A1 Rust 1.98.1 desktop library binary (base `e50afbd`, only A1
metadata overlays) was run **50 times with eight test threads**. All passed.
This repeat did **not** reproduce the original intermittent event.

A new deterministic integration test pauses a real unrelated `/bin/true` child
between fork and exec using two pipes. It then drops the parent Sender/Receiver
and immediately reopens the same directory and ledger, while the child is still
paused. On the old implementation **both cases fail with EAGAIN (11)** and
`Another receiver/sender owns this directory`. Exit 101, complete failure log,
old test source and binary were retained before changing product code.

This demonstrates a **product ownership-lifetime defect**, not shared test
directory names: every test uses independent temporary directories. The forced
fork window proves the mechanism, but does not prove that it was the sole cause
of the earlier unrecorded failure.

## Mechanism and fix

Linux [flock locks belong to open file descriptions](https://man7.org/linux/man-pages/man2/flock.2.html),
which fork/dup descriptors share. O_CLOEXEC closes them at exec, not at fork.
Closing only the parent's descriptor therefore need not release the lock while
a child is still before exec. Explicit unlock ends that ownership without waiting
for an unrelated child. `StateLock` already used this approach; the directory-root
locks in both Sender and Receiver did not.

The new private `DirectoryLock` explicitly unlocks on drop, including error
paths. It retains nonblocking acquisition and the existing live-owner rejection;
no sleep, arbitrary retry delay or global test-serialization workaround is added.

The root guard is also declared before the state guard. Rust drops struct fields
in [declaration order](https://doc.rust-lang.org/reference/destructors.html), so a
state-lock waiter cannot be admitted while the old object still owns the root.
Both roles use the same guard. The filesystem/path validation and persisted
ledger formats are unchanged.

Five new regressions cover:

- Explicit release with a duplicate descriptor and preservation of a successor's
  ownership when the stale descriptor later closes (unit test).
- Actual pre-exec fork windows for Receiver and Sender, including rejection of
  competing ledgers while an owner is live and preservation of an existing file.
- Receiver and Sender same-state waiter handoff, 64 attempts per test.

The child executes only async-signal-safe syscalls before exec. It has a bounded
10-second wait, and the parent releases/joins it even on an assertion failure.
No existing app, phone, relay or user directory participates.

## Current verification

- Fixed fork-window tests: 2/2 passed.
- Complete four-case integration suite: **50 parallel repetitions passed**
  (four threads; 200 repeated test executions, not 200 unique scenarios).
- Reporter/workflow checks and full root regressions are recorded separately in
  [release validation](../RELEASE_VALIDATION.md).

Private evidence: `target/ci-lock-audit-DFDGaQlJ/` (before/failure and unchanged
baseline repetitions), `target/ci-reports/locks-stress/` (after), and each named
full-suite report. The old failing test binary SHA-256 is
`339e12518aaa001be8d2a80f6301176f71825120187b28ef13a803ae9cb4d24a`.

This patch fixes normal ownership teardown with fork-inherited descriptors. It
does not promise immediate lock release after SIGKILL while another process still
holds a descriptor, nor general NFS/SMB semantics. There is no schema migration,
release, server deployment, phone installation or data deletion in this
investigation. The evidence above was collected before committing/pushing the fix.

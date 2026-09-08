# Physical-device acceptance — 2026-09-08

The functional checks completed, with one reproducible diagnostic/usability
finding. A real periodic task also executed and its file reached Linux, but the
phone returned to the foreground at execution time: **strict unattended,
screen-off triggering is not verified by this run**. Final configuration
restoration and preservation checks completed. This is bounded development-build acceptance,
not exhaustive certification.

## Artifact and environment

- Android: physical vivo V2329A / PD2329, Android 16 / API 36, ARM64.
- Installed development APK SHA-256:
  `a508d83fa73b9eaab2eb1a237c31207451c215eb562eb72fdbd1c39a703e3327`.
  It is the same artifact covered by the
  [September 6 regression report](brand-integration-2026-09-06.md).
- Linux: the actual GTK release application, not a substitute sender/receiver.
  Binary SHA-256:
  `86e45173c4ef23e0ef04077eab8ae350f4db9302df5e34f114b1ec3230ea3566`.
- Transport: the existing HTTPS VPS and paired, isolated `Init-test` Folder.
  The phone used its real cellular connection. No relay, TLS, proxy, firewall,
  global battery setting, or system scheduling override was changed.
- The phone remained connected to USB power and ADB. These observations cannot
  establish unplugged battery use, deep Doze behavior, or all OEM restrictions.

## Isolation and evidence

The existing phone database contained 3 Folders and 8 uploaded transfers. After
checking that no transfers were queued or uploading, MiRelay alone was stopped
briefly for a private app-data backup and immediately reopened. SQLite integrity
and foreign keys were checked. The existing Linux test directory, receiver
ledger, configuration files, and registry were backed up as well.

The pre-existing test pairing and initialized directory were reused; no new
pairing, reinitialization, uninstall, data clear, or token replacement was used.
Only generated files under the unique `qa-20260908-QoMz2N` test subdirectory were
added or modified. No real Pixiv or other user source was initialized.

Private evidence is under `target/phone-qa-20260908-QoMz2N/`. It includes backups
and application database observations and must not be published wholesale.
Live snapshots are observations, not substitutes for a stopped, consistent
recovery backup. Device-only log/memory evidence is under
`android/.local/phone-monitor-92ymnp_h/` and is likewise private and ignored by Git.

## Completed functional checks

| Scenario | Observed result |
| --- | --- |
| Paused directory receives new local files | No upload while paused; prior remote state unchanged. |
| Unmetered-only policy on a metered cellular connection | No transfer was created; the network restriction was respected. |
| Resume the initialized directory | Existing pairing and history were retained; new files were discovered without another initialization. |
| More than one staging batch | 31 files crossed the 20-file batch boundary automatically, with an observed continuation worker. |
| Arbitrary non-image bytes | A 256 KiB binary and two later 8 MiB binaries arrived with matching actual phone/Linux SHA-256. |
| Relative paths | Nested directories, spaces, and a Chinese filename were preserved. |
| Same bytes at different paths | Both separately named files arrived; content deduplication did not erase one path. |
| Upload versus receipt | All initial 31 uploads were observed without ACK before Linux received them. Linux then verified, wrote, and acknowledged all 31. |
| Same size and mtime, different content | A synthetic source was updated while restoring its original size and mtime. The content change was detected and delivered. |
| Existing Linux content conflict | A deliberate local edit was preserved as a history file, and the phone showed `Synced · conflict copy kept`. |
| SIGKILL during upload | The exact MiRelay PID was killed after the app reported 7 MiB of an 8 MiB test upload. The system restarted the task without an ADB app launch. The same tus session completed, confirmed by its exact VPS upload record, final content hashes, and Linux ACK. |
| Repeated checks and receives | Transfer rows and directory-file ledger rows remained identical across repeat checks; no duplicate version was generated. |
| Invalid source and retry | A zero-byte file prevented a complete scan. After removing that exact generated empty file, the existing retry and stability follow-up automatically sent its valid neighbor. See the usability finding below. |
| Rename / no deletion propagation | Renaming one generated source created the new Linux path while retaining the old Linux file, with both hashes verified. |
| Linux GTK automatic receive | After enabling it for this test Folder only, the GTK app automatically received and acknowledged the two retry/rename files. |

The first 31-file batch contained 265,312 bytes. Two additional binaries were
8,388,608 bytes each. These are modest functional samples, not a large-directory
or maximum-file-size stress test.

Final totals: **37 new uploaded versions, 36 latest test paths**. All latest
paths matched their expected content hashes and had real Linux acknowledgements.
The phone had 45 uploaded transfer records including its original 8 records.

The first attempted interruption missed its window because its upload had already
finished; it is not counted as an interrupted-upload pass. A second synthetic file
was then interrupted successfully. An initial watcher also incorrectly expected
a local `offset` field in the resume file; the successful injection used the app's
recorded progress and preserved tus-session identity, followed by server-side
verification. The recorded 7 MiB is app progress, not a packet capture proving
the exact VPS offset at the instant of SIGKILL; an in-flight final chunk may
have reached the server. The verified conclusion is successful recovery using
the same persisted tus session and correct final receipt, not an exact number
of bytes retransmitted. No fault was injected into an unrelated active transfer.

## Finding: an empty file is difficult to diagnose

The current documented policy rejects zero-byte files and requires a complete
directory scan. The real phone followed that policy: an empty file blocked the
scan, including its otherwise valid neighbor, and the worker retried.

However, the displayed explanation only mentions access, nesting, loading state,
and the 5,000-entry limit. It does not identify the empty file or explain the
zero-byte restriction. The same dialog still displays `0 waiting · 0 need attention`
alongside the source-level error. This is a reproducible usability/diagnostic
problem; the safety policy itself was not changed during testing.

A follow-up should report a safe relative path and specific scan-failure reason,
and distinguish source-level failures from per-file attention counts. This run
does not silently skip invalid files or implement a product fix.

Subsequent work addressed this finding in the [source diagnostics fix](android-source-diagnostics-2026-09-08.md)
and passed a focused [physical-phone regression](phone-empty-file-2026-09-08.md).
The observations below remain the historical results for the earlier APK.

The exact empty test file was removed from the phone after verifying its size was
zero. Its local fixture remains available. The renamed source's old content also
remains in the generated fixtures and on Linux. No real user file was deleted.

## Real periodic background check

Before adding the dedicated periodic-only file, the test Folder had no
unfinished one-off check, retry, batch, or stability job. Its existing periodic
WorkManager task had a 30-minute initial delay and interval, with period count 0;
its nominal first due time was approximately 10:20:39 local time.

The UI activity helper was stopped, MiRelay was sent to the background, and a
sleep input was issued. Only then was `periodic-only.txt` added. The observer makes
read-only requests and records actual power state; it never launches MiRelay,
presses Check now, forces JobScheduler work, or simulates battery/network state.
Final conclusions must use observed screen state and WorkManager execution, not
the fact that a sleep command was sent.

Observed outcome:

- The original periodic work ID was preserved and its period count advanced from
  0 to 1. The pre-existing manual-check work ID remained finished and unchanged.
  No Check now action, forced job, or simulated constraint caused this delivery.
- The periodic scan completed at **10:25:58**, about 35 minutes 20 seconds after
  it was enqueued and 5 minutes 20 seconds after its nominal first due time.
  Its stability follow-up completed at 10:26:12; upload completed at 10:26:15.
  Linux GTK Auto received and acknowledged the file; the observer verified its
  hash and receipt at approximately 10:26:45.
- Screen state was mixed earlier in the observation. After MiRelay was seen in
  the foreground at approximately 10:18, it was explicitly returned to the
  background and a sleep input was issued again, without requesting any scan.
  Sampling then recorded locked/asleep and MiRelay not foreground through
  10:25:47. At 10:25:57 the phone was unlocked/awake with MiRelay foreground,
  immediately before the worker-success log at 10:25:58.
- A package-scoped scheduling inspection found network, battery, storage,
  background restriction and quota conditions satisfied, while the timer
  constraint was still pending after the nominal due time. No device setting
  was changed to bypass that wait.

This proves a real periodic execution and automatic Linux delivery without
Check now. It does **not** establish whether that task would have run with no
foreground transition, nor that the transition caused execution. The correct
result for a strict unattended screen-off test is **inconclusive / repeat needed**,
not a pass and not a demonstrated transfer failure. A manual receipt refresh
was used only after this observation ended, for final phone UI verification.

## Bounded resource and crash observations

Two main-process CPU samples lasted approximately 182 seconds each. Both used
the actual debug phone APK with USB charging and the existing Auto configuration;
Linux GTK Auto remained enabled. The first included early awake activity and
averaged about 0.81% of one CPU core on Android and 0.23% on Linux. The second
included 11 asleep samples followed by 2 awake samples and averaged about 0.17%
on Android and 0.22% on Linux. During the second sample's approximately 152-second
asleep portion, the measured Android process CPU ticks did not advance.

These are sampled process CPU observations, not battery drain, whole-device CPU,
frame-rate, memory-leak, or release-performance guarantees.

The UID-filtered log/memory monitor ran for **45.64 minutes**, recorded **179**
samples, and was explicitly stopped. Sampled main-process PSS ranged from
70.8 to 286.9 MiB; the last sample after reopening the app was 137.3 MiB. There
were no `FATAL EXCEPTION`, `Fatal signal`, or `JNI DETECTED ERROR` markers in
that bounded log. The final package exit history for this date contained only
the two deliberate stopped-app backup operations and the injected SIGKILL,
with no additional recorded crash or ANR exit. This is not a claim that all
possible hangs, memory pressure conditions, or long-run leaks were excluded.

## Limits and cleanup

Final preservation checks passed:

- The original 3 Folder records, encrypted credentials, and 8 transfer records
  were unchanged. All 37 new transfer rows belonged to the isolated test prefix.
- The phone test source was restored to paused, unmetered-only mode. Its tree
  grant and directory history were retained. The other Auto source's settings
  were unchanged; only its ordinary last-scan time advanced naturally.
- The Linux registry was restored to its original settings, with automatic
  receive off. All original Linux configuration files and original test-directory
  files were byte-identical to the backups, including previous history files.
- A fresh stopped-app backup passed SQLite checks, then MiRelay was reopened.
  The final UI showed the periodic test file as Synced and test Auto off.
- The bounded monitor, CPU/focus observers, and UI activity helper ended. No
  persistent diagnostic service was installed. Synthetic files and private
  backups were retained for inspection; both apps remain available for use.

No product source code, APK, or server binary was changed during this test run.
The only new repository deliverable from that testing session was this report;
no commit or push was performed during the session.

Not covered by this run: a phone reboot, unplugged/overnight background execution,
forced low battery or low storage, loss/restoration of SAF permissions, a full
cellular/Wi-Fi/VPN transition matrix, all OEM launchers/providers, production
signing, and maximum-size or maximum-directory-scale performance. Earlier
automated tests complement but do not replace these real-device gaps.

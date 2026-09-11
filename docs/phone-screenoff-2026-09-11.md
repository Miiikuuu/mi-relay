# Physical-phone screen-off Auto observation — 2026-09-11

**Not passed in the bounded observation window.** The isolated Folder's
periodic scan had not executed approximately 43 minutes after enqueueing,
about 13 minutes after its nominal earliest execution time. The generated file
never entered the Android transfer queue or the relay inventory. This does not
prove it would never execute with a longer wait, or establish a specific root cause.

A subsequent [per-app battery-control comparison](phone-battery-control-2026-09-11.md)
includes a successful connected screen-off repeat after allowing MiRelay background
power usage. The observations below remain the historical smart-control result.

## Build and isolation

- Physical vivo V2329A, Android 16/API 36, connected to USB power and ADB.
- Installed APK SHA-256:
  `6b37120ebfecabd377b4b283797d40d281e4133e44eda8ebed613afd51a9c74f`.
  No build, installation, data clear, re-pairing or reinitialization occurred.
- Reused the already initialized `Init-test` Folder. Its Auto setting was
  temporarily resumed with unmetered-only disabled; other source settings were
  not edited. The existing HTTPS relay was reachable from the observer.
- Saved a stopped, consistent app backup and Linux test data/configuration
  backups before setup. Only one new synthetic text file was added under
  `phone-screenoff-20260911-jPT3UU/` in the isolated phone source.
- No forced JobScheduler execution, simulated constraints, global battery
  changes, uninstall or credential replacement was used.

## Timeline (Asia/Shanghai)

- **13:41:54:** normal Resume action enqueued a periodic scan with a 30-minute
  initial delay and interval. Its initial one-off scan subsequently succeeded.
- **13:43:49:** after verifying that no one-off scan remained pending, the phone
  was put to sleep and the synthetic file was pushed. Before/after observations
  showed asleep, locked and MiRelay not foreground.
- **14:11:54:** nominal earliest periodic execution time, not a guaranteed deadline.
- **14:13:39:** read-only follow-up found the phone awake/unlocked, but MiRelay
  not foreground. The periodic work still had period count 0 and attempt count 0.
  No continuous observer had run across the initial waiting interval; it must
  not be described as 30 continuously verified screen-off minutes.
- **14:14:23–14:24:42:** a bounded observer ran after explicitly putting the phone
  back to sleep. All 29 samples showed asleep, locked and MiRelay not foreground;
  none showed an upload or receipt. Samples were approximately 21 seconds apart.
  The observer only made read-only requests after its initial sleep action and
  exited normally at the end of the window.
- **14:24:45:** final database and power checks confirmed the original periodic
  work ID, period count 0, attempt count 0, no new transfer and no source error.
  The last-wake/last-sleep fields were consistent with the second sleep action.

## Diagnostic evidence and limits

JobScheduler still reported `TIMING_DELAY` as unsatisfied despite an elapsed
earliest-run time. Network, battery-not-low, storage-not-low, quota, background
restriction and not-dozing conditions were satisfied. The app was in the ACTIVE
standby bucket, was not force-stopped, and its background app-op default was allow.

MiRelay also appeared in the OEM frozen-package list. This is a diagnostic lead,
not proof that OEM freezing caused the delay. No settings change or foreground
recovery experiment was performed during the observed interval. Available
UID-filtered log snapshots contained no crash markers, but these are bounded
buffer snapshots, not a continuous crash-monitoring guarantee.

The Linux GUI was launched and setup attempted to enable automatic receive.
However, that action was not followed by a persisted-state assertion. Final
inspection found automatic receive off and its registry already matching the
baseline. Accordingly, **Linux Auto was not established by this run**; successful
accessibility actions alone are insufficient evidence. This does not affect
the independently verified Android finding: there was no scan or upload to receive.

## Foreground follow-up and cleanup

After the user unlocked the phone, a **14:33:10** observation still showed the
original periodic work at period count 0, no upload, and MiRelay not foreground.
The app was then explicitly brought to the foreground to restore test settings.
At **14:33:27**, that same periodic work completed successfully; the subsequent
snapshot showed period count 1 and a newly scheduled stability follow-up.
No Check now action or forced job was used. This establishes that execution
resumed after the foreground transition, not that the transition is a proven
sole cause. It does not turn the prior screen-off observation into a pass.

Cleanup paused the source and cancelled its stability follow-up before any test
transfer appeared. The source briefly showed one waiting candidate. No end-to-end
foreground delivery is claimed. Saving the original unmetered-only option required
resuming through the existing UI and then pausing again; a first pause attempt was
too early for the asynchronous Resume operation, so actual database/UI state was
rechecked before pausing successfully. All resulting test jobs were confirmed
cancelled. These cleanup actions are outside the unattended observation window.

USB-connected, powered observations do not establish unplugged, overnight or
deep-Doze behavior. This is not a full regression suite or a product fix.

## Preservation and handoff

- Original 3 Folder/credential rows and all 48 original transfer rows preserved.
- All 42 original phone-source files and 48 original Linux files unchanged by hash.
- Linux Folder configuration files remain byte-identical, and registry settings
  match the baseline with automatic receive off; settings dialog closed.
- The synthetic phone file is retained. No files were deleted.
- The bounded observer has stopped; the ordinary Linux app remains open.
- Phone cleanup completed after unlock: the isolated source is paused with
  unmetered-only enabled and no source error; its remaining jobs are cancelled.
  A final stopped, consistent backup verified identical Folder/credential and
  transfer tables. Only test-source revision/last-scan runtime fields and the
  other source's ordinary last-scan time changed. Both devices' original file
  hashes were rechecked. MiRelay was reopened after backup.

Private, Git-ignored evidence is in `target/phone-screenoff-20260911-jPT3UU/`:
`screenoff-start.json`, `due-check-result.json`, `watch-samples.jsonl`,
`watch-ended.json`, `final-check-result.json`, `final-check-jobs.txt`,
`final-preservation.json`, `linux-restored.json`, `after-unlock-result.json`,
`foreground-before-pause-result.json`, `cleanup-verified.json` and
`restored-final-preservation.json`. App databases, backups and raw logs must not
be published wholesale.

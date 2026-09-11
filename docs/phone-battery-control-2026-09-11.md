# Physical-phone battery-control comparison — 2026-09-11

This follow-up investigates the delayed background scheduling recorded in the
[earlier screen-off observation](phone-screenoff-2026-09-11.md). It changes no
product code or APK. Times below are Asia/Shanghai.

## Authorized change

On the same vivo V2329A/API 36 phone, the per-app screen for **MiRelay** initially
selected **Background power smart control**. With user approval, it was changed
to **Allow background power usage**. The selected option was visually verified
after returning to the settings screen. No global battery policy, other app's
settings, self-start option, VPN, relay or pairing was changed.

JobScheduler subsequently showed MiRelay in the **EXEMPTED** standby bucket
rather than ACTIVE, and it was absent from the inspected OEM frozen-package list.
These are observed effects of the settings change, not proof of a particular
vendor implementation defect. The option explicitly warns of increased battery use.

The installed APK remains
`6b37120ebfecabd377b4b283797d40d281e4133e44eda8ebed613afd51a9c74f`.
The already initialized, isolated `Init-test` source was reused. A fresh stopped
app backup and Linux test-data/configuration backups were made before resuming it.

## First allowed-background run: incomplete screen-state evidence

- The periodic task was enqueued at approximately 14:47:54 with a 30-minute delay.
- A new test file was added after sleep at approximately 14:48:22. Initial samples
  included an awake/unlocked interval; the phone was put back to sleep at 14:49:29.
- USB/ADB disconnected around 15:02. The bounded observer continued recording
  observation failures and exited normally around 15:33 without observing upload.
  Its `uploaded: false` end marker is **not evidence that no upload occurred**.
- At reconnection around 16:05, the original periodic task had period count 2.
  A stability follow-up had been enqueued around 15:18:00, and the retained app
  log recorded another periodic completion around 15:48:06. The test file was
  already uploaded, with no source error, before opening MiRelay on reconnection.
- Manual Receive in the actual Linux GTK app obtained the two pending synthetic
  files, including the fixture retained from the preceding smart-control test.
  The allowed-background fixture's actual phone/Linux SHA-256 and both server
  and Linux acknowledgements were verified. Linux automatic receive stayed off.

This supports improved periodic execution but does not establish continuous
screen-off behavior during the missing interval. Power and screen state could
not be verified during the disconnect, so this first run is not a strict
single-variable comparison.

## Connected repeat

A new fixture was added after locking the phone at approximately 16:06:29,
without reopening MiRelay or requesting a manual scan. The existing periodic
work ID and period count 2 were retained; no timer reset was used. Its next nominal
earliest execution was approximately 16:18:06. A new bounded read-only observer
records screen/power state, server inventory and periodic-work state.

**Connected screen-off repeat passed.** All 35 samples across approximately
12 minutes 7 seconds showed asleep, locked, USB-powered and MiRelay not foreground.
There were no observation errors; the maximum sample gap was 21.75 seconds.
PowerManager's last-wake and last-sleep timestamps were unchanged between the
pre-due and post-upload inspections. The observer exited normally after upload.

The same periodic work ID advanced from period count 2 to 3. The pre-existing
manual-check record was unchanged. App logs recorded the periodic scan completing
at **16:18:13**, its stability follow-up at **16:18:28**, and upload at **16:18:30**.
The observer saw the new file on the relay at **16:18:36**, without foregrounding
MiRelay, a manual check, forced job execution or a settings change during the window.

After that Android-only observation ended, manual Receive in the actual Linux
GTK app obtained the new 70-byte file. Actual phone/Linux hashes and both server
and Linux acknowledgements matched. This verifies automatic screen-off Android
upload plus subsequent manual Linux receipt, not automatic Linux reception.

This provides a verified workaround on this particular vivo phone: allowing
MiRelay background power usage restored successful screen-off periodic execution
in the tested window. It supports investigating vendor power management as the
source of the earlier delay, but is not proof of a specific internal vendor bug,
exact scheduling guarantees, all OEMs, overnight operation or unplugged deep Doze.
No always-on foreground service or application-code change was introduced.

## Handoff state

Both bounded observers have ended. The authorized MiRelay-only **Allow background
power usage** setting is retained for continued use; its original value was
**Background power smart control**. This may increase battery consumption.

Original 3 Folder/credential records, 48 prior transfer records, 43 prior
phone-source files and 48 prior Linux files passed preservation checks. Synthetic
files are retained; nothing was deleted. Linux automatic receive remains off,
its registry settings match the baseline, and its Folder configuration files
were confirmed byte-identical to the backups.

Final phone cleanup completed after user unlock. The isolated source is paused
with unmetered-only enabled and no source error; its remaining WorkManager jobs
are cancelled. A final stopped, consistent backup confirmed the original
Folder/credential rows and all 48 original transfer rows were preserved. Exactly
three new rows belong to the synthetic fixtures from the smart-control run,
first allowed-background run and connected repeat. The other Auto source's
settings were unchanged; only its normal last-scan time advanced. MiRelay was
reopened after backup.

For cleanup only, the user authorized temporarily extending screen timeout.
The current user-0 value was recorded as **300000 ms (5 minutes)**, temporarily
changed to **600000 ms (10 minutes)**, then restored to **300000 ms** and read back
successfully. This happened after the screen-off observation finished and does
not affect that result. The authorized MiRelay per-app battery exemption remains
enabled, as disclosed in the handoff; no monitoring helper remains running.

## Private evidence

Private, Git-ignored backups, UI XML/screenshots, app/work databases and sample
logs are under `target/phone-battery-ab-20260911-m9aPWR/` and
`target/phone-battery-ab-20260911-BBQY6U/`. Do not publish these directories wholesale.
Key repeat evidence includes `observation-verified.json`, `connected-final-result.json`,
`connected-final-app-logcat.txt`, `received-verified.json` and `samples.jsonl`.
Cleanup evidence in the first run directory includes `cleanup-verified.json`,
`cleanup-final-preservation.json`, `timeout-before.json` and `timeout-restored.json`.

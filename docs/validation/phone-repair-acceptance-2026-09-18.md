# Physical acceptance after local repairs — September 18, 2026

**The scoped physical chain passed. Full release acceptance remains open.**

After the user confirmed the overwrite-install prompt, the repaired Android
debug APK ran on the connected vivo V2329A/API 36 with the installed Linux app
and a separate loopback relay. No uninstall, application-data reset,
instrumentation runner, production upgrade, commit or push was performed.

## Artifact identity

- Android APK: `d2a571a61bedf3b89fea23ca203c6622698363cb9529b4cbf357819b0a6bbc7b`.
- Installed Linux executable: `925eaae576c8f8d246e6fab45c73e920e53aa165e8f54aa3d3dda0c961b77714`.
- Local relay executable: `36e9f13d123fc34c927a7d23d243846c1e17cca744245df0bddabc8c1c858cf5`.
- Private evidence: `android/.local/phone-repaired-acceptance-20260918-p9sMQ4/`.
- `upgrade-verified.json`: `51a1c42ad3bc64e53f5da906ae4bf2018960517361c6da895ba929bd53957fb5`.
- `final-verified.json`: `c0135ef851b7bc993ee5aac9fcff4fde62ed5727f281560d4fdf45aa03ce62c4`.
- `teardown.json`: `decd8b572df20109a93d28c2f220199c9dafe2bc4f3f991a29639ddc02cdf0d9`.

The Android artifact is the one covered by the preceding
[local repair acceptance](local-repair-acceptance-2026-09-18.md), not the older
APK in the earlier physical continuation. Linux was not reinstalled in this run.

## Passed checks

1. Overwrite installation preserved signing certificate, UID, schema 7, database
   records and all six baseline private files/preferences. Fresh backups were
   taken; installed APK bytes were pulled and compared with the expected hash.
2. Native Linux invitation creation, Android system selection of an existing
   synthetic source directory, and visible matching verification codes. Both
   native apps completed pairing; no script supplied protocol confirmations.
3. Existing-directory initialization preview reported one identical, two
   missing, one different and one Linux-only file. Linux's regular automatic
   cycle received and acknowledged three versions. All four source hashes
   matched; original conflict content and the Linux-only file were retained.
4. Added an 8 MiB binary, a same-path modification and a zero-byte source.
   Android reported the empty-file error and queued no files in that scan;
   relay delivery count remained three. Repairing only the synthetic empty file
   to one byte and using Check now recovered synchronization. Six versions were
   acknowledged by the real Linux app; bytes/hashes and both previous conflict
   contents were verified. The error cleared without deleting the Folder.
5. Cancelling clean exit left all six deliveries and the connection intact.
   A stopped backup and cold reopen preceded the real phone-initiated exit.
   Premature removal was disabled while Linux cleanup was pending.
6. The regular Linux automatic cycle finished cleanup; server, sender and
   receiver receipts all became true. Phone Retry / check clean exit refreshed
   its completed state. Relay delivery/version/index rows and payload/unfinished
   upload files were removed. Safety locks and ownership markers were retained.
7. Linux remained at Clean exit complete with Receive disabled and no Ready
   regression for 67 seconds across 14 observations. Both apps' removal
   confirmation/cancel paths were exercised, then their test receipts removed.
8. Final phone database and protected-file snapshot exactly matched the
   pre-install baseline: zero Folders, zero transfers and six original private
   files. The test source URI grant was released. All six current phone source
   files, Linux received bytes, historical versions and Linux-only file remained
   intact. The screen timeout stayed at 300000 ms.

Only this run's desktop/relay processes and exact USB reverse mapping were
stopped/removed. Synthetic sources and private recovery backups were retained.

## Harness observations and limits

- The input method converted URL punctuation to Chinese characters. Exact-value
  checks blocked saving; temporary English input produced the expected URL.
  The original Chinese input mode was visually restored and the unused dialog
  cancelled. No global font, proxy or battery setting was changed.
- Strict accessibility locators rejected actions issued before a dialog settled
  and an outdated Save label (directory mode uses Review directory). Waiting
  for the actual controls resolved these harness sequencing issues. Early
  receipt assertions ran before the normal receiver cycle and were not counted
  as passes; final success required real acknowledgements and matching hashes.
- Linux switches responded correctly in this run. This does not resolve or
  disprove the earlier obscured-window/frame-callback observation.
- This used USB-powered, loopback HTTP transport with a separate synthetic
  Folder. Check now was used for incremental recovery. It is not acceptance of
  natural long screen-off Auto, unplugged operation, Wi-Fi/mobile handover,
  production HTTPS/Clash, optical QR, sustained performance or all media formats.
- The production server was not contacted or upgraded in this continuation.
  Its previously observed schema-4/clean-exit gap, release signing and the other
  release gates remain open. See [release status](../../RELEASE_VALIDATION.md).

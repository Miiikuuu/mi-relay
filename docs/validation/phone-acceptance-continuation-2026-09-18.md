# Physical acceptance continuation — September 18, 2026

**This scoped native chain passed; full/release acceptance remains open.**

The reconnected vivo V2329A ran the newly installed debug APK with the already
installed native Linux release app and a disposable real relay. No emulator
reset, instrumentation data clearing, script protocol acknowledgement or
production-server modification was used.

## Artifact and evidence identity

- Android APK SHA-256: `798258d4d89661ae88b6d3b644dd4028345a058eb8279b0401c315ecd41f9b31`.
- Installed Linux SHA-256: `925eaae576c8f8d246e6fab45c73e920e53aa165e8f54aa3d3dda0c961b77714`.
- Local debug relay SHA-256: `bc783bf1f7f5c035290159215d7c4343fbb1d8b6272eba04f647171527432147`.
- Private evidence: `android/.local/phone-acceptance-resume-20260918-ANRxq9/`.
- `upgrade-verified.json` SHA-256: `51a1c42ad3bc64e53f5da906ae4bf2018960517361c6da895ba929bd53957fb5`.
- `final-verified.json` SHA-256: `f68237f3835c29d051ecc5edaffed49ab32c02cb3ff9e8e9a6a2061508320af8`.
- `teardown.json` SHA-256: `80d00c64661a3c50d0e149160c1196032c11f8f90af4c5f557c53474e736d729`.

## Passed checks

1. User-approved overwrite install preserved signing identity, package UID,
   schema 7, original database records and six protected private files/preferences.
2. Actual desktop invitation creation, Android system selection of an existing
   synthetic directory, and matching visible verification codes on both devices.
3. Both native initialization previews: one identical, two missing, one
   different and one Linux-only file. Linux automatically received and
   acknowledged three versions; all four source hashes matched. The original
   conflict copy and Linux-only file survived.
4. Added an 8 MiB binary, modified an existing file and added a zero-byte file.
   Android explicitly rejected the empty source and queued no files in that
   scan; server delivery count remained three. Repairing only that synthetic
   file to one byte and using Check now recovered synchronization. Error state
   cleared after scheduling/retry; Linux's normal automatic receiver reached
   six total acknowledgements. Exact bytes/hashes and both old conflict versions
   were verified. This is not a natural periodic/screen-off Auto test.
5. Phone clean-exit confirmation Cancel left the relay untouched. A stopped
   pre-exit backup and cold reopen preceded the real phone-initiated exit.
   Premature receipt removal was disabled. Linux's ordinary automatic cycle
   completed peer cleanup; all three server/phone/receiver receipts became true.
   Phone's Retry / check clean exit refreshed its completed receipt.
6. Relay deliveries, directory versions/indexes, content and unfinished uploads
   were cleared. Phone test transfer/Auto records, credential and owned staging
   were removed. Original safety locks/ownership markers were preserved, not
   mistaken for remaining payloads.
7. Linux stayed at Clean exit complete with Receive disabled for over 66 seconds,
   with no Ready-state regression. Its removal confirmation explained original
   preservation; Cancel retained the configuration and confirmation removed it.
   Both test Folder receipts were removed through the native UIs.
8. Final Android tables and all six baseline private files matched the stopped
   pre-test snapshot. All six current phone source hashes, Linux received files,
   conflict history and Linux-only file remained intact. Test URI permission
   was released and both Folder lists were empty. Screen timeout stayed 300000 ms.

## Harness observations and boundaries

- Initially the Linux accessibility toggle actions returned success without
  their state changing. After the user made the window visible, both toggles
  completed and invitation creation worked. This supports a visibility/frame
  delivery dependency; it is not a completed root-cause diagnosis or a fix.
- Android input automation initially targeted the wrong focus and a URL was
  entered with reordered characters. Guards prevented saving that configuration.
  Later invitation comparison failures were a **harness error**: password fields
  expose bullets, not the secret. After correcting the check, the real server
  accepted the invitation and the visible verification codes matched. These
  masked-field failures must not be reported as product/input-method corruption.
- Strict native control locators also rejected transient ambiguous controls
  during dialog transitions; no ambiguous tap was executed.
- QR pairing was correctly disabled for the isolated HTTP endpoint. No optical
  QR pass is claimed. Transport was loopback plus exact-device USB reverse, not
  production HTTPS/Clash, an unplugged phone or Wi-Fi/mobile handover.
- No fresh long screen-off, overnight, all-format or performance qualification
  is claimed by this continuation. No application source fix, release signing,
  commit or push was performed.
- Owned desktop/relay processes and the exact USB mapping were stopped/removed.
  Synthetic source files and private recovery backups remain for inspection;
  no original media were deleted.

The [full acceptance findings](full-acceptance-2026-09-18.md) remain authoritative:
AC-01 quality gates, AC-02 ordered test-fixture contamination, AC-03 deployment
rehearsal and AC-04 the old production server were not fixed by this phone run.

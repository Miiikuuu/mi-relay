# Physical-phone empty-file regression — 2026-09-08

Both empty-file recovery scenarios passed on the connected vivo V2329A running
Android 16/API 36, using the actual VPS relay and Linux GTK receiver.

## Installed build and safeguards

The fixed development APK was installed in place after a fresh private backup
and matching signing-certificate verification. No uninstall or data clear was used.
All application database rows were identical immediately before and after installation.

Installed APK SHA-256:
`6b37120ebfecabd377b4b283797d40d281e4133e44eda8ebed613afd51a9c74f`.

Only the pre-existing isolated `Init-test` Folder was exercised. Generated files
used the unique `empty-qa-eB4rpg/` subdirectory of its source and destination.
No real Folder was reinitialized or re-paired. Only the test Folder's network policy
was temporarily relaxed for the current phone connection, then restored.

## Results

1. **Empty file later receives content.** A nested zero-byte file and a valid
   neighboring file were added while the test source was paused. After resuming,
   the phone identified `empty-qa-eB4rpg/nested/fill-later.txt`, explained the
   `empty (0 bytes)` restriction, and showed `0 waiting · 1 need attention
   (includes a source issue)`. Neither new file entered the transfer queue or
   server inventory. Adding content to that same empty file allowed the existing
   retry to upload both files without another manual check.
2. **Empty file is removed from the source.** A second empty file and valid
   neighbor were added, then a manual check initiated the scan. The phone identified
   `empty-qa-eB4rpg/remove-me.txt`, counted the source issue, and did not partially
   upload its neighbor. Closing and reopening the editor retained the diagnostic.
   Removing only that generated zero-byte file allowed the existing retry to send
   the valid neighbor without another manual check.
3. **Actual delivery and recovery state.** The Linux app's Receive action obtained
   all three resulting nonempty files. Phone and Linux bytes/SHA-256 matched the
   fixtures; server and Linux acknowledgements matched. Android subsequently
   recorded all three receipts, cleared the source error and showed zero issues.

The three received files were 67, 64 and 72 bytes. The removed empty test file has
a retained local copy in the private evidence directory; the three valid test files
remain on both devices for inspection.

## Preservation and cleanup

- Original 3 Folder/credential rows and all 45 original transfer rows preserved.
- All 39 original phone-source files and 45 original Linux files unchanged by hash.
- Linux configuration and Folder registry unchanged.
- Test source restored to paused with `Unmetered network only` enabled.
- Only test-source revision, last-scan time and next-version runtime fields changed.
- Temporary bounded UI-activity assistance stopped. No global phone settings changed.

Private evidence: `target/phone-empty-qa-20260908-eB4rpg/`. The final preservation
and receipt assertions passed in `final-verified.json`; scenario assertions are
recorded in `first-blocked-verified.json`, `second-blocked-verified.json`,
`first-received.json` and `second-received.json`. Screenshots include
`empty-blocked.png`, `reopened-error.png`, `recovered.png` and `restored-settings.png`.

## Limits

Zero-byte transfers are still unsupported: directory sync requires a complete
supported scan and does not silently skip them. This test verifies actionable
diagnostics and recovery after correcting the source, not zero-byte transport.
Android retry recovery was automatic after each correction; Linux reception was
explicitly requested in the GTK app. The phone app remained foreground/unlocked
for UI inspection. This is not a new unattended screen-off periodic-Auto result or
a full-device performance certification.

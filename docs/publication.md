# Source publication checklist — 2026-09-18

Scope: prepare the repository for public **experimental source** access, not
approve a stable release, produce a signed APK, or deploy a server. No public
visibility change, history rewrite, commit, push or release/tag creation has
been performed in this preparation pass.

## Prepared locally

- Owner-selected MIT license for original code/documentation; separate reserved
  branding terms and preserved third-party icon notices.
- English README, contribution guidance, security-reporting guidance, and
  explicit one-way/no-E2EE/development status and background-operation limits.
- Current historical-report files have personal home paths and private SSH
  aliases redacted without changing their reported test results.
- Original local design archive and audit input are ignored, retained locally,
  and not selected for publication. Runtime data, phone backups and signing
  materials remain excluded. Ignore rules do not remove already committed data.
- CI adds exact inline-QR/nonmodal-About tests and a bounded tracked-file hygiene
  check. No deployment or signing secrets are supplied to CI.

## Inspection evidence and limits

At inspection, the local and SSH-advertised remote main both referenced
`c558f99eaf0c992ddd1477a2bf8fe72fdee7e8d2`; the remote advertised no other branches
or tags. Local history had nine commits and 529 reachable blobs. Author emails
used a GitHub no-reply address. Hidden remote refs, server-side caches and
unreachable remote objects are not covered by this inspection.

The tracked working-tree pattern check passed. Newly added files were checked
separately, since an unstaged new file is outside its default scope. All 31
tracked raster images were checked for EXIF/common descriptive metadata and
produced no candidates; this is not a guarantee against information in pixels.

The reachable-history check reports **two historical blobs with personal home
paths**. A separate targeted review also found **four historical blobs containing
a private SSH host alias**. The reviewed patterns found no private-key headers
or common GitHub/AWS access-key strings. Long literal credential candidates were
reviewed as environment-variable names or synthetic test credentials. This is
not an exhaustive entropy scan, credential rotation assessment, third-party
dependency audit or security certification.

Reproduce the bounded checks with:

```sh
python3 scripts/audit-public.py
python3 scripts/audit-public.py --history
```

The second command currently returns **1** because old paths remain in history.
It must not be reported as a pass. Output contains rule/location information,
not the matched credential contents. Untracked/ignored files, LFS payloads,
remote-only refs and image metadata require separate review.

## Validation in this preparation pass

- Rust desktop regression: **243 passed, 0 failed, 24 ignored**, with the heavy
  greater-than-4-GiB scan excluded (its earlier evidence remains separate).
- Python script tests: **45 passed**, including six publication-audit tests.
- Deployment unit tests: **14 passed**.
- Strict all-feature/all-target Clippy, root Rust formatting, locked package
  metadata and whitespace checks passed.

These do not include a new Android/device acceptance, fresh deployment test,
release signing run, or a hosted GitHub Actions run. See
[release validation status](../RELEASE_VALIDATION.md).

## Decisions/actions still required before publication

1. Decide whether to retain the old private aliases/paths in public history or
   authorize history sanitization. Sanitization changes commit IDs and needs a
   private recovery backup plus an explicitly authorized remote update; it must
   not be a silent force push.
2. Commit only reviewed files, push the approved candidate, and inspect its real
   GitHub CI results. Recheck the exact candidate rather than treating this dirty
   working tree as a frozen release.
3. Use an authenticated GitHub owner session to verify and change repository
   visibility, enable a private vulnerability-reporting channel if available,
   and review branch protection. The unauthenticated API returned 404, which does
   not prove the repository's visibility or the user's administration rights.
   No authenticated repository-administration client was available locally.
4. Keep binary releases separate: signing identity/backups, packaged dependency
   notices and remaining acceptance gates still need their documented review.

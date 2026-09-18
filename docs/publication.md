# Source publication checklist — 2026-09-18

Scope: prepare the repository for public **experimental source** access, not
approve a stable release, produce a signed APK, or deploy a server. The initial
preparation was local only. The owner subsequently authorized a private backup,
history sanitization and a lease-protected force push; see the continuation
below. No public visibility change or release/tag creation is claimed here.

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

The initial reachable-history check reported **two historical blobs with personal home
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

Before sanitization the second command returned **1** because old paths remained
in history; that result was not counted as a pass. Output contains rule/location information,
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

## Authorized history sanitization

The owner explicitly authorized backup, sanitization and the remote history
update. A private directory outside the repository holds a verified complete Git
backup, an all-ref bundle, a 350-file working-tree archive and the commit mapping.
No private backup path, source archive or signing material is committed here.

An isolated repository rewrote ten commits, preserving author/committer data,
messages and topology. Only six historical document blobs changed: personal home
prefixes and the private SSH alias became generic examples. Every changed blob
was checked against the exact intended replacements; code and artwork remained
byte-identical in every corresponding commit. The latest tree was also identical
before and after rewriting, preserving the already-reviewed local fixes.

The sanitized candidate's 566 reachable blobs passed the bounded history scanner.
An additional check of all reachable objects found none of the exact reviewed
personal-path, SSH-alias, production-IP or device-serial patterns. Script tests
(45), deployment tests (14) and workflow syntax checks were rerun and passed.
This does not expand the earlier scan's security guarantees or acceptance scope.

The push is scoped to `main` with an explicit expected old remote SHA, never a
mirror push. It must fail if the remote branch changes in the meantime. Old
objects remain recoverable in private local backups/reflogs. A rewritten branch
does not prove GitHub has purged cached commits, hidden refs or other clones;
no server-side erasure is claimed. Historical report hashes identify earlier
evidence, not the rewritten current candidate.

## Actions still required before public visibility

1. Verify the approved pushed candidate and inspect its real GitHub CI results.
   A local test pass does not establish a hosted CI pass.
2. Use an authenticated GitHub owner session to verify and change repository
   visibility, enable a private vulnerability-reporting channel if available,
   and review branch protection. The unauthenticated API returned 404, which does
   not prove the repository's visibility or the user's administration rights.
   No authenticated repository-administration client was available locally.
3. Keep binary releases separate: signing identity/backups, packaged dependency
   notices and remaining acceptance gates still need their documented review.

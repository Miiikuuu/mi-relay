#!/usr/bin/env python3
"""Read-only publication hygiene check. Reports locations, never matched contents.

Default: tracked working-tree files. --history: reachable Git blobs in all local
refs. This bounded pattern check is not a complete secret/security scanner.
Untracked/ignored files, remote-only refs, LFS objects and image metadata need
separate review. Exit 1 means findings; exit 2 means an incomplete/erroring scan.
"""
import argparse
import json
from pathlib import Path
import re
import subprocess

RULES = {
    "private-key": re.compile(rb"-----BEGIN (?:RSA |EC |DSA |OPENSSH |ENCRYPTED )?PRIVATE KEY-----"),
    "service-token": re.compile(rb"\b(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,}|AKIA[0-9A-Z]{16})\b"),
    "personal-home": re.compile(rb"/home/" rb"(?!user/|alice/|example/)[^/\s\"']+/"),
}
SENSITIVE_SUFFIXES = {".key", ".pem", ".p12", ".pfx", ".pk8", ".jks", ".keystore", ".db", ".sqlite", ".sqlite3", ".apk"}
LIMIT = 16 * 1024 * 1024


def findings(path, body, revision=None):
    result = []
    def add(rule, line=None):
        item = {"path": path, "rule": rule}
        if line is not None:
            item["line"] = line
        if revision is not None:
            item["blob"] = revision
        result.append(item)
    p = Path(path)
    if p.suffix.lower() in SENSITIVE_SUFFIXES or p.name in {".env", "server.env", "local.properties"}:
        add("sensitive-file")
    # Detect private-key headers even in binary containers. No decoded values
    # are printed or persisted; line locations are sufficient for local review.
    for name, pattern in RULES.items():
        for match in pattern.finditer(body):
            add(name, body.count(b"\n", 0, match.start()) + 1)
    return result


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args])


def scan(root, history=False):
    result, count = [], 0
    if history:
        objects = git(root, "rev-list", "--objects", "--all").splitlines()
        # cat-file distinguishes commits/trees from blobs before reading data.
        entries = git(root, "cat-file", "--batch-check=%(objectname) %(objecttype) %(objectsize)",
                      "--batch-all-objects").splitlines()
        metadata = {parts[0]: (parts[1], int(parts[2])) for line in entries if (parts := line.split())}
        for item in objects:
            oid, _, name = item.partition(b" ")
            kind, size = metadata[oid]
            if kind != b"blob":
                continue
            if size > LIMIT:
                raise ValueError("Oversized Git blob needs manual review: " + oid.decode())
            body = git(root, "cat-file", "blob", oid.decode())
            result.extend(findings(name.decode(errors="replace"), body, oid.decode()))
            count += 1
    else:
        for name in git(root, "ls-files", "-z").split(b"\0"):
            if not name:
                continue
            path = name.decode()
            p = root / path
            if p.is_symlink():
                raise ValueError("Tracked symlink needs manual review: " + path)
            if not p.exists():
                continue  # A tracked deletion; history is checked separately.
            if p.stat().st_size > LIMIT:
                raise ValueError("Oversized tracked file needs manual review: " + path)
            result.extend(findings(path, p.read_bytes()))
            count += 1
    return {"scope": "reachable-history" if history else "tracked-working-tree", "files_or_blobs": count, "findings": result}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--history", action="store_true")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    try:
        report = scan(root, args.history)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(json.dumps({"error": str(error)}))
        return 2
    print(json.dumps(report, indent=2))
    return 1 if report["findings"] else 0


if __name__ == "__main__":
    raise SystemExit(main())

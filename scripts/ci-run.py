#!/usr/bin/env python3
"""Run one CI command (optionally repeatedly), retaining failures and provenance.

Only target/ci-reports is intended for artifact upload, never the entire target
tree, Android .local, keys or device backups. Repeated runs are not unique cases.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import signal
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def stop(process):
    # The process group belongs solely to this invocation, including compiler
    # or test children. Never signal a name, user session or unrelated process.
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        pass
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    process.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--label", required=True)
    parser.add_argument("--repeat", type=int, default=1)
    parser.add_argument("--timeout", type=int, default=1200)
    parser.add_argument("--min-tests", type=int, default=0,
                        help="Reject successful Cargo filters that actually execute too few tests")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if (not re.fullmatch(r"[a-z0-9][a-z0-9-]{0,63}", args.label)
            or not 1 <= args.repeat <= 100 or not 1 <= args.timeout <= 3600
            or args.min_tests < 0 or not command):
        parser.error("Use a safe label, 1–100 repetitions, 1–3600 second timeout and a command")
    report = ROOT / "target/ci-reports" / args.label
    report.mkdir(parents=True, exist_ok=False)
    git = lambda *rest: subprocess.check_output(["git", *rest], cwd=ROOT)
    # Hash uncommitted inputs too. A dirty checkout must never be advertised as
    # acceptance of HEAD. The index contains paths/hashes, not file contents.
    names = git("ls-files", "--cached", "--others", "--exclude-standard", "-z").decode().split("\0")
    inputs = {name: sha(ROOT / name) for name in sorted(set(names) - {""})
              if (ROOT / name).is_file() and not (ROOT / name).is_symlink()}
    index = report / "inputs.json"
    index.write_text(json.dumps(inputs, indent=2) + "\n")
    result = {
        "commit": git("rev-parse", "HEAD").decode().strip(),
        "dirty": bool(git("status", "--porcelain")),
        "source_index_sha256": sha(index),
        "host": platform.platform(),
        "started_at_utc": datetime.now(timezone.utc).isoformat(),
        "command": command,
        "repeat_requested": args.repeat,
        "environment": {key: os.environ[key] for key in (
            "RUSTUP_TOOLCHAIN", "CARGO_BUILD_JOBS", "CARGO_INCREMENTAL",
            "CARGO_PROFILE_DEV_DEBUG", "GDK_BACKEND", "GSK_RENDERER",
            "GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT") if key in os.environ},
        "runs": [],
    }

    def save():
        (report / "result.json").write_text(json.dumps(result, indent=2) + "\n")

    save()
    for attempt in range(1, args.repeat + 1):
        log = report / f"run-{attempt:03d}.log"
        start = time.monotonic()
        print(f"{args.label}: run {attempt}/{args.repeat}: {command}", flush=True)
        with log.open("x") as output:
            try:
                process = subprocess.Popen(command, cwd=ROOT, stdout=output,
                                           stderr=subprocess.STDOUT, start_new_session=True)
            except OSError as error:
                output.write(f"Could not start command: {error}\n")
                code = 127
            else:
                try:
                    code = process.wait(timeout=args.timeout)
                except subprocess.TimeoutExpired:
                    stop(process)
                    code = 124
                except BaseException:
                    stop(process)
                    raise
        text = log.read_text(errors="replace")
        counts = [tuple(map(int, match)) for match in re.findall(
            r"test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;", text)]
        command_code = code
        if code == 0 and sum(row[0] + row[1] for row in counts) < args.min_tests:
            code = 1
            print(f"Expected at least {args.min_tests} executed tests; refusing empty/partial success", flush=True)
        result["runs"].append({"exit_code": code, "command_exit_code": command_code, "log": log.name,
                               "log_sha256": sha(log), "elapsed_seconds": round(time.monotonic() - start, 3),
                               "rust_counts": dict(zip(("passed", "failed", "ignored"),
                                                       [sum(row[i] for row in counts) for i in range(3)]))
                               if counts else None})
        save()
        print(text[-4000:], flush=True)
        if code:
            raise SystemExit(code if code > 0 else 1)
    result["completed"] = True
    save()


if __name__ == "__main__":
    main()

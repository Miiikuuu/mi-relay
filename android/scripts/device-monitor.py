#!/usr/bin/env python3
"""Bounded, read-only MiRelay diagnostics on ONE explicitly selected device.

Does not install/launch/stop apps, clear logs/data, change settings or read files
being transferred. UID filtering follows app restarts without following other
apps. Local reports may contain private app log messages: keep them private.
"""
import argparse
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import tempfile
import threading
import time

ANDROID = Path(__file__).resolve().parents[1]
ADB = ANDROID / ".local/sdk/platform-tools/adb"
PACKAGE = "io.mirelay.android"
MAX_LOG_BYTES = 20 * 1024 * 1024


def package_uid(output):
    match = re.search(r"^package:io\.mirelay\.android uid:(\d+)\s*$", output, re.MULTILINE)
    if not match:
        raise RuntimeError("MiRelay is not installed for the current Android user.")
    return match.group(1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True, help="Exact authorized ADB serial; never selects a device automatically")
    parser.add_argument("--minutes", type=int, default=30, choices=range(1, 181), metavar="1..180")
    args = parser.parse_args()
    os.umask(0o077)

    def adb(*parts, timeout=8, check=True):
        result = subprocess.run([str(ADB), "-s", args.serial, *parts], capture_output=True, text=True,
                                errors="replace", timeout=timeout)
        if check and result.returncode:
            raise RuntimeError("ADB command failed or the selected device disconnected.")
        return result.stdout

    def uid():
        return package_uid(adb("shell", "cmd", "package", "list", "packages", "-U", "--user", "current", PACKAGE))

    if adb("get-state").strip() != "device":
        raise RuntimeError("Selected device is not authorized and connected.")
    app_uid = uid()
    start_device_time = adb("shell", "date", "+%s.%3N").strip()
    if not re.fullmatch(r"\d+\.\d{3}", start_device_time):
        raise RuntimeError("Cannot establish a safe log start timestamp.")
    report = Path(tempfile.mkdtemp(prefix="phone-monitor-", dir=ANDROID / ".local"))
    (report / "session.json").write_text(json.dumps({
        "host_pid": os.getpid(), "package": PACKAGE, "uid": app_uid,
        "minutes": args.minutes, "started_unix": time.time(), "device_log_start": start_device_time,
        "scope": "MiRelay UID logs and main-process memory; no phone settings/data changes",
    }, indent=2))
    print(f"REPORT={report}", flush=True)

    stop = threading.Event()
    for number in (signal.SIGINT, signal.SIGTERM):
        signal.signal(number, lambda *_: stop.set())
    state = {"reason": "duration elapsed", "bytes": 0, "fatal_log_markers": 0}
    error_file = (report / "logcat-errors.txt").open("w")
    logs = subprocess.Popen([str(ADB), "-s", args.serial, "logcat", "--uid=" + app_uid,
        "-b", "main,system,crash", "-v", "threadtime", "-T", start_device_time, "*:I"],
        stdout=subprocess.PIPE, stderr=error_file, text=True, errors="replace", bufsize=1)

    def collect():
        try:
            with (report / "app.log").open("w", buffering=1) as destination:
                while not stop.is_set():
                    line = logs.stdout.readline(16384)
                    if not line:
                        break
                    state["bytes"] += len(line.encode("utf-8"))
                    if state["bytes"] > MAX_LOG_BYTES:
                        state["reason"] = "20 MiB log limit reached"; stop.set(); break
                    destination.write(line)
                    if "FATAL EXCEPTION" in line or "Fatal signal" in line or "JNI DETECTED ERROR" in line:
                        state["fatal_log_markers"] += 1
                        print("ALERT: app fatal marker recorded; inspect private app.log.", flush=True)
        except Exception:
            state["reason"] = "local log recording failed"; stop.set()

    reader = threading.Thread(target=collect, name="mirelay-log-reader", daemon=True)
    reader.start()
    print("RECORDING: MiRelay-only logs; sampling main-process memory every 15 seconds.", flush=True)
    deadline = time.monotonic() + args.minutes * 60
    try:
        with (report / "samples.jsonl").open("w", buffering=1) as samples:
            while not stop.is_set() and time.monotonic() < deadline:
                if logs.poll() is not None:
                    state["reason"] = "logcat exited / USB disconnected"; break
                if uid() != app_uid:
                    state["reason"] = "app UID changed; stopped to avoid following another app"; break
                pid_text = adb("shell", "pidof", PACKAGE, check=False).strip()
                sample = {"at_unix": time.time(), "pids": pid_text.split() if re.fullmatch(r"\d+(?:\s+\d+)*", pid_text) else []}
                if sample["pids"]:
                    # The package filter is important: never dump other apps' memory.
                    memory = adb("shell", "dumpsys", "meminfo", PACKAGE)
                    (report / "meminfo-latest.txt").write_text(memory)
                    for label in ("PSS", "RSS"):
                        match = re.search(r"TOTAL " + label + r":\s+(\d+)", memory)
                        if match:
                            sample["total_" + label.lower() + "_kb"] = int(match.group(1))
                samples.write(json.dumps(sample) + "\n")
                print("SAMPLE " + json.dumps(sample), flush=True)
                stop.wait(min(15, max(0, deadline - time.monotonic())))
        if stop.is_set() and state["reason"] == "duration elapsed":
            state["reason"] = "stopped by signal"
    except (RuntimeError, subprocess.TimeoutExpired):
        state["reason"] = "device disconnected, unauthorized or package unavailable"
    finally:
        stop.set()
        if logs.poll() is None:
            logs.terminate()
            try:
                logs.wait(timeout=3)
            except subprocess.TimeoutExpired:
                logs.kill(); logs.wait(timeout=3)
        reader.join(timeout=3)
        error_file.close()
        state["finished_unix"] = time.time()
        (report / "summary.json").write_text(json.dumps(state, indent=2))
        print("STOPPED " + json.dumps(state), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

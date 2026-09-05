#!/usr/bin/env python3
"""Actual Android/ART tests on the repository-owned emulator, never a USB device.

Only synthetic fixtures and throwaway local servers. Reports survive failures.
Requires emulator.sh setup/start and build.sh assembleDebug/assembleDebugAndroidTest.
"""
import argparse
import hashlib
import http.client
import http.server
import json
import os
from pathlib import Path
import re
import socket
import sqlite3
import ssl
import subprocess
import tempfile
import threading
import time
import traceback

ROOT = Path(__file__).resolve().parents[2]
ANDROID = ROOT / "android"
ADB = ANDROID / ".local/sdk/platform-tools/adb"
SERIAL = "emulator-5580"
PACKAGE = "io.mirelay.android"
TOKEN = "isolated-emulator-test-token"
AVD = "MiRelay_API36_QA"
events = []
slow_paths = set()
flaky_remaining = 1
# Synthetic credentials only, retained in memory for the isolated paired receiver.
paired_receivers = {}
directory_records = {}
report_root = None


def command(*args, timeout=60, check=True, binary=False):
    result = subprocess.run([str(a) for a in args], capture_output=True, text=not binary, timeout=timeout)
    if check and result.returncode:
        raise RuntimeError(f"Command failed: {args}\n{result.stdout}\n{result.stderr}")
    return result.stdout


def adb(*args, **kwargs):
    return command(ADB, "-s", SERIAL, *args, **kwargs)


class Proxy(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def handle_request(self):
        global flaky_remaining
        path = self.path
        if path == "/__environment":
            events.append({"method": self.command, "path": path, "status": 200})
            body = b"mirelay-isolated-environment-ok"
            self.send_response(200); self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)
            return
        prefix = next((p for p in ("/slow", "/flaky") if path.startswith(p + "/")), "")
        target = path[len(prefix):]
        size = int(self.headers.get("Content-Length", "0"))
        if size > 16 * 1024 * 1024:
            self.send_error(413); return
        payload = self.rfile.read(size)
        directory_match = re.fullmatch(r"/__directory/([0-9a-f-]{36})/(prepare|receive|edit)", path)
        if directory_match:
            identity, action = directory_match.groups()
            credential = paired_receivers.get(identity)
            if self.command != "POST" or not credential or self.headers.get("Authorization") != "Bearer " + credential:
                self.send_error(403); return
            try:
                import base64
                import uuid
                assert str(uuid.UUID(identity)) == identity and report_root is not None
                data = report_root / ("directory-" + identity)
                destination, state = data / "destination", data / "state"
                destination.mkdir(parents=True, exist_ok=True)
                if action in ("prepare", "edit"):
                    body = json.loads(payload)
                    assert len(payload) <= 1024 * 1024
                    if action == "prepare":
                        assert not (state / "receiver.json").exists(), "Test directory is already initialized"
                    for item in body.get("files", []):
                        parts = item["path"].split("/")
                        assert len(parts) <= 17 and all(p not in ("", ".", "..") and not p.lower().startswith(".mirelay") and "\\" not in p and ":" not in p for p in parts)
                        file = destination.joinpath(*parts)
                        assert not file.is_symlink()
                        file.parent.mkdir(parents=True, exist_ok=True)
                        file.write_bytes(base64.b64decode(item["data"], validate=True))
                if action != "edit":
                    output = subprocess.run([str(ROOT / "target/debug/mirelay-directory"),
                        "--directory", str(destination), "--state-dir", str(state),
                        "--server-url", f"http://127.0.0.1:18081/f/{identity}", "--allow-insecure-http", "receive"],
                        env={**os.environ, "MIRELAY_TOKEN": credential}, capture_output=True, text=True, timeout=60)
                    (data / "receive.txt").write_text(output.stdout + output.stderr)
                    assert output.returncode == 0, "Isolated directory receiver failed"
                ledger = json.loads((state / "receiver.json").read_text())["files"]
                directory_records[identity] = ledger
                files = []
                for file in destination.rglob("*"):
                    relative = file.relative_to(destination).as_posix()
                    if file.is_file() and not any(part.startswith(".mirelay") for part in relative.split("/")):
                        files.append({"path": relative, "sha256": hashlib.sha256(file.read_bytes()).hexdigest(), "size": file.stat().st_size})
                receipts = json.loads(json.dumps(ledger))
                for value in receipts.values():
                    if value.get("history"):
                        value["history_sha256"] = hashlib.sha256((destination / value["history"]).read_bytes()).hexdigest()
                response = json.dumps({"files": files, "receipts": receipts}).encode()
                self.send_response(200); self.send_header("Content-Type", "application/json"); self.send_header("Content-Length", str(len(response))); self.end_headers(); self.wfile.write(response)
                events.append({"method": self.command, "path": path, "status": 200})
            except Exception:
                (report_root / "directory-harness-error.txt").write_text(traceback.format_exc())
                self.send_error(500)
            return
        event = {"method": self.command, "path": path, "bytes": size, "at": time.monotonic()}
        if prefix == "/flaky" and self.command == "OPTIONS" and flaky_remaining:
            flaky_remaining -= 1
            event["status"] = 503; events.append(event)
            self.send_response(503); self.send_header("Content-Length", "0"); self.end_headers(); return
        if self.command == "PATCH" and (prefix == "/slow" or target in slow_paths):
            time.sleep(0.8)
        connection = http.client.HTTPConnection("127.0.0.1", 18081, timeout=15)
        try:
            connection.request(self.command, target, body=payload, headers={k: v for k, v in self.headers.items() if k.lower() not in ("host", "connection")})
            response = connection.getresponse()
            body = response.read()
            if self.command == "POST" and target == "/api/v1/folders" and response.status == 200:
                import uuid
                created = json.loads(body)
                identity = created["folder_id"]
                assert str(uuid.UUID(identity)) == identity
                paired_receivers[identity] = created["receiver_token"]
            event["status"] = response.status; events.append(event)
            self.send_response(response.status)
            for key, value in response.getheaders():
                if key.lower() in ("connection", "transfer-encoding", "content-length"):
                    continue
                if key.lower() == "location" and prefix == "/slow":
                    slow_paths.add(value)
                self.send_header(key, value)
            self.send_header("Content-Length", str(len(body))); self.end_headers()
            if self.command != "HEAD":
                self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass  # Expected when the host kills an uploading app process.
        finally:
            connection.close()

    do_OPTIONS = do_POST = do_PATCH = do_HEAD = do_GET = do_DELETE = do_PUT = handle_request


def instrument(name, report):
    print(f"Running {name}", flush=True)
    output = adb("shell", "am", "instrument", "-w", "-r", "-e", "isolatedAvd", AVD,
                 "-e", "class", "io.mirelay.android." + name,
                 PACKAGE + ".test/androidx.test.runner.AndroidJUnitRunner", timeout=360, check=False)
    (report / (name.replace("#", "-") + ".txt")).write_text(output)
    passed = "OK (" in output and "FAILURES!!!" not in output and "INSTRUMENTATION_FAILED" not in output
    print(output[-4500:], flush=True)
    return passed


def transfers(report):
    data = adb("exec-out", "run-as", PACKAGE, "cat", "databases/relay.db", binary=True)
    snapshot = report / "device-db.sqlite"
    snapshot.write_bytes(data)
    with sqlite3.connect(snapshot) as db:
        db.row_factory = sqlite3.Row
        return [dict(r) for r in db.execute("SELECT id, name, status, uploaded, size, delivery_id FROM transfers")]


def wait_for(predicate, timeout=60):
    end = time.monotonic() + timeout
    while not predicate():
        if time.monotonic() > end:
            raise TimeoutError("Test condition timed out")
        time.sleep(0.25)


def observe(report):
    """Bounded debug/emulator samples, not device-independent performance budgets."""
    starts = []
    for index in range(5):
        adb("shell", "am", "force-stop", PACKAGE)
        output = adb("shell", "am", "start", "-W", "-n", PACKAGE + "/.MainActivity")
        (report / f"start-{index + 1}.txt").write_text(output)
        assert "Status: ok" in output, output
        starts.append(int(re.search(r"TotalTime: (\d+)", output).group(1)))
        time.sleep(1)
    time.sleep(3)
    pid = adb("shell", "pidof", "-s", PACKAGE).strip()
    def ticks():
        fields = adb("shell", "run-as", PACKAGE, "cat", f"/proc/{pid}/stat").split(") ", 1)[1].split()
        return int(fields[11]) + int(fields[12])
    before = ticks(); started = time.monotonic()
    (report / "mem-before.txt").write_text(adb("shell", "dumpsys", "meminfo", PACKAGE))
    time.sleep(10)
    elapsed = time.monotonic() - started; after = ticks()
    hz = int(adb("shell", "getconf", "CLK_TCK").strip())
    (report / "mem-after.txt").write_text(adb("shell", "dumpsys", "meminfo", PACKAGE))
    summary = {"process_cold_start_ms": starts, "idle_seconds": elapsed,
               "idle_cpu_seconds": (after - before) / hz, "scope": "debug APK, software-rendered x86_64 emulator; warm filesystem caches"}
    (report / "performance.json").write_text(json.dumps(summary, indent=2))
    original_night = adb("shell", "cmd", "uimode", "night").strip().split(": ")[-1]
    original_rotation = adb("shell", "wm", "user-rotation").strip().split()
    original_font = adb("shell", "settings", "get", "system", "font_scale").strip()
    try:
        adb("shell", "cmd", "uimode", "night", "yes"); time.sleep(2)
        (report / "dark.png").write_bytes(adb("exec-out", "screencap", "-p", binary=True))
        adb("shell", "wm", "user-rotation", "lock", "1"); time.sleep(2)
        (report / "landscape.png").write_bytes(adb("exec-out", "screencap", "-p", binary=True))
        adb("shell", "wm", "user-rotation", "lock", "0")
        adb("shell", "settings", "put", "system", "font_scale", "1.3"); time.sleep(2)
        (report / "large-font.png").write_bytes(adb("exec-out", "screencap", "-p", binary=True))
    finally:
        adb("shell", "cmd", "uimode", "night", original_night)
        adb("shell", "wm", "user-rotation", *original_rotation)
        if original_font == "null":
            adb("shell", "settings", "delete", "system", "font_scale")
        else:
            adb("shell", "settings", "put", "system", "font_scale", original_font)
    (report / "lastanr.txt").write_text(adb("shell", "dumpsys", "activity", "lastanr"))
    print(json.dumps(summary, indent=2), flush=True)


def main():
    global report_root
    parser = argparse.ArgumentParser()
    parser.add_argument("--classes", default="DeviceDirectoryTest,DeviceRuntimeTest,DeviceAutoTest,DeviceUiTest,NotificationDeniedTest")
    parser.add_argument("--skip-recovery", action="store_true")
    parser.add_argument("--observe", action="store_true", help="sample startup/idle and capture dark/landscape/large-font UI; no server or data reset")
    args = parser.parse_args()
    assert adb("emu", "avd", "name").splitlines()[0] == AVD, "Refusing unrelated device"
    assert adb("shell", "getprop", "sys.boot_completed").strip() == "1", "Emulator not booted"
    report = Path(tempfile.mkdtemp(prefix="device-qa-", dir=ANDROID / ".local"))
    report_root = report
    print(f"REPORT={report}", flush=True)
    if args.observe:
        observe(report)
        return 0
    results = {}
    results["apk_sha256"] = hashlib.sha256((ANDROID / "app/build/outputs/apk/debug/app-debug.apk").read_bytes()).hexdigest()
    # Verify all test loopback ports are free before starting any child process.
    for port in (18080, 18081, 18082):
        with socket.socket() as probe:
            probe.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            probe.bind(("127.0.0.1", port))
    env = dict(os.environ, MIRELAY_SERVER_TOKEN=TOKEN, MIRELAY_TOKEN=TOKEN,
               MIRELAY_ADMIN_TOKEN="isolated-emulator-admin-credential-000001")
    server_log = (report / "server.log").open("w")
    server = subprocess.Popen([str(ROOT / "target/debug/mirelay-server"), "--data-dir", str(report / "server"),
                               "serve", "--listen", "127.0.0.1:18081"], env=env, stdout=server_log, stderr=subprocess.STDOUT)
    proxy = http.server.ThreadingHTTPServer(("127.0.0.1", 18080), Proxy)
    threading.Thread(target=proxy.serve_forever, daemon=True).start()
    command("openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", report / "tls-key.pem",
            "-out", report / "tls-cert.pem", "-days", "1", "-subj", "/CN=MiRelay emulator test",
            "-addext", "subjectAltName=IP:10.0.2.2")
    tls = http.server.ThreadingHTTPServer(("127.0.0.1", 18082), Proxy)
    tls_context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    tls_context.load_cert_chain(report / "tls-cert.pem", report / "tls-key.pem")
    tls.socket = tls_context.wrap_socket(tls.socket, server_side=True)
    threading.Thread(target=tls.serve_forever, daemon=True).start()
    try:
        time.sleep(0.5)
        assert server.poll() is None, "Test server failed to start"
        process = subprocess.Popen([str(ADB), "-s", SERIAL, "shell", "-T", "toybox", "nc", "-w", "3", "10.0.2.2", "18080"],
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        process.stdin.write(b"GET /__environment HTTP/1.0\r\n\r\n"); process.stdin.flush()
        time.sleep(1)  # toybox nc exits on stdin EOF; allow the response before closing.
        stdout, stderr = process.communicate(timeout=10)
        probe = subprocess.CompletedProcess(process.args, process.returncode, stdout.decode(), stderr.decode())
        assert "mirelay-isolated-environment-ok" in probe.stdout, f"Network probe failed: {probe.returncode}: {probe.stdout!r} {probe.stderr!r}"
        results["environment_loopback"] = True
        (report / "environment.txt").write_text(command("bash", ANDROID / "scripts/emulator.sh", "check") + probe.stdout)
        adb("shell", "input", "keyevent", "82")
        adb("shell", "wm", "size", "720x1280"); adb("shell", "wm", "density", "320")
        adb("install", "-r", ANDROID / "app/build/outputs/apk/debug/app-debug.apk", timeout=90)
        adb("install", "-r", ANDROID / "app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk", timeout=90)
        # The guard above proves this is our disposable AVD. Never clear a real phone.
        adb("shell", "am", "force-stop", PACKAGE)
        adb("shell", "pm", "clear", PACKAGE)
        adb("shell", "logcat", "-c")
        adb("shell", "am", "start", "-W", "-n", PACKAGE + "/.MainActivity")
        time.sleep(2)
        (report / "empty-screen.png").write_bytes(adb("exec-out", "screencap", "-p", binary=True))
        print("Environment passed: boot, ADB, 720p display, screenshot, local HTTP, APK installation.", flush=True)
        for name in args.classes.split(","):
            if name == "NotificationDeniedTest":
                adb("shell", "am", "force-stop", PACKAGE)
                adb("shell", "pm", "revoke", PACKAGE, "android.permission.POST_NOTIFICATIONS", check=False)
            results[name] = instrument(name, report)
        for automatic in (() if args.skip_recovery else (False, True)):
            prefix = "auto-" if automatic else ""
            seed = "seedInterruptedAutomaticUpload" if automatic else "seedInterruptedUpload"
            verify = "verifyRecoveredAutomaticUpload" if automatic else "verifyRecoveredUpload"
            results[prefix + "recovery_seed"] = instrument("RecoverySeedTest#" + seed, report)
            if results[prefix + "recovery_seed"]:
                start = len(events)
                adb("shell", "am", "start", "-W", "-n", PACKAGE + "/.MainActivity")
                wait_for(lambda: any(e["method"] == "PATCH" and e.get("status") == 204 for e in events[start:]))
                before = transfers(report)
                assert before[0]["status"] == "UPLOADING", before
                pid = adb("shell", "pidof", "-s", PACKAGE).strip()
                assert pid.isdigit()
                adb("shell", "run-as", PACKAGE, "kill", "-9", pid)
                time.sleep(1)
                adb("shell", "am", "start", "-W", "-n", PACKAGE + "/.MainActivity")
                wait_for(lambda: transfers(report)[0]["status"] in ("UPLOADED", "FAILED"), timeout=90)
                results[prefix + "process_sigkill_resume"] = instrument("RecoverySeedTest#" + verify, report)
                (report / (prefix + "recovery-before.json")).write_text(json.dumps(before, indent=2))
                (report / (prefix + "recovery-after.json")).write_text(json.dumps(transfers(report), indent=2))
        adb("shell", "am", "start", "-W", "-n", PACKAGE + "/.MainActivity")
        time.sleep(2)
        (report / "final-screen.png").write_bytes(adb("exec-out", "screencap", "-p", binary=True))
        (report / "app-meminfo.txt").write_text(adb("shell", "dumpsys", "meminfo", PACKAGE))
        (report / "app-gfxinfo.txt").write_text(adb("shell", "dumpsys", "gfxinfo", PACKAGE))
        pending = command(ROOT / "target/debug/mirelay-server", "--data-dir", report / "server", "status")
        (report / "server-before.txt").write_text(pending)
        command(ROOT / "target/debug/mirelay", "--config", report / "linux.toml", "init", "--data-dir", report / "linux",
                "--server-url", "http://127.0.0.1:18081", "--allow-insecure-http")
        received = subprocess.run([str(ROOT / "target/debug/mirelay"), "--config", str(report / "linux.toml"), "sync"], env=env, capture_output=True, text=True, timeout=60)
        (report / "linux-sync.txt").write_text(received.stdout + received.stderr)
        assert received.returncode == 0, received.stdout + received.stderr
        state_path = report / "linux/state.json"
        state = json.loads(state_path.read_text()) if state_path.exists() else {"deliveries": {}}
        records = list(state["deliveries"].values())
        for record in records:
            payload = Path(record["stored_path"]).read_bytes()
            assert payload == bytes(i % 251 for i in range(len(payload))), record["original_name"]
            assert hashlib.sha256(payload).hexdigest() == record["sha256"]
        results["linux_exact_bytes_and_hash"] = True
        results["linux_delivery_count"] = len(records)
        final_status = command(ROOT / "target/debug/mirelay-server", "--data-dir", report / "server", "status")
        (report / "server-after.txt").write_text(final_status)
        assert re.search(r"pending:\s+0\b", final_status), final_status
        assert int(re.search(r"acknowledged:\s+(\d+)", final_status).group(1)) == len(records)
        results["linux_all_acknowledged"] = True
        paired_count = 0
        for identity, credential in paired_receivers.items():
            data = report / ("paired-" + identity)
            config = report / ("paired-" + identity + ".toml")
            command(ROOT / "target/debug/mirelay", "--config", config, "init", "--data-dir", data,
                    "--server-url", f"http://127.0.0.1:18081/f/{identity}", "--allow-insecure-http")
            output = subprocess.run([str(ROOT / "target/debug/mirelay"), "--config", str(config), "sync"],
                                    env={**env, "MIRELAY_TOKEN": credential}, capture_output=True, text=True, timeout=60)
            (report / ("paired-" + identity + "-sync.txt")).write_text(output.stdout + output.stderr)
            assert output.returncode == 0, "Paired Linux receiver failed"
            saved = data / "state.json"
            paired_records = list(json.loads(saved.read_text())["deliveries"].values()) if saved.exists() else []
            for record in paired_records:
                payload = Path(record["stored_path"]).read_bytes()
                assert payload == bytes(i % 251 for i in range(len(payload)))
                assert hashlib.sha256(payload).hexdigest() == record["sha256"]
                assert record["delivery_status"] == "acknowledged"
            paired_count += len(paired_records)
            status = command(ROOT / "target/debug/mirelay-server", "--data-dir", report / "server",
                             "--device-id", "folder_" + identity, "status")
            assert re.search(r"pending:\s+0\b", status), "Paired Folder still has pending content"
        directory_count = sum(len(records) for records in directory_records.values())
        assert records or paired_count or directory_count, "No actual Linux deliveries"
        if paired_receivers:
            assert paired_count + directory_count >= 1, "Pairing was created but no paired Android upload reached Linux"
        results["paired_linux_delivery_count"] = paired_count
        results["paired_linux_exact_bytes_hash_ack"] = True if paired_count else None
        results["directory_linux_path_count"] = directory_count
        results["directory_receipts"] = directory_records
        print(f"Linux verified {len(records)} legacy, {paired_count} paired delivery-only files and {directory_count} synchronized paths.", flush=True)
    except Exception as error:
        results["harness_error"] = traceback.format_exc()
        print(f"ERROR: {error}", flush=True)
    finally:
        (report / "logcat.txt").write_text(adb("logcat", "-d", "-v", "threadtime", check=False))
        # Original screen pixels from device assertions; no image transformations.
        for name in adb("shell", "run-as", PACKAGE, "ls", "files/visual-qa", check=False).splitlines():
            if re.fullmatch(r"[a-z0-9-]+\.png", name):
                visual = report / "visual"
                visual.mkdir(exist_ok=True)
                (visual / name).write_bytes(adb("exec-out", "run-as", PACKAGE, "cat", "files/visual-qa/" + name, binary=True))
        (report / "http-events.json").write_text(json.dumps(events, indent=2))
        (report / "results.json").write_text(json.dumps(results, indent=2))
        proxy.shutdown(); proxy.server_close()
        tls.shutdown(); tls.server_close()
        server.terminate()
        try:
            server.wait(timeout=10)
        except subprocess.TimeoutExpired:
            server.kill(); server.wait()
        server_log.close()
        print(json.dumps(results, indent=2), flush=True)
        print(f"REPORT={report}", flush=True)
    return 1 if "harness_error" in results or any(v is False for v in results.values()) else 0


if __name__ == "__main__":
    raise SystemExit(main())

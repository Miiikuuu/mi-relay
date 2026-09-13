#!/usr/bin/env python3
"""Exercise upgrade --apply using real binaries in disposable Linux namespaces.

Requires bubblewrap. Only systemctl is replaced with a child-process controller;
file permissions, backup, fsync, atomic replacement, HTTP and SQLite are real.
This is not a systemd/fresh-install acceptance test. No production data is copied.
"""
import argparse
import base64
from contextlib import closing
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import secrets
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
import uuid
from unittest.mock import patch


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def database_snapshot(path):
    """Read a consistent snapshot; never print rows or credential hashes."""
    with closing(sqlite3.connect(f"{path.as_uri()}?mode=ro", uri=True)) as db:
        db.execute("BEGIN")
        if db.execute("PRAGMA integrity_check").fetchall() != [("ok",)]:
            raise RuntimeError("Database integrity check failed")
        if db.execute("PRAGMA foreign_key_check").fetchall():
            raise RuntimeError("Database foreign-key check failed")
        tables = {}
        columns = {}
        for (name,) in db.execute("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"):
            quoted = '"' + name.replace('"', '""') + '"'
            cursor = db.execute(f"SELECT * FROM {quoted} ORDER BY rowid")
            columns[name] = [field[0] for field in cursor.description]
            tables[name] = cursor.fetchall()
        return {"schema": db.execute("PRAGMA user_version").fetchone()[0], "tables": tables,
                "columns": columns}


def require_retained(before, after, expected_schema):
    if after["schema"] != expected_schema:
        raise RuntimeError("Unexpected migrated database schema")
    for name, rows in before["tables"].items():
        old_columns = before["columns"][name]
        new_columns = after["columns"].get(name, [])
        allowed = ["disconnected"] if name == "folders" and before["schema"] < 4 <= expected_schema else []
        if new_columns != old_columns + allowed:
            raise RuntimeError("Unexpected migrated columns")
        current = after["tables"][name]
        if allowed and any(row[-1] != 0 for row in current):
            raise RuntimeError("Existing Folder was implicitly disconnected")
        if [row[:len(old_columns)] for row in current] != rows:
            raise RuntimeError("Pre-upgrade database records changed")


def stage_artifacts(directory, old_binary, new_binary):
    directory.mkdir()
    for name, source in (("old-server", old_binary), ("new-server", new_binary)):
        expected = digest(source)
        destination = directory / name
        shutil.copy2(source, destination)
        if digest(destination) != expected:
            raise RuntimeError("Release artifact changed while being staged")


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--old-binary", required=True)
    parser.add_argument("--new-binary", required=True)
    parser.add_argument("--old-schema", type=int, choices=(1, 2, 3), default=3)
    parser.add_argument("--new-schema", type=int, choices=(2, 3, 4), default=4)
    parser.add_argument("--marker", help=argparse.SUPPRESS)
    args = parser.parse_args(argv)
    if args.new_schema <= args.old_schema:
        parser.error("Expected an increasing schema version; downgrade is not an upgrade rehearsal.")
    return args


def inside(args):
    if os.geteuid() != 0 or Path("/etc/mirelay/rehearsal-marker").read_text() != args.marker:
        raise RuntimeError("Refusing to run outside the disposable namespace.")
    spec = importlib.util.spec_from_file_location("deploy_setup", Path(__file__).with_name("setup.py"))
    setup = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(setup)
    setup.BACKEND.parent.mkdir(parents=True)
    setup.STATE.mkdir(parents=True)
    shutil.copy2(args.old_binary, setup.BACKEND)
    legacy = secrets.token_hex(32)
    existing_admin = secrets.token_hex(32) if args.old_schema >= 2 else None
    initial_env = f"MIRELAY_SERVER_TOKEN={legacy}\n"
    if existing_admin:
        initial_env += f"MIRELAY_ADMIN_TOKEN={existing_admin}\n"
    setup.atomic_file(setup.ENV, initial_env.encode())
    original_env = setup.ENV.read_bytes()
    process = None
    controller_calls = []
    run_command = setup.run

    def environment(path=setup.ENV):
        values = dict(line.split("=", 1) for line in path.read_text().splitlines())
        return {**os.environ, **values, "HTTP_PROXY": "", "HTTPS_PROXY": "", "ALL_PROXY": ""}

    def control(command, **kwargs):
        nonlocal process
        if command[0] != "systemctl":
            return run_command(command, **kwargs)
        if command not in (["systemctl", "stop", "mirelay-server.service"],
                           ["systemctl", "start", "mirelay-server.service"]):
            raise AssertionError("Unexpected service operation")
        controller_calls.append(command[1])
        if command[1] == "stop":
            if process and process.poll() is None:
                process.terminate()
                process.wait(timeout=10)
        else:
            if process and process.poll() is None:
                raise AssertionError("Start before stopping the previous process")
            process = subprocess.Popen([str(setup.BACKEND), "--data-dir", str(setup.STATE),
                "serve", "--listen", "127.0.0.1:8080"], env=environment(),
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        return ""

    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def request(method, path, *, token=legacy, body=None, headers=None, expected=200, port=8080):
        fields = {"Mirelay-Protocol-Version": "1", "Authorization": f"Bearer {token}"}
        fields.update(headers or {})
        if isinstance(body, dict):
            fields["Content-Type"] = "application/json"
            body = json.dumps(body).encode()
        req = urllib.request.Request(f"http://127.0.0.1:{port}" + path, data=body, headers=fields, method=method)
        try:
            response = opener.open(req, timeout=5)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            assert response.status == expected, f"HTTP {response.status}, expected {expected}"
            return response.headers, response.read(1024 * 1024)

    payload = bytes(range(256)) * 1024
    sha = hashlib.sha256(payload).hexdigest()
    metadata = ",".join(key + " " + base64.b64encode(value.encode()).decode() for key, value in {
        "filename": "rehearsal.bin", "media_type": "application/octet-stream", "sha256": sha}.items())
    tus = {"Tus-Resumable": "1.0.0"}
    db_path = setup.STATE / "mirelay-server.sqlite3"

    def version(path=db_path):
        with closing(sqlite3.connect(f"file:{path}?mode=ro", uri=True)) as db:
            return db.execute("PRAGMA user_version").fetchone()[0]

    def deliveries(path=db_path):
        with closing(sqlite3.connect(f"file:{path}?mode=ro", uri=True)) as db:
            return db.execute("SELECT * FROM deliveries ORDER BY sequence").fetchall()

    command = ["upgrade", "--server-binary", args.new_binary, "--server-sha256", digest(Path(args.new_binary)), "--apply"]

    def create_folder(admin, name, *, ready=True):
        _, body = request("POST", "/api/v1/folders", token=admin, body={"name": name})
        folder = json.loads(body)
        scoped = "/f/" + folder["folder_id"]
        receiver = folder["receiver_token"]
        sender = secrets.token_hex(32)
        request("GET", scoped + "/api/v1/deliveries", token=receiver, expected=409)
        if ready:
            _, body = request("POST", "/api/v1/pairings/claim", body={"pairing_code": folder["pairing_code"], "sender_token": sender})
            verification = json.loads(body)["verification"]
            request("POST", scoped + "/api/v1/pairing/confirm", token=receiver, body={"verification": verification})
        return scoped, receiver, sender

    with patch.object(setup, "run", side_effect=control):
        try:
            control(["systemctl", "start", "mirelay-server.service"])
            setup.local_health()
            assert version() == args.old_schema, "Old binary does not match the requested source schema"
            uploads = []
            for length in (len(payload), 65536):
                headers, _ = request("POST", "/api/v1/uploads", body=b"", expected=201,
                    headers={**tus, "Upload-Length": str(len(payload)), "Upload-Metadata": metadata})
                path = urllib.parse.urljoin("/api/v1/uploads", headers["Location"])
                request("PATCH", path, body=payload[:length], expected=204,
                    headers={**tus, "Upload-Offset": "0", "Content-Type": "application/offset+octet-stream"})
                uploads.append(path)
            old_pair = None
            pending_pair = None
            if existing_admin:
                old_pair = create_folder(existing_admin, "Existing ready Folder")
                pending_pair = create_folder(existing_admin, "Existing unclaimed Folder", ready=False)
                headers, _ = request("POST", old_pair[0] + "/api/v1/uploads", token=old_pair[2], body=b"", expected=201,
                    headers={**tus, "Upload-Length": str(len(payload)), "Upload-Metadata": metadata})
                paired_upload = urllib.parse.urljoin(old_pair[0] + "/api/v1/uploads", headers["Location"])
                request("PATCH", paired_upload, token=old_pair[2], body=payload[:65536], expected=204,
                    headers={**tus, "Upload-Offset": "0", "Content-Type": "application/offset+octet-stream"})
            before_rows = deliveries()
            before_database = database_snapshot(db_path)
            assert len(before_rows) == 1
            before_calls = list(controller_calls)
            with patch.object(setup, "verified_file", side_effect=ValueError("bad checksum")):
                try:
                    setup.main(command)
                    raise AssertionError("Expected hash failure")
                except ValueError:
                    pass
            assert controller_calls == before_calls
            print("PASS: bad checksum leaves the running service untouched", flush=True)

            for operation in ("copytree", "sync_backup"):
                target = setup.shutil if operation == "copytree" else setup
                with patch.object(target, operation, side_effect=OSError("injected backup failure")):
                    try:
                        setup.main(command)
                        raise AssertionError("Expected backup failure")
                    except OSError:
                        pass
                setup.local_health()
                assert digest(setup.BACKEND) == digest(Path(args.old_binary))
                assert setup.ENV.read_bytes() == original_env and version() == args.old_schema
                assert database_snapshot(db_path) == before_database
                headers, _ = request("HEAD", uploads[1], headers=tus)
                assert headers["Upload-Offset"] == "65536"
                print(f"PASS: {operation} failure restarts the unchanged old backend", flush=True)

            before_backups = set(Path("/var/backups/mirelay").iterdir())
            setup.main(command)
            backup, = set(Path("/var/backups/mirelay").iterdir()) - before_backups
            require_retained(before_database, database_snapshot(db_path), args.new_schema)
            assert database_snapshot(backup / "data/mirelay-server.sqlite3") == before_database
            assert (backup / "server.env").read_bytes() == original_env
            assert (backup / "old-server").read_bytes() == Path(args.old_binary).read_bytes()
            credentials = environment()
            assert credentials["MIRELAY_SERVER_TOKEN"] == legacy
            admin = credentials["MIRELAY_ADMIN_TOKEN"]
            assert len(admin) == 64 and admin != legacy
            if existing_admin:
                assert admin == existing_admin and setup.ENV.read_bytes() == original_env
            if args.new_schema >= 3:
                migrated = database_snapshot(db_path)
                assert migrated["tables"]["directory_versions"] == []
                assert migrated["tables"]["directory_indexes"] == []
            assert setup.ENV.stat().st_mode & 0o777 == 0o600
            print(f"PASS: durable private backup, schema {args.old_schema} -> {args.new_schema}, all existing rows/credentials retained", flush=True)

            headers, _ = request("HEAD", uploads[1], headers=tus)
            assert headers["Upload-Offset"] == "65536"
            request("PATCH", uploads[1], body=payload[65536:], expected=204,
                headers={**tus, "Upload-Offset": "65536", "Content-Type": "application/offset+octet-stream"})
            _, body = request("GET", "/api/v1/deliveries?status=pending&limit=50")
            items = json.loads(body)["items"]
            assert len(items) == 2
            for item in items:
                _, received = request("GET", "/api/v1/deliveries/" + item["delivery_id"] + "/content",
                    headers={"If-Match": '"sha256:' + item["sha256"] + '"'})
                assert received == payload
            print("PASS: interrupted v1 tus upload resumes after upgrade; exact content retained", flush=True)

            if old_pair:
                request("GET", pending_pair[0] + "/api/v1/deliveries", token=pending_pair[1], expected=409)
                headers, _ = request("HEAD", paired_upload, token=old_pair[2], headers=tus)
                assert headers["Upload-Offset"] == "65536"
                request("PATCH", paired_upload, token=old_pair[2], body=payload[65536:], expected=204,
                    headers={**tus, "Upload-Offset": "65536", "Content-Type": "application/offset+octet-stream"})
                _, body = request("GET", old_pair[0] + "/api/v1/deliveries?status=pending&limit=50", token=old_pair[1])
                item, = json.loads(body)["items"]
                _, received = request("GET", old_pair[0] + "/api/v1/deliveries/" + item["delivery_id"] + "/content",
                    token=old_pair[1], headers={"If-Match": '"sha256:' + sha + '"'})
                assert received == payload
                request("GET", old_pair[0] + "/api/v1/deliveries", token=old_pair[2], expected=403)
                if args.new_schema >= 3:
                    _, body = request("GET", old_pair[0] + "/api/v1/directory", token=old_pair[1])
                    assert json.loads(body) == {"schema_version": 1, "receiver": None, "entries": []}
                print("PASS: existing ready/unclaimed pairing and scoped tus offset retained; no implicit directory initialization", flush=True)

            scoped, receiver, sender = create_folder(admin, "Disposable rehearsal")
            request("GET", scoped + "/api/v1/deliveries?status=pending&limit=1", token=receiver)
            request("GET", scoped + "/api/v1/deliveries?status=pending&limit=1", token=sender, expected=403)
            print("PASS: new pairing becomes ready only after receiver confirmation", flush=True)

            if args.new_schema >= 3:
                inventory = {"id": str(uuid.uuid4()), "entries": []}
                update = {"previous_id": None, "inventory": inventory}
                request("PUT", scoped + "/api/v1/directory/index", token=sender, body=update, expected=403)
                request("PUT", scoped + "/api/v1/directory/index", token=receiver, body=update)
                relative = "Documents/更新.bin"
                directory_metadata = ",".join(
                    key + " " + base64.b64encode(value.encode()).decode()
                    for key, value in {"filename": relative.rsplit("/", 1)[-1],
                        "media_type": "application/octet-stream", "sha256": sha,
                        "relative_path": relative, "source_version": "1"}.items())
                headers, _ = request("POST", scoped + "/api/v1/uploads", token=sender, body=b"", expected=201,
                    headers={**tus, "Upload-Length": str(len(payload)), "Upload-Metadata": directory_metadata})
                directory_upload = urllib.parse.urljoin(scoped + "/api/v1/uploads", headers["Location"])
                request("PATCH", directory_upload, token=sender, body=payload[:65536], expected=204,
                    headers={**tus, "Upload-Offset": "0", "Content-Type": "application/offset+octet-stream"})
                control(["systemctl", "stop", "mirelay-server.service"])
                control(["systemctl", "start", "mirelay-server.service"])
                setup.local_health()
                headers, _ = request("HEAD", directory_upload, token=sender, headers=tus)
                assert headers["Upload-Offset"] == "65536"
                request("PATCH", directory_upload, token=sender, body=payload[65536:], expected=204,
                    headers={**tus, "Upload-Offset": "65536", "Content-Type": "application/offset+octet-stream"})
                _, body = request("GET", scoped + "/api/v1/directory", token=sender)
                state = json.loads(body)
                item, = state["entries"]
                assert state["receiver"] == inventory
                assert item["path"] == relative and item["version"] == 1 and not item["acknowledged"]
                _, received = request("GET", scoped + "/api/v1/deliveries/" + item["delivery_id"] + "/content",
                    token=receiver, headers={"If-Match": '"sha256:' + sha + '"'})
                assert received == payload and hashlib.sha256(received).hexdigest() == sha
                _, body = request("GET", scoped + "/api/v1/deliveries?status=pending&limit=50", token=receiver)
                assert json.loads(body)["items"] == [], "Legacy receiver must not consume directory deliveries"
                ack = {"path": relative, "version": 1, "sha256": sha, "conflict": True}
                request("POST", scoped + "/api/v1/directory/ack", token=sender, body=ack, expected=403)
                request("POST", scoped + "/api/v1/directory/ack", token=receiver,
                    body={**ack, "sha256": "0" * 64}, expected=409)
                request("POST", scoped + "/api/v1/directory/ack", token=receiver, body=ack)
                request("POST", scoped + "/api/v1/directory/ack", token=receiver, body={**ack, "conflict": False})
                _, body = request("GET", scoped + "/api/v1/directory", token=sender)
                item, = json.loads(body)["entries"]
                assert item["acknowledged"] and item["conflict"]
                for legacy_item in items:
                    _, retained = request("GET", "/api/v1/deliveries/" + legacy_item["delivery_id"] + "/content",
                        headers={"If-Match": '"sha256:' + sha + '"'})
                    assert retained == payload, "Directory ACK must not delete another queue's identical content"
                print("PASS: directory inventory, Unicode path, tus restart/resume, exact bytes, legacy isolation and bound receipts", flush=True)

            if args.new_schema >= 4:
                for token in (sender, receiver):
                    _, body = request("POST", scoped + "/api/v1/pairing/disconnect", token=token, body={})
                    assert json.loads(body)["state"] == "disconnected"
                    for endpoint in ("/api/v1/directory", "/api/v1/deliveries"):
                        _, body = request("GET", scoped + endpoint, token=token, expected=410)
                        assert json.loads(body)["code"] == "folder_disconnected"
                control(["systemctl", "stop", "mirelay-server.service"])
                control(["systemctl", "start", "mirelay-server.service"])
                setup.local_health()
                _, body = request("GET", scoped + "/api/v1/handshake", token=receiver)
                assert json.loads(body)["state"] == "disconnected"
                if old_pair:
                    request("GET", old_pair[0] + "/api/v1/directory", token=old_pair[1])
                print("PASS: disconnection is idempotent, survives restart, blocks both roles and isolates other Folders", flush=True)

            # A failed post-start health check must not rewind an already-upgraded DB.
            after_env = setup.ENV.read_bytes()
            after_database = database_snapshot(db_path)
            with patch.object(setup, "local_health", side_effect=RuntimeError("injected health failure")):
                try:
                    setup.main(command)
                    raise AssertionError("Expected health failure")
                except RuntimeError:
                    pass
            setup.local_health()
            assert database_snapshot(db_path) == after_database
            assert setup.ENV.read_bytes() == after_env
            print("PASS: post-start failure does not roll back new data or rotate credentials", flush=True)

            # Recover to a DIFFERENT disposable directory, never overwrite current state.
            recovered = Path("/var/lib/recovered")
            shutil.copytree(backup / "data", recovered)
            recovery = subprocess.Popen([str(backup / "old-server"), "--data-dir", str(recovered),
                "serve", "--listen", "127.0.0.1:8081"], env=environment(backup / "server.env"),
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                import time
                for attempt in range(30):
                    try:
                        headers, _ = request("HEAD", uploads[1], headers=tus, port=8081)
                        break
                    except urllib.error.URLError:
                        time.sleep(0.1)
                else:
                    raise AssertionError("Recovered backend did not become ready")
                assert headers["Upload-Offset"] == "65536"
                assert deliveries(recovered / "mirelay-server.sqlite3") == before_rows
                assert database_snapshot(recovered / "mirelay-server.sqlite3") == before_database
                if old_pair:
                    headers, _ = request("HEAD", paired_upload, token=old_pair[2], headers=tus, port=8081)
                    assert headers["Upload-Offset"] == "65536"
            finally:
                recovery.terminate()
                recovery.wait(timeout=10)
            print(f"PASS: old binary + backup recover schema {args.old_schema}, pairing and interrupted offsets in isolation", flush=True)
        finally:
            control(["systemctl", "stop", "mirelay-server.service"])


def main():
    if not __debug__:
        raise RuntimeError("Assertions must be enabled for the rehearsal")
    args = parse_args()
    if args.marker:
        inside(args)
        return
    if os.geteuid() == 0:
        raise RuntimeError("Launch the rehearsal as an ordinary user, not host root.")
    for key in ("old_binary", "new_binary"):
        value = Path(getattr(args, key)).resolve(strict=True)
        if not value.is_file():
            raise RuntimeError("Expected an existing binary")
        setattr(args, key, str(value))
    with tempfile.TemporaryDirectory(prefix="mirelay-upgrade-rehearsal-") as directory:
        root = Path(directory)
        for name in ("etc/mirelay", "opt", "var"):
            (root / name).mkdir(parents=True)
        stage_artifacts(root / "artifacts", Path(args.old_binary), Path(args.new_binary))
        marker = secrets.token_hex(32)
        (root / "etc/mirelay/rehearsal-marker").write_text(marker)
        subprocess.run(["bwrap", "--die-with-parent", "--unshare-user", "--uid", "0", "--gid", "0",
            "--unshare-pid", "--unshare-net", "--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc",
            "--tmpfs", "/tmp", "--tmpfs", "/run", "--bind", str(root / "etc"), "/etc",
            "--bind", str(root / "opt"), "/opt", "--bind", str(root / "var"), "/var",
            "--ro-bind", str(root / "artifacts"), "/run/rehearsal-artifacts",
            sys.executable, "-B", str(Path(__file__).resolve()), "--old-binary", "/run/rehearsal-artifacts/old-server",
            "--new-binary", "/run/rehearsal-artifacts/new-server", "--old-schema", str(args.old_schema),
            "--new-schema", str(args.new_schema), "--marker", marker], check=True, timeout=180)


if __name__ == "__main__":
    main()

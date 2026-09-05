#!/usr/bin/env python3
"""Explicit, conservative systemd deployment. Planning is the default.

Only verified, caller-supplied release artifacts are executed. No curl|sh,
firewall reset, proxy edits, remote shell, or automatic database downgrade.
"""
import argparse
import hashlib
import ipaddress
import os
from pathlib import Path
import re
import secrets
import shutil
import stat
import subprocess
import sys
import tempfile
import time

TEMPLATES = Path(__file__).resolve().parent
BACKEND = Path("/opt/mirelay/bin/mirelay-server")
ENV = Path("/etc/mirelay/server.env")
STATE = Path("/var/lib/mirelay-server")


def run(args, *, timeout=20, env=None):
    try:
        result = subprocess.run(args, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                timeout=timeout, env=env, text=True)
        return result.stdout.strip()
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
        # Never dump environment files or arbitrary child output into logs.
        raise RuntimeError(f"Command failed or timed out: {args[0]}. Inspect the relevant service privately.") from error


def address(value):
    if not value or len(value) > 253:
        raise ValueError("Provide a public IPv4 address or DNS hostname, without a URL or port.")
    try:
        parsed = ipaddress.ip_address(value)
    except ValueError:
        labels = value.split(".")
        if len(labels) < 2 or all(label.isdecimal() for label in labels) or any(
            not re.fullmatch(r"[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?", label) for label in labels
        ):
            raise ValueError("Invalid DNS hostname.") from None
        return value.lower(), False
    if not isinstance(parsed, ipaddress.IPv4Address) or not parsed.is_global:
        raise ValueError("The IP installer requires a globally routable IPv4 address assigned to this host.")
    return str(parsed), True


def verified_file(filename, expected):
    path = Path(filename)
    if not path.is_absolute() or path.is_symlink() or not path.is_file():
        raise ValueError("Release binary must be an absolute, regular, non-symlink file.")
    if not re.fullmatch(r"[0-9a-fA-F]{64}", expected):
        raise ValueError("Supply the trusted release SHA-256, not an unchecked downloaded checksum.")
    with path.open("rb") as source:
        hasher = hashlib.sha256()
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            hasher.update(chunk)
        digest = hasher.hexdigest()
    if not secrets.compare_digest(digest, expected.lower()):
        raise ValueError("Release SHA-256 mismatch. Nothing was installed.")
    return path


def no_symlink(path):
    path = Path(path)
    for item in (path, *path.parents):
        if item.is_symlink():
            raise ValueError(f"Refusing a symlink in managed path: {item}")


def atomic_file(path, data, mode=0o600):
    path = Path(path)
    no_symlink(path)
    with tempfile.NamedTemporaryFile(dir=path.parent, prefix=".mirelay-", delete=False) as output:
        temporary = Path(output.name)
        os.fchmod(output.fileno(), mode)
        output.write(data)
        output.flush()
        os.fsync(output.fileno())
    try:
        os.replace(temporary, path)
        descriptor = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
    finally:
        if temporary.exists():
            temporary.unlink()


def caddy_config(host, is_ip):
    # All substitutions are validated hostnames/IPs, never shell/Caddy syntax.
    binding = f"bind {host}\n" if is_ip else ""
    profile = "profile shortlived\n" if is_ip else ""
    return ("{\n auto_https disable_redirects\n admin unix//run/mirelay-proxy/admin.sock\n"
            " persist_config off\n servers {\n protocols h1 h2\n timeouts {\n"
            " read_header 10s\n read_body 5m\n idle 2m\n }\n }\n}\n"
            f"https://{host} {{\n {binding}tls {{\n issuer acme https://acme-v02.api.letsencrypt.org/directory {{\n"
            f" {profile}disable_http_challenge\n }}\n }}\n reverse_proxy 127.0.0.1:8080\n}}\n").encode()


def sync_backup(directory):
    # Finish a durable backup before replacing any live artifact. Never follow
    # symlinks copied from the state directory.
    entries = sorted(directory.rglob("*"), key=lambda p: len(p.parts), reverse=True)
    # Persist the backup's directory entry as well as everything inside it.
    for path in [*entries, directory, directory.parent]:
        if path.is_symlink():
            continue
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        try:
            if not (stat.S_ISREG(os.fstat(descriptor).st_mode) or stat.S_ISDIR(os.fstat(descriptor).st_mode)):
                raise RuntimeError("Unexpected special file in backup.")
            os.fsync(descriptor)
        finally:
            os.close(descriptor)


def dependencies():
    if sys.platform != "linux":
        raise RuntimeError("This installer supports Linux with systemd only.")
    for command in ("systemctl", "systemd-analyze", "ss", "curl", "useradd", "getent", "ip"):
        if shutil.which(command) is None:
            raise RuntimeError(f"Missing prerequisite: {command}")


def local_health():
    for _ in range(10):
        try:
            if run(["curl", "--noproxy", "*", "--fail", "--silent", "--show-error", "--connect-timeout", "2",
                    "--max-time", "3", "http://127.0.0.1:8080/healthz"], timeout=5) == "ok":
                return
        except RuntimeError:
            pass
        time.sleep(1)
    raise RuntimeError("Backend health check failed. Existing backup/state was retained; no automatic database rollback was attempted.")


def check(host):
    dependencies()
    for service in ("mirelay-server.service", "mirelay-proxy.service"):
        run(["systemctl", "is-active", "--quiet", service])
    local_health()
    public = f"https://{host}"
    if run(["curl", "--fail", "--silent", "--show-error", "--connect-timeout", "5", "--max-time", "15", public + "/healthz"]) != "ok":
        raise RuntimeError("Unexpected public health response.")
    code = run(["curl", "--silent", "--show-error", "--output", "/dev/null", "--write-out", "%{http_code}",
                "--connect-timeout", "5", "--max-time", "15", public + "/api/v1/deliveries?status=pending&limit=1"])
    if code != "401":
        raise RuntimeError("Unauthenticated delivery request was not rejected with 401.")
    listeners = run(["ss", "-H", "-ltn", "sport = :8080"])
    if any(line.split()[3] != "127.0.0.1:8080" for line in listeners.splitlines()):
        raise RuntimeError("Backend port 8080 is not restricted to IPv4 loopback.")
    print("PASS: services, loopback backend, trusted HTTPS and authentication boundary.")
    print("Certificate renewal and long-term disk usage still need monitoring.")


def install(args, server):
    host, is_ip = address(args.address)
    caddy = verified_file(args.caddy_binary, args.caddy_sha256)
    print(f"Plan: isolated MiRelay accounts, backend on 127.0.0.1:8080, HTTPS at https://{host}.")
    print("Will create separate administrator and legacy credentials; Folders get scoped credentials through pairing.")
    print("SSH, other proxies, TCP 80, UDP 443 and firewall configuration will NOT be changed.")
    if not args.apply:
        print("Plan only. Re-run with --apply on the VPS after reviewing port 443/firewall/DNS requirements.")
        return
    dependencies()
    if os.geteuid() != 0:
        raise RuntimeError("Installation needs root; planning does not.")
    if is_ip:
        import json
        interfaces = json.loads(run(["ip", "-j", "-4", "address", "show"]))
        if not any(item.get("local") == host for interface in interfaces for item in interface.get("addr_info", [])):
            raise RuntimeError("Public IPv4 address is not assigned to this host. This IP template does not support a NAT frontend.")
    managed = [Path("/opt/mirelay"), Path("/etc/mirelay"), STATE, Path("/var/lib/mirelay-proxy"),
               Path("/etc/systemd/system/mirelay-server.service"), Path("/etc/systemd/system/mirelay-proxy.service")]
    for target in managed:
        no_symlink(target)
        if target.exists():
            raise RuntimeError(f"Existing installation or path: {target}. Use upgrade for the backend; do not overwrite it.")
    for port in (8080, 443):
        if run(["ss", "-H", "-ltn", f"sport = :{port}"]):
            raise RuntimeError(f"TCP {port} is already occupied. Nothing was installed.")
    for user in ("mirelay", "mirelay-proxy"):
        for database in ("passwd", "group"):
            result = subprocess.run(["getent", database, user], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5)
            if result.returncode == 0:
                raise RuntimeError(f"Account/group {user} already exists. Refusing to reuse an unverified identity.")
    if shutil.disk_usage("/var/lib").free < 256 * 1024 * 1024:
        raise RuntimeError("Less than 256 MiB free disk space.")
    with tempfile.TemporaryDirectory(prefix="mirelay-install-") as staging:
        stage = Path(staging)
        for source, name, digest in ((server,"mirelay-server",args.server_sha256),(caddy,"caddy",args.caddy_sha256)):
            shutil.copyfile(source, stage / name)
            verified_file(str(stage / name), digest)
            (stage / name).chmod(0o755)
        (stage / "Caddyfile").write_bytes(caddy_config(host, is_ip))
        run([str(stage / "mirelay-server"), "--version"])
        run([str(stage / "caddy"), "validate", "--config", str(stage / "Caddyfile"), "--adapter", "caddyfile"],
            env={**os.environ, "XDG_DATA_HOME": str(stage / "data"), "XDG_CONFIG_HOME": str(stage / "config")})
        for user, state in (("mirelay",STATE),("mirelay-proxy",Path("/var/lib/mirelay-proxy"))):
            run(["useradd", "--system", "--user-group", "--no-create-home", "--home-dir", str(state), "--shell", "/usr/sbin/nologin", user])
        Path("/opt/mirelay/bin").mkdir(parents=True, mode=0o755)
        ENV.parent.mkdir(mode=0o755)
        atomic_file(BACKEND, (stage / "mirelay-server").read_bytes(), 0o755)
        atomic_file(Path("/opt/mirelay/bin/caddy"), (stage / "caddy").read_bytes(), 0o755)
        atomic_file(ENV, f"MIRELAY_SERVER_TOKEN={secrets.token_hex(32)}\nMIRELAY_ADMIN_TOKEN={secrets.token_hex(32)}\n".encode())
        atomic_file(ENV.parent / "Caddyfile", (stage / "Caddyfile").read_bytes(), 0o644)
        for service in ("mirelay-server.service", "mirelay-proxy.service"):
            atomic_file(Path("/etc/systemd/system") / service, (TEMPLATES / service).read_bytes(), 0o644)
        run(["systemd-analyze", "verify", "/etc/systemd/system/mirelay-server.service", "/etc/systemd/system/mirelay-proxy.service"])
        run(["systemctl", "daemon-reload"])
        run(["systemctl", "enable", "--now", "mirelay-server.service", "mirelay-proxy.service"], timeout=45)
    local_health()
    print("Installed. TLS issuance may still be pending: allow inbound TCP 443, then run the check command.")
    print("Credentials are root-only in /etc/mirelay/server.env. Do not paste this file into logs or chat.")


def upgrade(args, server):
    print("Plan: stop only mirelay-server, back up its binary, environment and entire data directory, install the verified backend, then restart it.")
    print("Caddy, certificates, proxy services and firewall stay unchanged. Existing tokens are preserved.")
    if not args.apply:
        print("Plan only. This upgrades database schema; old binaries must NOT be started against the upgraded database.")
        return
    dependencies()
    if os.geteuid() != 0:
        raise RuntimeError("Upgrade needs root.")
    for path in (BACKEND, ENV, STATE, Path("/var/backups/mirelay")):
        no_symlink(path)
    if not BACKEND.is_file() or not ENV.is_file() or not STATE.is_dir():
        raise RuntimeError("No complete installation found; refusing an upgrade.")
    if stat.S_IMODE(ENV.stat().st_mode) != 0o600 or ENV.stat().st_uid != 0:
        raise RuntimeError("Environment file must be root-owned mode 0600.")
    size = sum(p.stat().st_size for p in STATE.rglob("*") if p.is_file() and not p.is_symlink())
    if shutil.disk_usage("/var").free < size + 256 * 1024 * 1024:
        raise RuntimeError("Insufficient space for a complete data backup and upgrade margin.")
    backup_root = Path("/var/backups/mirelay")
    backup_root.mkdir(mode=0o700, parents=True, exist_ok=True)
    backup_root.chmod(0o700)
    backup = Path(tempfile.mkdtemp(prefix="upgrade-", dir=backup_root))
    # Stage and re-hash before running anything or stopping the live backend.
    staged = backup / "new-server"
    shutil.copyfile(server, staged)
    verified_file(str(staged), args.server_sha256)
    staged.chmod(0o755)
    run([str(staged), "--version"])
    run(["systemctl", "stop", "mirelay-server.service"], timeout=55)
    try:
        shutil.copy2(BACKEND, backup / "old-server")
        shutil.copy2(ENV, backup / "server.env")
        shutil.copytree(STATE, backup / "data", symlinks=True)
        sync_backup(backup)
    except Exception:
        run(["systemctl", "start", "mirelay-server.service"])
        raise
    print(f"Private recovery backup: {backup}")
    # No automatic rollback once the new binary may have accepted deliveries.
    current = ENV.read_bytes()
    if not re.search(rb"^MIRELAY_ADMIN_TOKEN=", current, re.MULTILINE):
        atomic_file(ENV, current.rstrip(b"\n") + f"\nMIRELAY_ADMIN_TOKEN={secrets.token_hex(32)}\n".encode())
    atomic_file(BACKEND, staged.read_bytes(), 0o755)
    run(["systemctl", "start", "mirelay-server.service"])
    local_health()
    print("Backend upgraded; data and credentials preserved. Run check against the public HTTPS address.")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for command in ("install", "upgrade"):
        sub = commands.add_parser(command)
        sub.add_argument("--server-binary", required=True)
        sub.add_argument("--server-sha256", required=True)
        sub.add_argument("--apply", action="store_true", help="Actually modify this host; otherwise only print a plan")
        if command == "install":
            sub.add_argument("--address", required=True)
            sub.add_argument("--caddy-binary", required=True)
            sub.add_argument("--caddy-sha256", required=True)
    commands.add_parser("check").add_argument("--address", required=True)
    args = parser.parse_args(argv)
    if args.command == "check":
        check(address(args.address)[0])
    else:
        server = verified_file(args.server_binary, args.server_sha256)
        (install if args.command == "install" else upgrade)(args, server)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, RuntimeError, OSError) as failure:
        print(f"Stopped: {failure}", file=sys.stderr)
        print("No unrelated services were intentionally changed. After a partial install/upgrade, inspect MiRelay and any reported backup before retrying.", file=sys.stderr)
        sys.exit(1)

import hashlib
import importlib.util
import io
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from contextlib import redirect_stdout

spec = importlib.util.spec_from_file_location("setup", Path(__file__).with_name("setup.py"))
setup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(setup)


class SetupTests(unittest.TestCase):
    def test_rejects_config_injection_and_private_addresses(self):
        for value in ("example.com\n}", "example.com:443", "https://example.com", "$(id).com", "example..com", "127.0.0.1", "10.0.0.1", "::1", "999.1.2.3", "a" * 64 + ".com"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                setup.address(value)

    def test_public_address_and_domain_config_preserve_udp(self):
        for value in ("8.8.8.8", "relay.example.com"):
            host, is_ip = setup.address(value)
            config = setup.caddy_config(host, is_ip).decode()
            self.assertIn("protocols h1 h2", config)
            self.assertIn("disable_http_challenge", config)
            self.assertIn("127.0.0.1:8080", config)
            self.assertNotIn("tls internal", config)
            self.assertEqual("profile shortlived" in config, is_ip)

    def test_hash_mismatch_missing_file_relative_path_and_symlink(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "binary"
            path.write_bytes(b"fixture")
            sha = hashlib.sha256(b"fixture").hexdigest()
            self.assertEqual(setup.verified_file(str(path), sha), path)
            link = path.with_name("link")
            link.symlink_to(path)
            for source, checksum in ((str(path), "0" * 64), (str(link), sha), ("binary", sha), (str(path), "bad")):
                with self.assertRaises(ValueError):
                    setup.verified_file(source, checksum)

    def test_plan_never_runs_commands_or_installs_files(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fixture"
            path.write_bytes(b"not an executable")
            sha = hashlib.sha256(path.read_bytes()).hexdigest()
            with patch.object(setup, "run", side_effect=AssertionError("Plan executed a command")), patch.object(setup, "atomic_file", side_effect=AssertionError("Plan wrote a file")), redirect_stdout(io.StringIO()):
                setup.main(["install", "--address", "relay.example.com", "--server-binary", str(path), "--server-sha256", sha,
                            "--caddy-binary", str(path), "--caddy-sha256", sha])
                setup.main(["upgrade", "--server-binary", str(path), "--server-sha256", sha])

    def test_atomic_file_is_private_and_rejects_symlink_targets(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "secret"
            setup.atomic_file(path, b"old")
            setup.atomic_file(path, b"new")
            self.assertEqual(path.read_bytes(), b"new")
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            link = path.with_name("link")
            link.symlink_to(path)
            with self.assertRaises(ValueError):
                setup.atomic_file(link, b"overwrite")
            self.assertEqual(path.read_bytes(), b"new")

    def test_backup_syncs_parent_entry_without_following_symlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            backup = root / "backup"
            backup.mkdir()
            payload = backup / "data"
            payload.write_bytes(b"fixture")
            outside = root / "outside"
            outside.write_bytes(b"not backup data")
            (backup / "link").symlink_to(outside)
            synced = []
            with patch.object(setup.os, "fsync", side_effect=lambda fd: synced.append(Path(os.readlink(f"/proc/self/fd/{fd}")))):
                setup.sync_backup(backup)
            self.assertEqual(synced, [payload, backup, root])

    def test_backup_special_file_is_rejected_without_blocking(self):
        with tempfile.TemporaryDirectory() as directory:
            backup = Path(directory)
            os.mkfifo(backup / "fifo")
            with self.assertRaises(RuntimeError):
                setup.sync_backup(backup)


if __name__ == "__main__":
    unittest.main()

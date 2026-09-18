import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("audit_public", Path(__file__).with_name("audit-public.py"))
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class PublicationAuditTests(unittest.TestCase):
    def test_secret_output_contains_location_not_value(self):
        token = b"gh" + b"p_" + b"z" * 36
        report = audit.findings("example.txt", b"first\n" + token)
        self.assertEqual(report, [{"path": "example.txt", "rule": "service-token", "line": 2}])
        self.assertNotIn(token.decode(), str(report))

    def test_private_key_even_in_binary(self):
        marker = b"-----BEGIN " + b"OPENSSH PRIVATE KEY-----"
        self.assertEqual(audit.findings("asset.bin", b"\0" + marker)[0]["rule"], "private-key")

    def test_sensitive_file_even_empty(self):
        self.assertEqual(audit.findings("android/release.jks", b"")[0]["rule"], "sensitive-file")

    def test_personal_path_and_example(self):
        self.assertEqual(audit.findings("notes.md", b"/home/" + b"private-owner/Photos")[0]["rule"], "personal-home")
        self.assertEqual(audit.findings("notes.md", b"/home/user/Photos"), [])

    def test_history_finds_secret_removed_from_current_tree(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            def git(*args):
                subprocess.run(["git", "-C", str(root), *args], check=True, capture_output=True)
            git("init")
            git("config", "user.name", "Audit Test")
            git("config", "user.email", "audit@example.invalid")
            p = root / "notes.md"
            p.write_bytes(b"gh" + b"p_" + b"z" * 36)
            git("add", "notes.md"); git("commit", "-m", "synthetic fixture")
            p.write_text("removed\n")
            git("add", "notes.md"); git("commit", "-m", "remove fixture")
            self.assertFalse(audit.scan(root)["findings"])
            history = audit.scan(root, True)
            self.assertEqual(len(history["findings"]), 1)
            self.assertIn("blob", history["findings"][0])

    def test_symlink_fails_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(["git", "init", str(root)], check=True, capture_output=True)
            (root / "link").symlink_to("unrelated")
            subprocess.run(["git", "-C", str(root), "add", "link"], check=True, capture_output=True)
            with self.assertRaises(ValueError):
                audit.scan(root)


if __name__ == "__main__":
    unittest.main()

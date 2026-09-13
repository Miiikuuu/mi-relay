import contextlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("ci_run", Path(__file__).with_name("ci-run.py"))
driver = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(driver)


class ReportTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        subprocess.run(["git", "-c", "user.name=CI fixture", "-c", "user.email=ci@example.invalid",
                        "commit", "-q", "--allow-empty", "-m", "synthetic"], cwd=self.root, check=True)

    def invoke(self, code, *, status=0, extra=()):
        with (patch.object(driver, "ROOT", self.root),
              patch("sys.argv", ["ci-run.py", "--label", "test", *extra, "--", sys.executable, "-c", code]),
              contextlib.redirect_stdout(io.StringIO())):
            if status:
                with self.assertRaises(SystemExit) as caught:
                    driver.main()
                self.assertEqual(caught.exception.code, status)
            else:
                driver.main()
        return json.loads((self.root / "target/ci-reports/test/result.json").read_text())

    def test_success_and_repetition_keep_counts_separate(self):
        result = self.invoke("print('test result: ok. 3 passed; 0 failed; 1 ignored;')",
                             extra=("--repeat", "2", "--min-tests", "3"))
        self.assertTrue(result["completed"])
        self.assertEqual(len(result["runs"]), 2)
        for row in result["runs"]:
            self.assertEqual(row["rust_counts"], {"passed": 3, "failed": 0, "ignored": 1})
            self.assertEqual(driver.sha(self.root / "target/ci-reports/test" / row["log"]), row["log_sha256"])

    def test_failure_is_retained_and_stops_repetition(self):
        result = self.invoke("print('failure evidence'); raise SystemExit(7)", status=7, extra=("--repeat", "3"))
        self.assertNotIn("completed", result)
        self.assertEqual(len(result["runs"]), 1)
        self.assertEqual(result["runs"][0]["exit_code"], 7)

    def test_timeout_is_not_a_success(self):
        result = self.invoke("import time; time.sleep(60)", status=124, extra=("--timeout", "1"))
        self.assertEqual(result["runs"][0]["exit_code"], 124)
        self.assertNotIn("completed", result)

    def test_empty_cargo_filter_is_rejected(self):
        result = self.invoke("print('test result: ok. 0 passed; 0 failed; 0 ignored;')",
                             status=1, extra=("--min-tests", "1"))
        self.assertEqual(result["runs"][0]["command_exit_code"], 0)
        self.assertEqual(result["runs"][0]["exit_code"], 1)

    def test_label_cannot_escape_reports_directory(self):
        with (patch.object(driver, "ROOT", self.root),
              patch("sys.argv", ["ci-run.py", "--label", "../escape", "--", sys.executable]),
              contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit)):
            driver.main()
        self.assertFalse((self.root / "target").exists())

    def test_missing_command_is_recorded(self):
        with (patch.object(driver, "ROOT", self.root),
              patch("sys.argv", ["ci-run.py", "--label", "missing", "--", str(self.root / "not-a-command")]),
              contextlib.redirect_stdout(io.StringIO()), self.assertRaises(SystemExit) as caught):
            driver.main()
        self.assertEqual(caught.exception.code, 127)
        result = json.loads((self.root / "target/ci-reports/missing/result.json").read_text())
        self.assertEqual(result["runs"][0]["exit_code"], 127)


if __name__ == "__main__":
    unittest.main()

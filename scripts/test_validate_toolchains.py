"""Exercise the A1 driver's bookkeeping with fake compilers, never real devices."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location(
    "toolchain_validation", Path(__file__).with_name("validate-toolchains.py"))
driver = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(driver)


class ValidationTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.ndk = self.root / "ndk"
        binary = self.ndk / "toolchains/llvm/prebuilt/linux-x86_64/bin/clang"
        binary.parent.mkdir(parents=True)
        binary.write_bytes(b"fixture")
        (self.ndk / "source.properties").write_text("Pkg.Revision = 27.2.12479018\n")
        files = {
            "Cargo.toml": '[package]\nname="fixture"\nrust-version="1.88"\n',
            "android/native/Cargo.toml": '[package]\nname="native"\nrust-version="1.88"\n',
            "rust-toolchain.toml": '[toolchain]\nchannel="1.98.1"\n',
            "Cargo.lock": "root locked fixture\n",
            "android/native/Cargo.lock": "native locked fixture\n",
        }
        stream = io.BytesIO()
        with tarfile.open(fileobj=stream, mode="w") as archive:
            for name, text in files.items():
                content = text.encode()
                entry = tarfile.TarInfo(name)
                entry.size = len(content)
                archive.addfile(entry, io.BytesIO(content))
                path = self.root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(content)
        self.archive = stream.getvalue()
        (self.root / "secret.txt").write_text("untracked must never be copied")

    def run_driver(self, outcome=None, overlay=False):
        def fake_run(command, **kwargs):
            if "build" in command:
                if outcome == "timeout":
                    raise subprocess.TimeoutExpired(command, 3600)
                if outcome == "failure":
                    return subprocess.CompletedProcess(command, 101)
                target_dir = Path(kwargs["env"]["CARGO_TARGET_DIR"])
                if "--target" in command:
                    target = command[command.index("--target") + 1]
                    paths = [target_dir / target / "release/libmirelay_android.so"]
                else:
                    paths = [target_dir / "debug" / name
                             for name in ("mirelay", "mirelay-upload", "mirelay-server", "mirelay-desktop")]
                for path in paths:
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(b"fake binary")
            return subprocess.CompletedProcess(command, 0)

        argv = ["validate-toolchains.py", "--ndk", str(self.ndk)]
        if overlay:
            argv.append("--working-metadata")
        with (patch.object(driver, "ROOT", self.root),
              patch.object(driver.platform, "platform", return_value="fixture-linux"),
              patch.object(driver, "capture", side_effect=["a" * 40, "GTK fixture"]),
              patch.object(driver.subprocess, "check_output", return_value=self.archive),
              patch.object(driver.subprocess, "run", side_effect=fake_run),
              patch("sys.argv", argv), contextlib.redirect_stdout(io.StringIO())):
            if outcome:
                with self.assertRaises(SystemExit) as caught:
                    driver.main()
                self.assertEqual(caught.exception.code, 124 if outcome == "timeout" else 101)
            else:
                driver.main()
        report, = (self.root / "target").iterdir()
        return report, json.loads((report / "results.json").read_text())

    def test_matrix_records_separate_locks_artifacts_and_clean_inputs(self):
        report, result = self.run_driver()
        builds = [step for step in result["steps"] if "build" in step["command"]]
        self.assertEqual(len(builds), 8)
        self.assertEqual(len(result["steps"]), 12)
        self.assertEqual(len(result["artifacts"]), 12)
        self.assertEqual(len(result["locks"]), 2)
        self.assertTrue(result["all_builds_passed"])
        self.assertEqual(result["metadata_overlays"], {})
        self.assertFalse((report / "source/secret.txt").exists())
        self.assertEqual(driver.digest(report / "runner.py"), result["runner_sha256"])
        for step in builds:
            self.assertIn("--locked", step["command"])
            self.assertIn("--offline", step["command"])
            self.assertEqual(step["exit_code"], 0)
            self.assertEqual(driver.digest(report / step["log"]), step["log_sha256"])

    def test_failed_build_retains_logs_without_claiming_success(self):
        report, result = self.run_driver("failure")
        self.assertEqual(len(result["steps"]), 3)
        self.assertEqual(result["steps"][-1]["exit_code"], 101)
        self.assertNotIn("all_builds_passed", result)
        self.assertTrue((report / result["steps"][-1]["log"]).is_file())

    def test_timeout_is_recorded_as_failure(self):
        _, result = self.run_driver("timeout")
        self.assertEqual(result["steps"][-1]["exit_code"], 124)
        self.assertTrue(result["steps"][-1]["timed_out"])
        self.assertNotIn("all_builds_passed", result)

    def test_overlay_is_explicit_and_restricted_to_metadata(self):
        (self.root / "Cargo.toml").write_text('[package]\nname="changed"\nrust-version="1.88"\n')
        report, result = self.run_driver(overlay=True)
        self.assertEqual(set(result["metadata_overlays"]), set(driver.METADATA_FILES))
        self.assertIn('name="changed"', (report / "source/Cargo.toml").read_text())
        self.assertFalse((report / "source/secret.txt").exists())

    def test_mismatched_package_minima_are_rejected(self):
        (self.root / "android/native/Cargo.toml").write_text(
            '[package]\nname="native"\nrust-version="1.85"\n')
        with self.assertRaisesRegex(RuntimeError, "MSRVs disagree"):
            self.run_driver(overlay=True)


if __name__ == "__main__":
    unittest.main()

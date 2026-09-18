import importlib.util
import os
import argparse
import json
from pathlib import Path
import struct
import tempfile
import unittest
from contextlib import ExitStack
from unittest.mock import patch
import zipfile

SPEC = importlib.util.spec_from_file_location("android_release", Path(__file__).resolve().parents[1] / "android/scripts/release.py")
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


def metadata():
    return {"package": "io.mirelay.android", "version_name": "0.1.0-beta.1", "version_code": 2,
            "min_sdk": 26, "target_sdk": 36, "debuggable": False, "debug_certificate": False,
            "zip_aligned_16k": True, "signature_valid": True, "certificates_sha256": ["ab" * 32]}


def elf(machine=183, alignment=16384):
    data = bytearray(120)
    data[:6] = b"\x7fELF\x02\x01"
    struct.pack_into("<H", data, 18, machine)
    struct.pack_into("<Q", data, 32, 64)
    struct.pack_into("<HH", data, 54, 56, 1)
    struct.pack_into("<I", data, 64, 1)
    struct.pack_into("<Q", data, 112, alignment)
    return data


class ReleaseTests(unittest.TestCase):
    def test_package_versions_must_match(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "android/native").mkdir(parents=True)
            (root / "Cargo.toml").write_text('[package]\nversion = "0.1.0-beta.1"\n')
            (root / "android/native/Cargo.toml").write_text('[package]\nversion = "0.1.0-beta.1"\n')
            with patch.object(release, "ROOT", root):
                self.assertEqual(release.source_version(), "0.1.0-beta.1")
                (root / "android/native/Cargo.toml").write_text('[package]\nversion = "0.1.0"\n')
                with self.assertRaises(ValueError):
                    release.source_version()

    def test_special_apk_path_rejected_before_copy(self):
        with self.assertRaises(ValueError):
            release.checked_apk_path(Path("/dev/zero"))

    def signing_fixture(self, *, wrong_hash=False, wrong_signer=False, previous_mismatch=False):
        # These mock crypto tool invocations; they do NOT prove real release signing.
        with tempfile.TemporaryDirectory() as directory, ExitStack() as stack:
            root = Path(directory)
            key = root / "fake.jks"
            key.write_text("fake key-path fixture, not cryptographic material")
            key.chmod(0o600)
            source = root / "unsigned.apk"
            source.write_bytes(b"synthetic APK fixture")
            output = root / "result"
            output.mkdir()
            args = argparse.Namespace(apk=source, apk_sha256=("00" * 32 if wrong_hash else release.digest(source)),
                                      certificate_sha256="ab" * 32, keystore=key, alias="release-test",
                                      version="0.1.0-beta.1", version_code=2,
                                      previous_apk=(source if previous_mismatch else None))
            unsigned = {**metadata(), "signature_valid": False, "certificates_sha256": [], "signing_material_present": False}
            signed = {**metadata(), "signing_material_present": True}
            if wrong_signer:
                signed["certificates_sha256"] = ["cd" * 32]
            inspected = [unsigned, signed]
            if previous_mismatch:
                inspected.append({**metadata(), "version_code": 1, "certificates_sha256": ["cd" * 32]})
            stack.enter_context(patch.dict(os.environ, {}, clear=True))
            stack.enter_context(patch.object(release.sys.stdin, "isatty", return_value=True))
            stack.enter_context(patch.object(release.sys.stderr, "isatty", return_value=True))
            stack.enter_context(patch.object(release, "output_directory", return_value=output))
            stack.enter_context(patch.object(release, "tools", return_value=(Path("/fake-sdk"), {})))
            stack.enter_context(patch.object(release, "inspect", side_effect=inspected))
            password = stack.enter_context(patch.object(release.getpass, "getpass", side_effect=["synthetic-passphrase", ""]))

            def fake_tool(command, **kwargs):
                self.assertNotIn("synthetic-passphrase", " ".join(map(str, command)))
                self.assertNotIn("synthetic-passphrase", str(kwargs.get("env")))
                if Path(command[0]).name == "zipalign":
                    Path(command[-1]).write_bytes(Path(command[-2]).read_bytes())
                else:
                    self.assertEqual(kwargs["password"], "synthetic-passphrase\nsynthetic-passphrase\n")
                    Path(command[command.index("--out") + 1]).write_bytes(b"mock signed bytes")
            stack.enter_context(patch.object(release, "capture", side_effect=fake_tool))
            if wrong_hash or wrong_signer or previous_mismatch:
                with self.assertRaises(ValueError):
                    release.sign(args)
                self.assertFalse((output / "signed.apk").exists())
                self.assertFalse((output / "verified.json").exists())
                if wrong_hash:
                    password.assert_not_called()
            else:
                release.sign(args)
                self.assertEqual((output / "signed.apk").read_bytes(), b"mock signed bytes")
                report = json.loads((output / "verified.json").read_text())
                self.assertEqual(report["apk"]["certificates_sha256"], ["ab" * 32])
                self.assertNotIn("synthetic-passphrase", (output / "verified.json").read_text())
            self.assertEqual(source.read_bytes(), b"synthetic APK fixture")

    def test_local_signing_orchestration_does_not_put_password_in_argv_or_reports(self):
        self.signing_fixture()

    def test_wrong_input_hash_blocks_password_prompt(self):
        self.signing_fixture(wrong_hash=True)

    def test_wrong_signer_never_gets_accepted_output_name(self):
        self.signing_fixture(wrong_signer=True)

    def test_legacy_signer_mismatch_never_gets_accepted_output_name(self):
        self.signing_fixture(previous_mismatch=True)

    def test_version_code_bounds_and_ambiguous_input(self):
        self.assertEqual(release.version_code("2"), 2)
        self.assertEqual(release.version_code("2100000000"), 2100000000)
        for value in ("1", "0", "-1", "02", "2.0", "2\n", "2100000001", "9" * 50):
            with self.subTest(value=value), self.assertRaises(ValueError):
                release.version_code(value)

    def test_public_certificate_fingerprint(self):
        self.assertEqual(release.fingerprint(":".join(["AB"] * 32)), "ab" * 32)
        for value in ("", "g" * 64, "a" * 63, "a" * 65, "a" * 64 + "\n"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                release.fingerprint(value)

    def test_badging_requires_complete_identity(self):
        text = "package: name='io.mirelay.android' versionCode='1' versionName='0.1.0-dev'\nminSdkVersion:'26'\ntargetSdkVersion:'36'\napplication-debuggable\n"
        self.assertTrue(release.parse_badging(text)["debuggable"])
        with self.assertRaises(ValueError):
            release.parse_badging(text.replace("minSdkVersion:'26'", ""))

    def test_signer_parser_distinguishes_public_key_from_certificate(self):
        info = release.parse_signers("Signer #1 certificate DN: C=US, O=Android, CN=Android Debug\nSigner #1 certificate SHA-256 digest: " + "ab" * 32 + "\nSigner #1 public key SHA-256 digest: " + "cd" * 32)
        self.assertTrue(info["debug_certificate"])
        self.assertEqual(info["certificates_sha256"], ["ab" * 32])

    def test_release_rejects_debug_wrong_identity_versions_sdk_and_alignment(self):
        release.release_checks(metadata(), "0.1.0-beta.1", 2, "ab" * 32)
        bad = {"debuggable": True, "debug_certificate": True, "package": "other.app",
               "version_name": "0.1.0-dev", "version_code": 1, "min_sdk": 27,
               "target_sdk": 35, "zip_aligned_16k": False, "signature_valid": False,
               "certificates_sha256": ["cd" * 32]}
        for field, value in bad.items():
            with self.subTest(field=field), self.assertRaises(ValueError):
                release.release_checks({**metadata(), field: value}, "0.1.0-beta.1", 2, "ab" * 32)

    def test_multiple_signers_and_dev_version_not_accepted(self):
        with self.assertRaises(ValueError):
            release.release_checks({**metadata(), "certificates_sha256": ["ab" * 32, "cd" * 32]}, "0.1.0-beta.1", 2, "ab" * 32)
        with self.assertRaises(ValueError):
            release.release_checks(metadata(), "0.1.0-dev", 2)

    def test_upgrade_requires_same_signer_and_increasing_code(self):
        previous = metadata()
        candidate = {**metadata(), "version_code": 3}
        release.upgrade_checks(previous, candidate)
        for field, value in (("certificates_sha256", ["cd" * 32]), ("package", "other.app"),
                             ("version_code", 2), ("version_code", 1), ("signature_valid", False)):
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                release.upgrade_checks(previous, {**candidate, field: value})

    def test_elf_validates_machine_alignment_headers_and_bounds(self):
        self.assertEqual(release.elf_alignment(elf(), 183), [16384])
        bad = [elf(machine=62), elf(alignment=4096), elf(alignment=20000), elf()[:63]]
        offset = elf()
        struct.pack_into("<Q", offset, 32, 10000)
        bad.append(offset)
        data_size = elf()
        struct.pack_into("<Q", data_size, 96, 10000)
        bad.append(data_size)
        for data in bad:
            with self.subTest(data=data[:20]), self.assertRaises(ValueError):
                release.elf_alignment(data, 183)

    def test_all_native_libraries_are_checked_and_both_abis_required(self):
        with tempfile.TemporaryDirectory() as directory:
            apk = Path(directory) / "synthetic.apk"
            with zipfile.ZipFile(apk, "w") as archive:
                for abi, machine in release.ABIS.items():
                    archive.writestr(f"lib/{abi}/libmirelay_android.so", elf(machine))
            self.assertEqual(len(release.native_libraries(apk)), 2)
            with zipfile.ZipFile(apk, "a") as archive:
                archive.writestr("lib/arm64-v8a/libthirdparty.so", elf(alignment=4096))
            with self.assertRaises(ValueError):
                release.native_libraries(apk)
            with zipfile.ZipFile(apk, "w") as archive:
                archive.writestr("lib/arm64-v8a/libmirelay_android.so", elf())
            with self.assertRaises(ValueError):
                release.native_libraries(apk)

    def test_unsigned_detection_includes_v1_and_apk_signing_blocks(self):
        with tempfile.TemporaryDirectory() as directory:
            apk = Path(directory) / "synthetic.apk"
            with zipfile.ZipFile(apk, "w") as archive:
                archive.writestr("AndroidManifest.xml", b"fixture")
            self.assertFalse(release.has_signing_material(apk))
            data = bytearray(apk.read_bytes())
            end = data.rfind(b"PK\x05\x06")
            central = struct.unpack_from("<I", data, end + 16)[0]
            data[central - 16:central] = b"APK Sig Block 42"
            apk.write_bytes(data)
            self.assertTrue(release.has_signing_material(apk))
            with zipfile.ZipFile(apk, "w") as archive:
                archive.writestr("META-INF/CERT.RSA", b"not a real certificate")
            self.assertTrue(release.has_signing_material(apk))

    def test_private_key_requires_owner_only_outside_repository(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repository = root / "repo"
            repository.mkdir()
            key = root / "fake.jks"
            key.write_text("Not a private key; path guard fixture only")
            key.chmod(0o600)
            with patch.object(release, "ROOT", repository):
                self.assertEqual(release.private_key_path(key), key)
                key.chmod(0o644)
                with self.assertRaises(ValueError):
                    release.private_key_path(key)
                key.chmod(0o600)
                link = repository / "link.jks"
                link.symlink_to(key)
                with self.assertRaises(ValueError):
                    release.private_key_path(link)
            with patch.object(release, "ROOT", root), self.assertRaises(ValueError):
                release.private_key_path(key)

    def test_signing_refuses_ci_and_noninteractive_before_reading_key(self):
        with patch.dict(os.environ, {"CI": "true"}), self.assertRaises(ValueError):
            release.sign(None)
        with patch.dict(os.environ, {}, clear=True), patch.object(release.sys.stdin, "isatty", return_value=False), self.assertRaises(ValueError):
            release.sign(None)


if __name__ == "__main__":
    unittest.main()

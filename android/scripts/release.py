#!/usr/bin/env python3
"""Prepare/inspect/verify APKs, or explicitly sign a copy on a local terminal.

No key generation, installation, uploads, tags, device access or data migration.
Requires Python 3.11+, the repository-local Android toolchain and Linux.
"""
import argparse
import getpass
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import struct
import subprocess
import sys
import tempfile
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[2]
VERSION = re.compile(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:-(?:alpha|beta|rc)\.(?:0|[1-9][0-9]*))?")
ABIS = {"arm64-v8a": 183, "x86_64": 62}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def fingerprint(value):
    value = value.replace(":", "").lower()
    require(re.fullmatch(r"[0-9a-f]{64}", value), "Expected a SHA-256 fingerprint (64 hexadecimal digits).")
    return value


def version_code(value):
    require(re.fullmatch(r"[1-9][0-9]{0,9}", str(value)), "Invalid versionCode.")
    code = int(value)
    require(2 <= code <= 2100000000, "Release versionCode must be in 2..2100000000.")
    return code


def source_version():
    versions = [tomllib.loads((ROOT / name).read_text())["package"]["version"]
                for name in ("Cargo.toml", "android/native/Cargo.toml")]
    require(versions[0] == versions[1] and VERSION.fullmatch(versions[0]),
            "Root/native package versions must match and be a release or alpha/beta/rc version.")
    return versions[0]


def capture(command, *, env=None, check=True, password=None):
    # Never print command arguments or captured signing diagnostics/passwords.
    result = subprocess.run([str(x) for x in command], cwd=ROOT, env=env,
                            input=password, text=True, encoding="utf-8", stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, timeout=120)
    if check and result.returncode:
        raise ValueError(f"{Path(command[0]).name} failed (exit {result.returncode}); artifact not accepted.")
    return result


def tools():
    sdk = Path(os.environ.get("ANDROID_SDK_ROOT") or os.environ.get("ANDROID_HOME")
               or ROOT / "android/.local/sdk")
    build = sdk / "build-tools/35.0.0"
    jdk = ROOT / "android/.local/jdk"
    require(all((build / name).is_file() for name in ("aapt2", "apksigner", "zipalign"))
            and (jdk / "bin/java").is_file(), "Install the documented local Android toolchain first.")
    env = {**os.environ, "JAVA_HOME": str(jdk), "PATH": str(jdk / "bin") + os.pathsep + os.environ.get("PATH", ""),
           "LC_ALL": "C"}
    return build, env


def parse_badging(text):
    package = re.search(r"^package: name='([^']+)' versionCode='([0-9]+)' versionName='([^']+)'", text, re.M)
    minimum = re.search(r"^minSdkVersion:'([0-9]+)'$", text, re.M)
    target = re.search(r"^targetSdkVersion:'([0-9]+)'$", text, re.M)
    require(package and minimum and target, "APK metadata is missing or malformed.")
    return {"package": package[1], "version_code": int(package[2]), "version_name": package[3],
            "min_sdk": int(minimum[1]), "target_sdk": int(target[1]),
            "debuggable": "application-debuggable" in text.splitlines()}


def parse_signers(text):
    signers = re.findall(r"^Signer #[0-9]+ certificate SHA-256 digest: ([0-9a-fA-F]{64})$", text, re.M)
    return {"certificates_sha256": [s.lower() for s in signers],
            "debug_certificate": bool(re.search(r"^Signer #[0-9]+ certificate DN:.*CN=Android Debug(?:,|$)", text, re.M))}


def elf_alignment(data, machine):
    require(len(data) >= 64 and data[:6] == b"\x7fELF\x02\x01", "Expected a little-endian ELF64 library.")
    require(struct.unpack_from("<H", data, 18)[0] == machine, "Wrong native ELF machine.")
    offset = struct.unpack_from("<Q", data, 32)[0]
    entry, count = struct.unpack_from("<HH", data, 54)
    require(entry == 56 and 0 < count < 1024 and offset + count * entry <= len(data), "Invalid ELF program headers.")
    alignments = []
    for pos in range(offset, offset + count * entry, entry):
        if struct.unpack_from("<I", data, pos)[0] != 1:
            continue
        file_offset, address = struct.unpack_from("<QQ", data, pos + 8)
        size = struct.unpack_from("<Q", data, pos + 32)[0]
        alignment = struct.unpack_from("<Q", data, pos + 48)[0]
        require(alignment >= 16384 and alignment & (alignment - 1) == 0
                and (file_offset - address) % alignment == 0 and file_offset + size <= len(data),
                "Native LOAD segment is not valid/16 KiB aligned.")
        alignments.append(alignment)
    require(alignments, "Native library has no LOAD segments.")
    return alignments


def native_libraries(apk):
    libraries = {}
    total_bytes = 0
    with zipfile.ZipFile(apk) as archive:
        names = archive.namelist()
        require(len(names) == len(set(names)), "Duplicate APK ZIP entries.")
        for item in archive.infolist():
            if not item.filename.startswith("lib/"):
                continue
            if item.is_dir():
                continue
            parts = item.filename.split("/")
            require(len(parts) == 3 and parts[1] in ABIS and parts[2].endswith(".so"), "Unexpected native APK entry.")
            require(item.file_size <= 128 * 1024 * 1024, "Native library exceeds inspection budget.")
            total_bytes += item.file_size
            require(total_bytes <= 512 * 1024 * 1024 and len(libraries) < 32, "Native inspection budget exceeded.")
            data = archive.read(item)
            libraries[item.filename] = {"sha256": hashlib.sha256(data).hexdigest(),
                                        "load_alignment": elf_alignment(data, ABIS[parts[1]])}
    require(all(f"lib/{abi}/libmirelay_android.so" in libraries for abi in ABIS), "Both MiRelay JNI ABIs are required.")
    return libraries


def has_signing_material(apk):
    with zipfile.ZipFile(apk) as archive:
        if any(re.fullmatch(r"META-INF/[^/]+\.(RSA|DSA|EC|SF)", name, re.I) for name in archive.namelist()):
            return True
    # Locate the non-ZIP64 APK central directory, then inspect its preceding
    # signing-block magic. Invalid/ambiguous ZIP metadata is never "unsigned".
    with apk.open("rb") as source:
        size = source.seek(0, 2)
        source.seek(max(0, size - 65557))
        tail = source.read()
        end = tail.rfind(b"PK\x05\x06")
        require(end >= 0 and end + 22 <= len(tail), "Missing ZIP end record.")
        comment_size = struct.unpack_from("<H", tail, end + 20)[0]
        require(end + 22 + comment_size == len(tail), "Invalid ZIP end record.")
        central = struct.unpack_from("<I", tail, end + 16)[0]
        require(16 <= central < size and central != 0xffffffff, "Unsupported APK central directory.")
        source.seek(central - 16)
        return source.read(16) == b"APK Sig Block 42"


def checked_apk_path(apk):
    apk = apk.resolve(strict=True)
    require(apk.is_file() and apk.stat().st_size <= 256 * 1024 * 1024, "APK must be a regular file of at most 256 MiB.")
    return apk


def inspect(apk):
    apk = checked_apk_path(apk)
    before = digest(apk)
    build, env = tools()
    metadata = parse_badging(capture([build / "aapt2", "dump", "badging", apk], env=env).stdout)
    signature = capture([build / "apksigner", "verify", "--verbose", "--print-certs", apk], env=env, check=False)
    align = capture([build / "zipalign", "-c", "-P", "16", "4", apk], env=env, check=False)
    metadata.update(sha256=before, signature_valid=signature.returncode == 0,
                    signing_material_present=has_signing_material(apk),
                    zip_aligned_16k=align.returncode == 0, **parse_signers(signature.stdout),
                    native_libraries=native_libraries(apk))
    require(digest(apk) == before, "APK changed during inspection.")
    return metadata


def release_checks(info, version, code, certificate=None):
    require(VERSION.fullmatch(version), "Invalid release version.")
    require(info["package"] == "io.mirelay.android", "Wrong application ID.")
    require(info["version_name"] == version and info["version_code"] == version_code(code), "APK version does not match the requested version.")
    require(info["min_sdk"] == 26 and info["target_sdk"] == 36, "Unexpected Android SDK bounds.")
    require(not info["debuggable"] and not info["debug_certificate"], "Debug builds/identities are not release artifacts.")
    require(info["zip_aligned_16k"], "APK ZIP alignment check failed.")
    if certificate is not None:
        require(info["signature_valid"] and info["certificates_sha256"] == [fingerprint(certificate)],
                "APK does not have the expected single release signer.")


def upgrade_checks(previous, candidate):
    require(previous["signature_valid"] and candidate["signature_valid"], "Both APK signatures must verify.")
    require(previous["package"] == candidate["package"], "Package IDs differ; not an in-place upgrade.")
    require(previous["certificates_sha256"] == candidate["certificates_sha256"]
            and len(candidate["certificates_sha256"]) == 1, "Signing identity differs; do not uninstall or clear data to bypass this.")
    require(candidate["version_code"] > previous["version_code"], "An upgrade needs a strictly greater versionCode.")


def private_key_path(path):
    require(not path.is_symlink(), "Keystore symlinks are not accepted.")
    path = path.resolve(strict=True)
    info = path.stat()
    require(not path.is_relative_to(ROOT.resolve()), "Keep the release keystore outside the repository.")
    require(stat.S_ISREG(info.st_mode) and info.st_uid == os.getuid() and info.st_mode & 0o077 == 0,
            "Keystore must be an owner-only regular file (for example mode 600).")
    return path


def output_directory():
    parent = ROOT / "android/.local/releases"
    parent.mkdir(parents=True, exist_ok=True)
    return Path(tempfile.mkdtemp(prefix="candidate-", dir=parent))


def prepare(args):
    version, code = source_version(), version_code(args.version_code)
    commit = capture(["git", "rev-parse", "HEAD"]).stdout.strip()
    dirty = bool(capture(["git", "status", "--porcelain"]).stdout.strip())
    require(not dirty or args.allow_dirty, "Freeze/commit the source first, or explicitly use --allow-dirty for unsigned development validation.")
    result = subprocess.run(["bash", "android/scripts/build.sh", ":app:assembleRelease",
                             f"-PmirelayReleaseVersion={version}", f"-PmirelayReleaseVersionCode={code}"], cwd=ROOT)
    require(result.returncode == 0, "Unsigned release build failed.")
    output = output_directory()
    apk = output / "unsigned.apk"
    shutil.copyfile(checked_apk_path(ROOT / "android/app/build/outputs/apk/release/app-release-unsigned.apk"), apk)
    info = inspect(apk)
    release_checks(info, version, code)
    require(not info["signing_material_present"], "Prepare must not use a signing identity.")
    report = {"scope": "unsigned build, not release approval", "commit": commit, "dirty": dirty, "apk": info}
    (output / "prepare.json").write_text(json.dumps(report, indent=2) + "\n")
    return {"output": str(output), **report}


def sign(args):
    require(not os.environ.get("CI") and not os.environ.get("GITHUB_ACTIONS"), "Signing is local-only; CI is not permitted.")
    require(sys.stdin.isatty() and sys.stderr.isatty(), "Run signing in your own interactive terminal; never send passwords through chat.")
    certificate = fingerprint(args.certificate_sha256)
    expected_hash = fingerprint(args.apk_sha256)
    key = private_key_path(args.keystore)
    require(re.fullmatch(r"[A-Za-z0-9_.-]{1,100}", args.alias), "Invalid key alias.")
    source = checked_apk_path(args.apk)
    output = output_directory()
    copied = output / "input.apk"
    shutil.copyfile(source, copied)
    require(digest(copied) == expected_hash, "Input APK hash differs from the reviewed artifact.")
    info = inspect(copied)
    release_checks(info, args.version, args.version_code)
    require(not info["signing_material_present"], "Sign only the reviewed unsigned APK, not an already signed package.")
    build, env = tools()
    aligned, signed = output / "aligned.apk", output / "unverified.apk"
    capture([build / "zipalign", "-P", "16", "4", copied, aligned], env=env)
    store_password = getpass.getpass("Keystore password (local terminal only): ")
    key_password = getpass.getpass("Key password (Enter = same as keystore): ") or store_password
    require(all(p and "\n" not in p and "\r" not in p for p in (store_password, key_password)), "Passwords must be nonempty single lines.")
    try:
        capture([build / "apksigner", "sign", "--ks", key, "--ks-key-alias", args.alias,
                 "--ks-pass", "stdin", "--key-pass", "stdin", "--pass-encoding", "utf-8", "--v4-signing-enabled", "false",
                 "--out", signed, aligned], env=env, password=store_password + "\n" + key_password + "\n")
    finally:
        store_password = key_password = ""  # Python cannot guarantee secure memory erasure.
    info = inspect(signed)
    release_checks(info, args.version, args.version_code, certificate)
    if args.previous_apk:
        upgrade_checks(inspect(args.previous_apk), info)
    signed.rename(output / "signed.apk")
    report = {"scope": "signed artifact checks; not installation or release approval", "apk": info}
    (output / "verified.json").write_text(json.dumps(report, indent=2) + "\n")
    return {"output": str(output), **report}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="action", required=True)
    prep = commands.add_parser("prepare", help="Build an unsigned release with matching Cargo versions")
    prep.add_argument("--version-code", required=True)
    prep.add_argument("--allow-dirty", action="store_true")
    view = commands.add_parser("inspect", help="Read-only identity/ABI/alignment inspection, not release acceptance")
    view.add_argument("apk", type=Path)
    for name in ("verify", "sign"):
        sub = commands.add_parser(name)
        sub.add_argument("apk", type=Path)
        sub.add_argument("--version", required=True)
        sub.add_argument("--version-code", required=True)
        sub.add_argument("--certificate-sha256", required=True, help="Reviewed public release certificate fingerprint")
        sub.add_argument("--previous-apk", type=Path)
        if name == "sign":
            sub.add_argument("--apk-sha256", required=True)
            sub.add_argument("--keystore", type=Path, required=True)
            sub.add_argument("--alias", required=True)
    args = parser.parse_args()
    try:
        if args.action == "prepare":
            result = prepare(args)
        elif args.action == "sign":
            result = sign(args)
        else:
            result = inspect(args.apk)
            if args.action == "verify":
                release_checks(result, args.version, args.version_code, args.certificate_sha256)
                if args.previous_apk:
                    upgrade_checks(inspect(args.previous_apk), result)
        print(json.dumps(result, indent=2))
    except (ValueError, OSError, zipfile.BadZipFile, subprocess.TimeoutExpired) as error:
        print(f"Artifact not accepted: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

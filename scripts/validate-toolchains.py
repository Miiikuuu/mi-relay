#!/usr/bin/env python3
"""A1 build-only validation in a fresh Git source export, never a device/relay test.

Requires preinstalled Rust toolchains, cached locked crates, Linux GTK development
packages and Android NDK r27c. Does not install dependencies or change defaults.
Each invocation creates a new private evidence directory under ignored target/.
"""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import subprocess
import tarfile
import tempfile
import time
import tomllib


ROOT = Path(__file__).resolve().parents[1]
METADATA_FILES = ("Cargo.toml", "android/native/Cargo.toml", "rust-toolchain.toml")


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def capture(command, cwd=ROOT):
    return subprocess.check_output(command, cwd=cwd, text=True).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--revision", default="HEAD", help="Git commit to export")
    parser.add_argument("--working-metadata", action="store_true",
                        help="Overlay ONLY the three toolchain/package manifests; report their hashes")
    parser.add_argument("--ndk", type=Path,
                        default=ROOT / "android/.local/sdk/ndk/27.2.12479018")
    args = parser.parse_args()
    revision = capture(["git", "rev-parse", "--verify", args.revision + "^{commit}"])
    (ROOT / "target").mkdir(exist_ok=True)
    report = Path(tempfile.mkdtemp(prefix="toolchain-validation-", dir=ROOT / "target"))
    (report / "runner.py").write_bytes(Path(__file__).read_bytes())
    source = report / "source"
    source.mkdir()
    archive = subprocess.check_output(["git", "archive", "--format=tar", revision], cwd=ROOT)
    with tarfile.open(fileobj=io.BytesIO(archive)) as bundle:
        bundle.extractall(source, filter="data")
    overlays = {}
    if args.working_metadata:
        for name in METADATA_FILES:
            content = (ROOT / name).read_bytes()
            (source / name).write_bytes(content)
            overlays[name] = hashlib.sha256(content).hexdigest()
    index = {str(path.relative_to(source)): digest(path)
             for path in sorted(source.rglob("*")) if path.is_file()}
    (report / "source-sha256.json").write_text(json.dumps(index, indent=2) + "\n")
    manifests = [tomllib.loads((source / name).read_text()) for name in METADATA_FILES[:2]]
    minimum = manifests[0]["package"]["rust-version"]
    if minimum != manifests[1]["package"]["rust-version"]:
        raise RuntimeError("Package MSRVs disagree")
    minimum += ".0" if minimum.count(".") == 1 else ""
    current = tomllib.loads((source / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    ndk = args.ndk.resolve() / "toolchains/llvm/prebuilt/linux-x86_64/bin"
    if not (ndk / "clang").is_file():
        raise RuntimeError("Install Android NDK r27c first")
    # Disallow ambient build overrides from silently changing the tested matrix.
    clean_env = {key: value for key, value in os.environ.items()
                 if key in ("CARGO_HOME", "RUSTUP_HOME")
                 or (not key.startswith(("CARGO_", "RUST", "CC_", "AR_"))
                     and key not in ("CC", "CXX", "AR"))}
    clean_env.update(CARGO_BUILD_JOBS="1", CARGO_INCREMENTAL="0", CARGO_TERM_COLOR="never")
    summary = {
        "scope": "A1 compilation only; not a frozen release or runtime acceptance",
        "base_commit": revision,
        "metadata_overlays": overlays,
        "source_index_sha256": digest(report / "source-sha256.json"),
        "runner_sha256": digest(report / "runner.py"),
        "host": platform.platform(),
        "ndk": str(args.ndk.resolve()),
        "ndk_properties": (args.ndk.resolve() / "source.properties").read_text(),
        "system_libraries": capture(["pkg-config", "--modversion", "gtk4", "libadwaita-1"]),
        "build_settings": {"jobs": 1, "incremental": False, "offline": True,
                           "locked": True, "android_api": 26,
                           "android_rustflags": "-C link-arg=-Wl,-z,max-page-size=16384"},
        "locks": {name: digest(source / name) for name in ("Cargo.lock", "android/native/Cargo.lock")},
        "steps": [],
    }

    def save():
        (report / "results.json").write_text(json.dumps(summary, indent=2) + "\n")

    def run(label, command, env):
        log = report / (label + ".log")
        start = time.monotonic()
        print(f"START {label}: {' '.join(command)}", flush=True)
        timed_out = False
        with log.open("w") as output:
            try:
                result = subprocess.run(command, cwd=source, env=env, stdout=output,
                                        stderr=subprocess.STDOUT, timeout=3600)
                exit_code = result.returncode
            except subprocess.TimeoutExpired:
                exit_code, timed_out = 124, True
        summary["steps"].append({"name": label, "command": command,
                                 "exit_code": exit_code, "timed_out": timed_out,
                                 "elapsed_seconds": round(time.monotonic() - start, 2),
                                 "log": log.name, "log_sha256": digest(log)})
        save()
        print(f"END {label}: exit={exit_code}", flush=True)
        if exit_code:
            print(log.read_text()[-6000:], flush=True)
            raise SystemExit(exit_code)

    save()
    print(f"Evidence: {report}", flush=True)
    for toolchain in (minimum, current):
        env = {**clean_env, "CARGO_TARGET_DIR": str(report / ("build-" + toolchain))}
        run(toolchain + "-rustc", ["rustup", "run", toolchain, "rustc", "-vV"], env)
        run(toolchain + "-cargo", ["rustup", "run", toolchain, "cargo", "-V"], env)
        cargo = ["rustup", "run", toolchain, "cargo"]
        for label, features in (("default", []), ("desktop", ["--features", "desktop"])):
            run(toolchain + "-" + label,
                [*cargo, "build", "--offline", "--locked", "--all-targets", *features], env)
        for target, clang in (("aarch64-linux-android", "aarch64-linux-android26"),
                              ("x86_64-linux-android", "x86_64-linux-android26")):
            target_env = {
                **env,
                "CARGO_TARGET_" + target.upper().replace("-", "_") + "_LINKER": str(ndk / (clang + "-clang")),
                "CC_" + target.replace("-", "_"): str(ndk / (clang + "-clang")),
                "AR_" + target.replace("-", "_"): str(ndk / "llvm-ar"),
                "RUSTFLAGS": "-C link-arg=-Wl,-z,max-page-size=16384",
            }
            run(toolchain + "-" + target,
                [*cargo, "build", "--offline", "--locked", "--manifest-path", "android/native/Cargo.toml",
                 "--release", "--target", target], target_env)
        if any(digest(source / name) != value for name, value in summary["locks"].items()):
            summary["lockfiles_changed"] = True
            save()
            raise RuntimeError("Validation changed a lockfile; no success is recorded")
    summary["artifacts"] = {
        str(path.relative_to(report)): digest(path)
        for toolchain in (minimum, current)
        for path in [*(report / ("build-" + toolchain) / "debug" / name
                      for name in ("mirelay", "mirelay-upload", "mirelay-server", "mirelay-desktop")),
                     *(report / ("build-" + toolchain) / target / "release/libmirelay_android.so"
                       for target in ("aarch64-linux-android", "x86_64-linux-android"))]
    }
    summary["all_builds_passed"] = True
    save()
    print("All eight locked builds passed. No runtime/device tests were run.", flush=True)


if __name__ == "__main__":
    main()

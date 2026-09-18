#!/usr/bin/env python3
"""Run native appearance checks on an owned headless, hardware-GL Weston.

Extract distro weston and libweston packages under target first; no host install
or real desktop connection. This is offscreen GPU QA, not physical display FPS.
Module map format: https://cgit.freedesktop.org/wayland/weston/tree/libweston/compositor.c
"""
import argparse
import os
import re
from pathlib import Path
import subprocess
import tempfile
import time


def stop_owned(process):
    if process is not None and process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--weston-root", type=Path, required=True)
    parser.add_argument("--test-binary", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    extracted = args.weston_root.resolve(strict=True)
    binary = args.test_binary.resolve(strict=True)
    if not extracted.is_relative_to(root / "target") or not binary.is_relative_to(root / "target/debug/deps"):
        parser.error("Use only repository-owned QA tools under target")
    lib = extracted / "usr/lib/x86_64-linux-gnu"
    backends = list(lib.glob("libweston-*/headless-backend.so"))
    if len(backends) != 1:
        parser.error("Expected exactly one extracted Weston backend version")
    modules = [backends[0], backends[0].with_name("gl-renderer.so"), lib / "weston/kiosk-shell.so"]
    for module in modules:
        if not module.is_file(): parser.error(f"Missing {module}")
    report = Path(tempfile.mkdtemp(prefix="appearance-gpu-", dir=root / "target"))
    print(f"REPORT={report}", flush=True)
    with tempfile.TemporaryDirectory(prefix="mirelay-gpu-") as runtime:
        env = {**os.environ, "XDG_RUNTIME_DIR": runtime, "WAYLAND_DISPLAY": "mirelay-gpu",
               "LD_LIBRARY_PATH": f"{lib}:{lib / 'weston'}",
               "WESTON_MODULE_MAP": ";".join(f"{p.name}={p}" for p in modules)}
        env.pop("DISPLAY", None)
        test = None
        with (report / "weston-console.log").open("w") as console, (report / "native-test.log").open("w") as output:
            weston = subprocess.Popen([str(extracted / "usr/bin/weston"), "--backend=headless", "--renderer=gl",
                "--shell=kiosk", "--socket=mirelay-gpu", "--width=1000", "--height=680", "--no-config",
                "--idle-time=0", f"--log={report / 'weston.log'}"], env=env, stdout=console, stderr=subprocess.STDOUT)
            try:
                deadline = time.monotonic() + 10
                while not (Path(runtime) / "mirelay-gpu").is_socket():
                    if weston.poll() is not None or time.monotonic() > deadline:
                        raise RuntimeError("Owned hardware Weston failed; see weston.log")
                    time.sleep(0.05)
                renderer_log = (report / "weston.log").read_text()
                if "GL renderer:" not in renderer_log or any(x in renderer_log.lower() for x in ("llvmpipe", "softpipe", "swrast")):
                    raise RuntimeError("Hardware compositor renderer was not confirmed")
                test = subprocess.Popen([str(binary),
                    "desktop::appearance::gpu_tests::offscreen_hardware_backdrop_cost",
                    "--exact", "--ignored", "--nocapture", "--test-threads=1"],
                    env={**env, "GDK_BACKEND": "wayland", "GSK_RENDERER": "gl", "GDK_DEBUG": "opengl",
                         "GTK_A11Y": "none", "G_DEBUG": "fatal-criticals", "MIRELAY_APPEARANCE_QA_DIR": str(report)},
                    stdout=output, stderr=subprocess.STDOUT)
                test.wait(timeout=60)
                output.flush()
                native_log = (report / "native-test.log").read_text()
                print(native_log, flush=True)
                if test.returncode: raise RuntimeError(f"Native GPU test exited {test.returncode}")
                device = re.search(r"Using rendering device: (/dev/dri/renderD\d+)", renderer_log)
                if (device is None or f"Device: {device[1]}," not in native_log
                        or "GTK renderer: GskGLRenderer" not in native_log
                        or any(x in native_log.lower() for x in ("llvmpipe", "softpipe", "swrast", "unrecognized value"))):
                    raise RuntimeError("Native GTK hardware device could not be matched to the compositor")
                print(f"Confirmed native GTK and compositor hardware device: {device[1]}; CLK_TCK={os.sysconf('SC_CLK_TCK')}", flush=True)
            finally:
                stop_owned(test)
                stop_owned(weston)


if __name__ == "__main__":
    main()

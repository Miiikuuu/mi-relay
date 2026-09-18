#!/usr/bin/env python3
"""Own an isolated Xvfb and focus only windows of the exact native test PID.

No window manager is needed; unlike a headless browser this supplies real X11
FocusIn/FocusOut events, including opening and closing native modal dialogs.
"""
import argparse
import ctypes as c
import os
from pathlib import Path
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--xvfb", type=Path, required=True)
    parser.add_argument("--test-binary", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    binary, xvfb = args.test_binary.resolve(strict=True), args.xvfb.resolve(strict=True)
    if not binary.is_relative_to(root / "target/debug/deps") or not xvfb.is_relative_to(root / "target"):
        parser.error("Use only this repository's test binary and extracted QA Xvfb")
    if Path("/tmp/.X11-unix/X98").exists() or Path("/tmp/.X98-lock").exists():
        parser.error("QA display :98 is already occupied; refusing to reuse it")
    report = Path(tempfile.mkdtemp(prefix="appearance-x11-", dir=root / "target"))
    print(f"REPORT={report}", flush=True)
    x = c.CDLL("libX11.so.6")
    def bind(name, result, *params):
        fn = getattr(x, name); fn.restype, fn.argtypes = result, params; return fn
    ptr, ul, integer = c.c_void_p, c.c_ulong, c.c_int
    open_display = bind("XOpenDisplay", ptr, c.c_char_p)
    close_display = bind("XCloseDisplay", integer, ptr)
    root_window = bind("XDefaultRootWindow", ul, ptr)
    class Attributes(c.Structure):
        _fields_ = [(name, integer) for name in ("x", "y", "width", "height", "border_width", "depth")] + [
            ("visual", ptr), ("root", ul), ("window_class", integer), ("bit_gravity", integer),
            ("win_gravity", integer), ("backing_store", integer), ("backing_planes", ul),
            ("backing_pixel", ul), ("save_under", integer), ("colormap", ul),
            ("map_installed", integer), ("map_state", integer), ("all_event_masks", c.c_long),
            ("your_event_mask", c.c_long), ("do_not_propagate_mask", c.c_long),
            ("override_redirect", integer), ("screen", ptr)]
    attributes = bind("XGetWindowAttributes", integer, ptr, ul, c.POINTER(Attributes))
    atom = bind("XInternAtom", ul, ptr, c.c_char_p, integer)
    query = bind("XQueryTree", integer, ptr, ul, c.POINTER(ul), c.POINTER(ul), c.POINTER(c.POINTER(ul)), c.POINTER(c.c_uint))
    prop = bind("XGetWindowProperty", integer, ptr, ul, ul, c.c_long, c.c_long, integer, ul, c.POINTER(ul), c.POINTER(integer), c.POINTER(ul), c.POINTER(ul), c.POINTER(ptr))
    free = bind("XFree", integer, ptr)
    focus = bind("XSetInputFocus", integer, ptr, ul, integer, ul)
    flush = bind("XSync", integer, ptr, integer)
    # A window may disappear between read-only enumeration and focusing it.
    callback_type = c.CFUNCTYPE(integer, ptr, ptr)
    @callback_type
    def ignore_raced_window(_display, _event): return 0
    bind("XSetErrorHandler", ptr, callback_type)(ignore_raced_window)
    display = None; test = None
    with (report / "xvfb.log").open("w") as server_log, (report / "native-test.log").open("w") as test_log:
        server = subprocess.Popen([str(xvfb), ":98", "-screen", "0", "1200x850x24", "-nolisten", "tcp", "-ac"], stdout=server_log, stderr=subprocess.STDOUT)
        try:
            deadline = time.monotonic() + 10
            while not display and time.monotonic() < deadline and server.poll() is None:
                display = open_display(b":98")
                if not display: time.sleep(0.05)
            if not display: raise RuntimeError("Owned Xvfb failed to start")
            test = subprocess.Popen([str(binary), "desktop::appearance::tests::native_background_stops_and_settings_save_without_touching_folders", "--exact", "--ignored", "--nocapture", "--test-threads=1"], stdout=test_log, stderr=subprocess.STDOUT,
                env={**os.environ, "DISPLAY": ":98", "GDK_BACKEND": "x11", "GSK_RENDERER": "cairo", "GTK_A11Y": "none", "G_DEBUG": "fatal-criticals", "MIRELAY_APPEARANCE_QA_DIR": str(report)})
            pid_atom = atom(display, b"_NET_WM_PID", 0)
            previous = 0; deadline = time.monotonic() + 45
            while test.poll() is None and time.monotonic() < deadline:
                tree_root, parent, children, count = ul(), ul(), c.POINTER(ul)(), c.c_uint()
                query(display, root_window(display), c.byref(tree_root), c.byref(parent), c.byref(children), c.byref(count))
                candidate = 0
                try:
                    for window in children[:count.value]:
                        kind, fmt, length, remaining, data = ul(), integer(), ul(), ul(), ptr()
                        prop(display, window, pid_atom, 0, 1, 0, 6, c.byref(kind), c.byref(fmt), c.byref(length), c.byref(remaining), c.byref(data))
                        if data:
                            try:
                                if length.value == 1 and fmt.value == 32 and c.cast(data, c.POINTER(ul))[0] == test.pid:
                                    attr = Attributes()
                                    if attributes(display, window, c.byref(attr)) and attr.map_state == 2 and attr.width >= 200 and attr.height >= 200:
                                        candidate = window
                            finally: free(data)
                finally:
                    if children: free(children)
                if candidate and candidate != previous:
                    focus(display, candidate, 1, 0); flush(display, 0)
                previous = candidate
                time.sleep(0.02)
            if test.poll() is None: raise TimeoutError("Native appearance test timed out")
            test_log.flush()
            print((report / "native-test.log").read_text(), flush=True)
            if test.returncode: raise RuntimeError(f"Native test exited {test.returncode}")
        finally:
            if test is not None and test.poll() is None:
                test.terminate(); test.wait(timeout=5)
            if display: close_display(display)
            server.terminate(); server.wait(timeout=5)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Opt-in real-process scrolling diagnostic. Use only a dedicated Xvfb display."""
import argparse
import ctypes as c
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--registry", required=True, type=Path)
    parser.add_argument("--isolated-display", required=True)
    parser.add_argument("--more-clicks", type=int, default=0, choices=range(0, 11))
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[2]
    registry = args.registry.resolve(strict=True)
    # Never drive the real desktop or use a production registry.
    if not registry.is_relative_to(repo / "target"):
        parser.error("registry must be a disposable fixture under this repository's target/")
    if args.isolated_display in (":0", ":1") or not args.isolated_display.startswith(":"):
        parser.error("use a dedicated local Xvfb display other than :0/:1")
    x = c.CDLL("libX11.so.6")
    xt = c.CDLL("libXtst.so.6")

    def bind(lib, name, result, *params):
        fn = getattr(lib, name)
        fn.restype, fn.argtypes = result, params
        return fn

    ptr, ul, integer = c.c_void_p, c.c_ulong, c.c_int
    open_display = bind(x, "XOpenDisplay", ptr, c.c_char_p)
    close_display = bind(x, "XCloseDisplay", integer, ptr)
    root_window = bind(x, "XDefaultRootWindow", ul, ptr)
    class Attributes(c.Structure):
        _fields_ = [(name, integer) for name in ("x", "y", "width", "height", "border_width", "depth")] + [
            ("visual", ptr), ("root", ul), ("window_class", integer), ("bit_gravity", integer),
            ("win_gravity", integer), ("backing_store", integer), ("backing_planes", ul),
            ("backing_pixel", ul), ("save_under", integer), ("colormap", ul),
            ("map_installed", integer), ("map_state", integer), ("all_event_masks", c.c_long),
            ("your_event_mask", c.c_long), ("do_not_propagate_mask", c.c_long),
            ("override_redirect", integer), ("screen", ptr)]
    attributes = bind(x, "XGetWindowAttributes", integer, ptr, ul, c.POINTER(Attributes))
    errors = []
    error_callback = c.CFUNCTYPE(integer, ptr, ptr)
    @error_callback
    def on_error(_display, _event):
        errors.append("X11 protocol error")
        return 0
    bind(x, "XSetErrorHandler", ptr, error_callback)(on_error)
    atom = bind(x, "XInternAtom", ul, ptr, c.c_char_p, integer)
    query = bind(x, "XQueryTree", integer, ptr, ul, c.POINTER(ul), c.POINTER(ul), c.POINTER(c.POINTER(ul)), c.POINTER(c.c_uint))
    prop = bind(x, "XGetWindowProperty", integer, ptr, ul, ul, c.c_long, c.c_long, integer, ul, c.POINTER(ul), c.POINTER(integer), c.POINTER(ul), c.POINTER(ul), c.POINTER(ptr))
    free = bind(x, "XFree", integer, ptr)
    focus = bind(x, "XSetInputFocus", integer, ptr, ul, integer, ul)
    raise_window = bind(x, "XRaiseWindow", integer, ptr, ul)
    warp = bind(x, "XWarpPointer", integer, ptr, ul, ul, integer, integer, c.c_uint, c.c_uint, integer, integer)
    flush = bind(x, "XSync", integer, ptr, integer)
    button = bind(xt, "XTestFakeButtonEvent", integer, ptr, c.c_uint, integer, ul)
    get_image = bind(x, "XGetImage", ptr, ptr, ul, integer, integer, c.c_uint, c.c_uint, ul, integer)
    pixel = bind(x, "XGetPixel", ul, ptr, integer, integer)
    destroy_image = bind(x, "XDestroyImage", integer, ptr)
    display = open_display(args.isolated_display.encode())
    if not display:
        raise RuntimeError("Cannot connect to isolated display")
    child = None
    try:
        protected = [p for p in registry.parent.rglob("*") if p.is_file() and p.suffix in (".toml", ".json", ".png")]
        before_files = {p: hashlib.sha256(p.read_bytes()).digest() for p in protected}
        child = subprocess.Popen([str(repo / "target/debug/mirelay-desktop"), "--new-instance", "--registry", str(registry)],
                                 env={**os.environ, "DISPLAY": args.isolated_display, "GDK_BACKEND": "x11", "GSK_RENDERER": "cairo", "G_DEBUG": "fatal-criticals"})
        pid_atom = atom(display, b"_NET_WM_PID", 0)
        window = None
        until = time.monotonic() + 10
        while not window and time.monotonic() < until:
            root, parent, children, count = ul(), ul(), c.POINTER(ul)(), c.c_uint()
            query(display, root_window(display), c.byref(root), c.byref(parent), c.byref(children), c.byref(count))
            try:
                for candidate in children[:count.value]:
                    kind, fmt, length, remaining, data = ul(), integer(), ul(), ul(), ptr()
                    prop(display, candidate, pid_atom, 0, 1, 0, 6, c.byref(kind), c.byref(fmt), c.byref(length), c.byref(remaining), c.byref(data))
                    if data:
                        if length.value == 1 and fmt.value == 32 and c.cast(data, c.POINTER(ul))[0] == child.pid:
                            attr = Attributes()
                            if attributes(display, candidate, c.byref(attr)) and attr.map_state == 2 and attr.width >= 800 and attr.height >= 480:
                                window = candidate
                        free(data)
            finally:
                if children:
                    free(children)
            time.sleep(0.05)
        if not window:
            raise RuntimeError("Child window was not found by its exact PID")
        raise_window(display, window)
        focus(display, window, 1, 0)
        warp(display, 0, window, 0, 0, 0, 0, 600, 400)
        flush(display, 0)
        assert not errors, errors
        time.sleep(3)

        def signature(left=300, top=280, width=400, height=200):
            image = get_image(display, window, left, top, width, height, ul(-1).value, 2)
            if not image:
                raise RuntimeError("Cannot read child-window pixels")
            try:
                return tuple(pixel(image, i, j) for i in range(0, width, 4) for j in range(0, height, 4))
            finally:
                destroy_image(image)

        def sample(label, seconds):
            def ticks():
                fields = Path(f"/proc/{child.pid}/stat").read_text().rsplit(")", 1)[1].split()
                return int(fields[11]) + int(fields[12])
            before, start = ticks(), time.monotonic()
            time.sleep(seconds)
            elapsed = time.monotonic() - start
            assert child.poll() is None, "Desktop exited during interaction"
            cpu = (ticks() - before) / os.sysconf("SC_CLK_TCK") / elapsed * 100
            rss = next(line.split()[1] for line in Path(f"/proc/{child.pid}/status").read_text().splitlines() if line.startswith("VmRSS:"))
            print("ALBUM_X11 " + json.dumps({"stage": label, "seconds": elapsed, "cpu_percent_one_core": cpu, "rss_kib": int(rss)}), flush=True)

        if args.more_clicks:
            attr = Attributes()
            assert attributes(display, window, c.byref(attr))
            middle, bottom = (attr.width + 234) // 2, attr.height - 38
            footer = signature(middle - 70, bottom - 12, 140, 24)
            for _ in range(args.more_clicks):
                warp(display, 0, window, 0, 0, 0, 0, middle, bottom)
                button(display, 1, 1, 0)
                button(display, 1, 0, 0)
                flush(display, 0)
                time.sleep(0.4)
            warp(display, 0, window, 0, 0, 0, 0, 600, 400)
            flush(display, 0)
            time.sleep(2)
            assert signature(middle - 70, bottom - 12, 140, 24) != footer, "Show More did not disappear; load all pages of a known fixture"
            print(f"ALBUM_X11 more_clicks={args.more_clicks} footer_changed=verified", flush=True)
        sample("before_scroll", 2)
        first = signature()
        for step in range(240):
            # Traverse far enough to recycle cells, not merely oscillate inside
            # GTK's prebound viewport buffer.
            direction = 5 if step < 120 else 4
            button(display, direction, 1, 0)
            button(display, direction, 0, 0)
            flush(display, 0)
            time.sleep(0.03)
            if step == 19:
                assert signature() != first, "Injected scrolling did not change the rendered photo wall"
        # Cover the delayed overlay-scrollbar fade in old-path comparisons too.
        sample("immediately_after_scroll", 4)
        time.sleep(10)
        sample("settled_after_scroll", 3)
        assert all(hashlib.sha256(p.read_bytes()).digest() == value for p, value in before_files.items()), "Fixture files or configuration changed"
        assert not errors, errors
        print("ALBUM_X11 scroll_events=240 pixels_changed=verified originals_unchanged=verified", flush=True)
    finally:
        if child is not None:
            child.terminate()
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
        close_display(display)


if __name__ == "__main__":
    main()

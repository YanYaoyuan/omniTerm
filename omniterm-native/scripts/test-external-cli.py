#!/usr/bin/env python3
"""Exercise the actual native window and Node CLI in an isolated Linux desktop.

No real profiles, credentials, model services or remote assets are used.
Requires Xvfb, X11 tools, Node.js and a built native application.
"""
import argparse
import ctypes
import json
import os
from pathlib import Path
import re
import select
import shutil
import subprocess
import sys
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    root = Path(__file__).resolve().parents[2]
    parser.add_argument("--binary", type=Path, default=root / "omniterm-native/target/release/omniterm-app")
    parser.add_argument("--xvfb", default=shutil.which("Xvfb"))
    parser.add_argument("--report", type=Path)
    parser.add_argument("--screenshot", type=Path)
    args = parser.parse_args()
    if sys.platform != "linux" or not args.xvfb:
        parser.error("This isolated GUI test needs Linux and --xvfb PATH")
    cli = root / "omniterm-cli/bin/omniterm.js"
    with tempfile.TemporaryDirectory(prefix="omniterm-external-cli-") as temporary:
        folder = Path(temporary)
        env = os.environ.copy()
        env.pop("OMNITERM_DISABLE_AUTOMATION", None)
        env["XDG_CONFIG_HOME"] = str(folder / "config")
        env["OMNITERM_AUTOMATION_DIR"] = str(folder / "discovery")
        config = folder / "config/omniTerm"
        env["OMNITERM_DATA_DIR"] = str(config)
        config.mkdir(parents=True)
        (config / "native-settings.json").write_text(json.dumps({
            "restoreWorkspace": False, "sshConfigPrompt": False,
            "localShell": {"program": "/bin/bash", "args": ["--noprofile", "--norc", "-i"]},
        }))
        read_fd, write_fd = os.pipe()
        app = xvfb = None
        with (folder / "window.log").open("wb") as log:
            try:
                xvfb = subprocess.Popen([args.xvfb, "-displayfd", str(write_fd), "-screen", "0", "1440x940x24", "-nolisten", "tcp"], pass_fds=(write_fd,), stdout=log, stderr=log)
                os.close(write_fd)
                ready, _, _ = select.select([read_fd], [], [], 10)
                assert ready, "Xvfb did not report a display"
                number = os.read(read_fd, 32).decode().strip()
                os.close(read_fd)
                assert number.isdigit(), "Invalid Xvfb display"
                env["DISPLAY"] = ":" + number
                env["WINIT_UNIX_BACKEND"] = "x11"
                env.pop("WAYLAND_DISPLAY", None)
                app = subprocess.Popen([str(args.binary.resolve())], env=env, cwd=folder, stdin=subprocess.DEVNULL, stdout=log, stderr=log)

                def call(*arguments, expected=0):
                    result = subprocess.run(["node", str(cli), "session", *map(str, arguments)], env=env, capture_output=True, text=True, timeout=15)
                    assert result.returncode == expected, (arguments, result.returncode, result.stdout, result.stderr)
                    return json.loads(result.stdout)

                deadline = time.monotonic() + 30
                while True:
                    assert app.poll() is None, (folder / "window.log").read_text(errors="replace")
                    instances = call("instances")["instances"]
                    if instances:
                        listing = call("list")
                        if listing["terminals"]:
                            break
                    assert time.monotonic() < deadline, "Native window did not expose a session"
                    time.sleep(0.15)
                terminal = listing["terminals"][0]
                session, epoch = terminal["ref"]["sessionId"], terminal["ref"]["epoch"]
                target = [session, "--epoch", epoch]
                call("setup", *target, "--shell", "bash")
                deadline = time.monotonic() + 10
                while True:
                    listing = call("list")
                    if listing["terminals"][0]["interaction"]["state"] == "ready":
                        break
                    assert time.monotonic() < deadline, listing
                    time.sleep(0.1)
                command = "printf 'NATIVE_CLI_RESULT_42\\n'; false"
                result = call("exec", *target, "--request-id", "gui-check", "--command", command, expected=3)
                assert result["operation"]["status"] == "completed", result
                assert result["operation"]["exitCode"] == 1, result
                assert "NATIVE_CLI_RESULT_42" in "\n".join(result["output"]["lines"]), result
                again = call("exec", *target, "--request-id", "gui-check", "--command", command, expected=3)
                assert again["operation"]["operationId"] == result["operation"]["operationId"]
                read = call("read", *target)
                delta = call("read", *target, "--since", read["seqEnd"])
                assert delta["lines"] == [], delta
                binding = folder / "terminal-target.json"
                call("bind", *target, "--out", binding)
                assert "token" not in binding.read_text()
                bound = call("info", "--target", binding)
                assert bound["ref"]["sessionId"] == session
                call("wait", "--target", binding, "--until", "ready", "--wait-ms", "1000")
                found = call("search", "--target", binding, "--query", "NATIVE_CLI_RESULT_42")
                assert found["matches"], found
                long = call("exec", *target, "--request-id", "long-check", "--command", "sleep 10", "--wait-ms", "0", expected=2)
                assert long["operation"]["status"] in ("queued", "running"), long
                cancelled = call("cancel", "long-check", expected=1)
                assert cancelled["operation"]["status"] in ("cancelled", "interrupted"), cancelled
                if args.screenshot:
                    from PIL import ImageGrab
                    args.screenshot.parent.mkdir(parents=True, exist_ok=True)
                    time.sleep(0.3)
                    ImageGrab.grab(xdisplay=env["DISPLAY"]).save(args.screenshot)

                # Close this test process's window normally, checking bridge cleanup.
                listing = subprocess.check_output(["xwininfo", "-root", "-tree"], env=env, text=True)
                window = None
                for candidate in re.findall(r"0x[0-9a-fA-F]+", listing):
                    prop = subprocess.run(["xprop", "-id", candidate, "_NET_WM_PID"], env=env, text=True, capture_output=True).stdout
                    if re.search(r"= " + str(app.pid) + r"\s*$", prop):
                        window = int(candidate, 16)
                        break
                assert window, "Test native window was not found"
                close_window(env["DISPLAY"], window)
                assert app.wait(timeout=10) == 0
                assert not list((folder / "discovery").glob("*.json")), "Discovery file survived normal close"
                report = {"passed": True, "sessionKind": terminal["kind"], "checks": ["native-window-discovery", "setup", "tracked-exec", "nonzero-exit-code", "output", "idempotency", "incremental-read", "target-binding", "readiness-wait", "output-search", "long-command-cancel", "normal-shutdown-cleanup"]}
                if args.report:
                    args.report.parent.mkdir(parents=True, exist_ok=True)
                    args.report.write_text(json.dumps(report, indent=2) + "\n")
                print(json.dumps(report))
            finally:
                if app and app.poll() is None:
                    app.terminate()
                    try:
                        app.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        app.kill()
                        app.wait()
                if xvfb and xvfb.poll() is None:
                    xvfb.terminate()
                    xvfb.wait(timeout=5)


def close_window(display_name, window):
    x11 = ctypes.CDLL("libX11.so.6")
    x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
    x11.XOpenDisplay.restype = ctypes.c_void_p
    x11.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
    x11.XInternAtom.restype = ctypes.c_ulong
    x11.XSendEvent.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_long, ctypes.c_void_p]
    x11.XFlush.argtypes = [ctypes.c_void_p]
    x11.XCloseDisplay.argtypes = [ctypes.c_void_p]

    class Data(ctypes.Union):
        _fields_ = [("l", ctypes.c_long * 5)]

    class Message(ctypes.Structure):
        _fields_ = [("type", ctypes.c_int), ("serial", ctypes.c_ulong), ("send_event", ctypes.c_int), ("display", ctypes.c_void_p), ("window", ctypes.c_ulong), ("message_type", ctypes.c_ulong), ("format", ctypes.c_int), ("data", Data)]

    class Event(ctypes.Union):
        _fields_ = [("client", Message), ("padding", ctypes.c_long * 24)]

    display = x11.XOpenDisplay(display_name.encode())
    assert display
    event = Event()
    event.client.type = 33
    event.client.display = display
    event.client.window = window
    event.client.message_type = x11.XInternAtom(display, b"WM_PROTOCOLS", 0)
    event.client.format = 32
    event.client.data.l[0] = x11.XInternAtom(display, b"WM_DELETE_WINDOW", 0)
    x11.XSendEvent(display, window, 0, 0, ctypes.byref(event))
    x11.XFlush(display)
    x11.XCloseDisplay(display)


if __name__ == "__main__":
    main()

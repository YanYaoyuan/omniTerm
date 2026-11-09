#!/usr/bin/env python3
# Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
"""Linux end-to-end SSH password/OTP test using isolated OS keyring and X server.
Requires: python3-paramiko, python3-secretstorage, gnome-keyring, libxtst6, xvfb.
Build the native release executable first. OMNITERM_APP / OMNITERM_XVFB override paths.
No personal keyring, SSH configuration, company servers or accounts are used.
"""

import os
import sys
import json
import pathlib
import tempfile
import subprocess
import socket
import threading
import time
import ctypes
import ctypes.util
import shutil
import paramiko
import secretstorage


if os.environ.get("OMNITERM_KEYRING_TEST") != "1":
    with tempfile.TemporaryDirectory(prefix="omniterm-isolated-vault-") as isolated:
        env = os.environ.copy()
        env.update(
            OMNITERM_KEYRING_TEST="1",
            XDG_DATA_HOME=isolated + "/data",
            XDG_RUNTIME_DIR=isolated + "/run",
        )
        pathlib.Path(env["XDG_RUNTIME_DIR"]).mkdir(mode=0o700)
        raise SystemExit(
            subprocess.run(
                ["dbus-run-session", "--", sys.executable, __file__], env=env
            ).returncode
        )

REPO = pathlib.Path(__file__).resolve().parents[2]
APP = pathlib.Path(
    os.environ.get("OMNITERM_APP", REPO / "omniterm-native/target/release/omniterm-app")
).resolve()
XVFB = pathlib.Path(
    os.environ.get("OMNITERM_XVFB", shutil.which("Xvfb") or "/usr/bin/Xvfb")
).resolve()
assert APP.is_file() and XVFB.is_file(), (
    "Build the app and install Xvfb, or set OMNITERM_APP / OMNITERM_XVFB"
)
PASS = "fixturepassword938"
OTPS = ["371682", "825139"]
DIRECT_SFTP = os.environ.get("OMNITERM_SFTP_AUTH_TEST") == "1"
LOGIN_USER = "fixture/192.0.2.10/root" if DIRECT_SFTP or os.environ.get("OMNITERM_SSH_TEST_ROUTED") == "1" else "fixture"


class WindowAttributes(ctypes.Structure):
    """Xlib's XWindowAttributes, including native pointer/long alignment."""

    _fields_ = [
        ("x", ctypes.c_int), ("y", ctypes.c_int),
        ("width", ctypes.c_int), ("height", ctypes.c_int),
        ("border_width", ctypes.c_int), ("depth", ctypes.c_int),
        ("visual", ctypes.c_void_p), ("root", ctypes.c_ulong),
        ("window_class", ctypes.c_int), ("bit_gravity", ctypes.c_int),
        ("win_gravity", ctypes.c_int), ("backing_store", ctypes.c_int),
        ("backing_planes", ctypes.c_ulong), ("backing_pixel", ctypes.c_ulong),
        ("save_under", ctypes.c_int), ("colormap", ctypes.c_ulong),
        ("map_installed", ctypes.c_int), ("map_state", ctypes.c_int),
        ("all_event_masks", ctypes.c_long), ("your_event_mask", ctypes.c_long),
        ("do_not_propagate_mask", ctypes.c_long), ("override_redirect", ctypes.c_int),
        ("screen", ctypes.c_void_p),
    ]


def until(fn, label, timeout=25):
    stop = time.monotonic() + timeout
    while time.monotonic() < stop:
        answer = fn()
        if answer:
            return answer
        time.sleep(0.05)
    raise AssertionError("Timed out: " + label)


class Server(paramiko.ServerInterface):
    def __init__(self, attempt):
        self.stage = 0
        self.attempt = attempt
        self.done = threading.Event()

    def get_allowed_auths(self, username):
        return "keyboard-interactive"

    def check_auth_interactive(self, username, submethods):
        assert username == LOGIN_USER
        return paramiko.InteractiveQuery(
            "Password authentication", "", ("Password:", False)
        )

    def check_auth_interactive_response(self, responses):
        if self.stage == 0:
            if responses != [PASS]:
                return paramiko.AUTH_FAILED
            self.stage = 1
            return paramiko.InteractiveQuery("", "", ("2nd Password:", False))
        return (
            paramiko.AUTH_SUCCESSFUL
            if responses == [OTPS[self.attempt]]
            else paramiko.AUTH_FAILED
        )

    def check_channel_request(self, kind, chanid):
        return (
            paramiko.OPEN_SUCCEEDED
            if kind == "session"
            else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED
        )

    def check_channel_exec_request(self, channel, command):
        def finish():
            channel.send(b"fixture-asset-connected\n")
            channel.send_exit_status(0)
            channel.close()
            self.done.set()

        threading.Thread(target=finish, daemon=True).start()
        return True


with tempfile.TemporaryDirectory(prefix="omniterm-password-test-") as tmp:
    root = pathlib.Path(tmp)
    env = os.environ.copy()
    readfd, writefd = os.pipe()
    xvfb = subprocess.Popen(
        [
            str(XVFB),
            "-displayfd",
            str(writefd),
            "-screen",
            "0",
            "1000x650x24",
            "-nolisten",
            "tcp",
        ],
        pass_fds=[writefd],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    os.close(writefd)
    with os.fdopen(readfd) as f:
        display = ":" + f.readline().strip()
    env["DISPLAY"] = display
    daemon = subprocess.Popen(
        [
            "gnome-keyring-daemon",
            "--foreground",
            "--unlock",
            "--components=secrets",
            "--control-directory",
            str(root / "keyring"),
        ],
        env=env,
        stdin=subprocess.PIPE,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    daemon.stdin.write(b"fixture-private-keyring")
    daemon.stdin.close()
    time.sleep(0.5)
    subprocess.run(
        [
            "gnome-keyring-daemon",
            "--start",
            "--components=secrets",
            "--control-directory",
            str(root / "keyring"),
        ],
        env=env,
        check=True,
        stdout=subprocess.DEVNULL,
    )
    oldenv = {
        k: os.environ.get(k) for k in ["XDG_DATA_HOME", "XDG_RUNTIME_DIR", "DISPLAY"]
    }
    for k in oldenv:
        os.environ[k] = env[k]
    conn = secretstorage.dbus_init()
    until(lambda: list(secretstorage.get_all_collections(conn)), "private keyring")
    x = ctypes.CDLL(ctypes.util.find_library("X11"))
    xt = ctypes.CDLL(ctypes.util.find_library("Xtst"))
    x.XOpenDisplay.argtypes = [ctypes.c_char_p]
    x.XOpenDisplay.restype = ctypes.c_void_p
    d = x.XOpenDisplay(display.encode())
    assert d
    x.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
    x.XDefaultRootWindow.restype = ctypes.c_ulong
    r = x.XDefaultRootWindow(d)
    x.XQueryTree.argtypes = [
        ctypes.c_void_p,
        ctypes.c_ulong,
        ctypes.POINTER(ctypes.c_ulong),
        ctypes.POINTER(ctypes.c_ulong),
        ctypes.POINTER(ctypes.POINTER(ctypes.c_ulong)),
        ctypes.POINTER(ctypes.c_uint),
    ]
    x.XFetchName.argtypes = [
        ctypes.c_void_p,
        ctypes.c_ulong,
        ctypes.POINTER(ctypes.c_char_p),
    ]
    x.XFree.argtypes = [ctypes.c_void_p]
    x.XGetWindowAttributes.argtypes = [
        ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(WindowAttributes)
    ]
    x.XSetInputFocus.argtypes = [
        ctypes.c_void_p,
        ctypes.c_ulong,
        ctypes.c_int,
        ctypes.c_ulong,
    ]
    x.XStringToKeysym.argtypes = [ctypes.c_char_p]
    x.XStringToKeysym.restype = ctypes.c_ulong
    x.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
    x.XKeysymToKeycode.restype = ctypes.c_uint
    x.XFlush.argtypes = [ctypes.c_void_p]
    x.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
    x.XInternAtom.restype = ctypes.c_ulong
    x.XGetWindowProperty.argtypes = [
        ctypes.c_void_p,
        ctypes.c_ulong,
        ctypes.c_ulong,
        ctypes.c_long,
        ctypes.c_long,
        ctypes.c_int,
        ctypes.c_ulong,
        ctypes.POINTER(ctypes.c_ulong),
        ctypes.POINTER(ctypes.c_int),
        ctypes.POINTER(ctypes.c_ulong),
        ctypes.POINTER(ctypes.c_ulong),
        ctypes.POINTER(ctypes.POINTER(ctypes.c_ubyte)),
    ]

    def window_pid(w):
        atom = x.XInternAtom(d, b"_NET_WM_PID", 0)
        typ = ctypes.c_ulong()
        fmt = ctypes.c_int()
        n = ctypes.c_ulong()
        rest = ctypes.c_ulong()
        data = ctypes.POINTER(ctypes.c_ubyte)()
        x.XGetWindowProperty(
            d,
            w,
            atom,
            0,
            1,
            0,
            6,
            ctypes.byref(typ),
            ctypes.byref(fmt),
            ctypes.byref(n),
            ctypes.byref(rest),
            ctypes.byref(data),
        )
        if not data:
            return None
        value = ctypes.cast(data, ctypes.POINTER(ctypes.c_ulong))[0]
        x.XFree(data)
        return value

    xt.XTestFakeKeyEvent.argtypes = [
        ctypes.c_void_p,
        ctypes.c_uint,
        ctypes.c_int,
        ctypes.c_ulong,
    ]

    def windows():
        rr = ctypes.c_ulong()
        parent = ctypes.c_ulong()
        children = ctypes.POINTER(ctypes.c_ulong)()
        n = ctypes.c_uint()
        x.XQueryTree(
            d,
            r,
            ctypes.byref(rr),
            ctypes.byref(parent),
            ctypes.byref(children),
            ctypes.byref(n),
        )
        result = []
        for i in range(n.value):
            attributes = WindowAttributes()
            # A window can already have a title while GPU/window setup has not
            # mapped it yet. Focusing it then produces X11 BadMatch in CI.
            if not x.XGetWindowAttributes(d, children[i], ctypes.byref(attributes)) or attributes.map_state != 2:
                continue
            name = ctypes.c_char_p()
            if x.XFetchName(d, children[i], ctypes.byref(name)) and name.value:
                if "omniTerm" in name.value.decode(errors="replace"):
                    result.append(children[i])
                x.XFree(name)
        if children:
            x.XFree(children)
        return result

    def enter(window, text):
        x.XSetInputFocus(d, window, 1, 0)
        for key in list(text) + ["Return"]:
            code = x.XKeysymToKeycode(d, x.XStringToKeysym(key.encode()))
            assert code
            xt.XTestFakeKeyEvent(d, code, 1, 0)
            xt.XTestFakeKeyEvent(d, code, 0, 0)
        x.XFlush(d)
        time.sleep(0.8)

    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen()
    port = listener.getsockname()[1]
    host = paramiko.RSAKey.generate(2048)
    known = root / "known_hosts"
    known.write_text(
        "[127.0.0.1]:%d %s %s\n" % (port, host.get_name(), host.get_base64())
    )
    transports = []
    errors = []

    def serve():
        try:
            for attempt in range(2):
                client, _ = listener.accept()
                t = paramiko.Transport(client)
                transports.append(t)
                t.add_server_key(host)
                server = Server(attempt)
                t.start_server(server=server)
                c = t.accept(30)
                if c is None:
                    raise AssertionError("No authenticated channel")
                server.done.wait(10)
        except Exception as e:
            errors.append(repr(e))

    threading.Thread(target=serve, daemon=True).start()
    try:
        window_counts = []
        for attempt in range(2):
            authdir = root / ("auth-" + str(attempt))
            authdir.mkdir(mode=0o700)
            runenv = env.copy()
            runenv.update(
                OMNITERM_SSH_ASKPASS="1",
                OMNITERM_SSH_AUTH_SCOPE='["fixture-loopback",null,2222,null,null]',
                OMNITERM_SSH_AUTH_STATE=str(authdir / "used"),
                SSH_ASKPASS=str(APP),
                SSH_ASKPASS_REQUIRE="force",
            )
            if DIRECT_SFTP:
                runenv.pop("OMNITERM_SSH_ASKPASS")
                scope = runenv.pop("OMNITERM_SSH_AUTH_SCOPE")
                runenv.pop("OMNITERM_SSH_AUTH_STATE")
                (authdir / "scope.json").write_text(json.dumps({
                    "version": 1, "scope": "direct-sftp-fixture",
                    "host": "127.0.0.1", "user": LOGIN_USER, "port": port,
                    "account": {"scope": scope, "remember": True},
                }))
                runenv.update(OMNITERM_QIZHI_ASKPASS="1", OMNITERM_QIZHI_AUTH_STATE=str(authdir))
            log = (root / ("ssh-" + str(attempt) + ".log")).open("wb")
            ssh = subprocess.Popen(
                [
                    "ssh",
                    "-F",
                    "/dev/null",
                    "-T",
                    "-p",
                    str(port),
                    "-o",
                    "IdentityAgent=none",
                    "-o",
                    "PreferredAuthentications=keyboard-interactive",
                    "-o",
                    "NumberOfPasswordPrompts=1",
                    "-o",
                    "StrictHostKeyChecking=yes",
                    "-o",
                    "UserKnownHostsFile=" + str(known),
                    "-o",
                    "User=" + LOGIN_USER,
                    "127.0.0.1",
                    "fixture-exec",
                ],
                env=runenv,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=log,
            )
            count = 0
            if attempt == 0:
                w = until(
                    lambda: next(iter(windows()), None), "first fixed-password window"
                )
                time.sleep(0.6)

                old_pid = window_pid(w)
                assert old_pid
                enter(w, PASS)
                count += 1
                until(
                    lambda: not pathlib.Path("/proc/" + str(old_pid)).exists(),
                    "first password helper exited",
                )
            w = until(lambda: next(iter(windows()), None), "manual OTP window")
            time.sleep(0.6)

            enter(w, OTPS[attempt])
            count += 1
            output, _ = ssh.communicate(timeout=25)
            log.close()
            if ssh.returncode != 0:
                raise AssertionError(
                    "SSH failed: "
                    + (root / ("ssh-" + str(attempt) + ".log")).read_text()
                )
            assert output == b"fixture-asset-connected\n", output
            if DIRECT_SFTP:
                assert not (authdir / "credential").exists(), "Direct SFTP must not need an exported credential"
            until(lambda: not windows(), "OTP window closed")
            window_counts.append(count)
            print(
                "PASS real OpenSSH + keyboard-interactive login %d: %d manual windows"
                % (attempt + 1, count),
                flush=True,
            )
        items = [
            item
            for collection in secretstorage.get_all_collections(conn)
            for item in collection.get_all_items()
        ]
        assert len(items) == 1, len(items)
        payload = items[0].get_secret()
        assert PASS.encode() in payload
        assert all(otp.encode() not in payload for otp in OTPS), (
            "OTP must not be persisted"
        )
        assert window_counts == [2, 1]
        assert not errors, errors
        print(
            "PASS password persisted only in isolated OS keyring; OTPs absent; second login skips fixed-password window",
            flush=True,
        )
    finally:
        for t in transports:
            t.close()
        listener.close()
        daemon.terminate()
        xvfb.terminate()
        daemon.wait(timeout=5)
        xvfb.wait(timeout=5)
        for k, v in oldenv.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v

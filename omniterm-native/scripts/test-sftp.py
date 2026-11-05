#!/usr/bin/env python3
# Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
"""Run SFTP regression tests against a private, rate-limited loopback sshd.

Usage: python3 scripts/test-sftp.py -- cargo test --locked sftp_end_to_end -- --ignored --nocapture
Also serves as the fixture's SFTP subsystem. No personal SSH config or keys are used.
"""

import json
import os
from pathlib import Path
import pwd
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time


def process_start(pid):
    try:
        return Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[19]
    except (OSError, IndexError):
        return None


def subsystem(server, root):
    child = subprocess.Popen([server, "-d", root], stdin=subprocess.PIPE, stdout=subprocess.PIPE, bufsize=0)
    parent = os.getppid()
    parent_start = process_start(parent)
    # Record only fixture-owned process IDs, with start times to guard against PID reuse.
    with (Path(root) / "processes.jsonl").open("a") as log:
        for pid in [os.getpid(), parent, child.pid]:
            log.write(json.dumps([pid, process_start(pid)]) + "\n")
    control = Path(os.environ["OMNITERM_SFTP_CONTROL"])

    def disconnect():
        while child.poll() is None:
            if control.exists():
                child.kill()
                try:
                    if parent_start and process_start(parent) == parent_start:
                        os.kill(parent, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                return
            time.sleep(0.01)

    def relay(source, destination):
        try:
            while chunk := os.read(source, 16384):
                time.sleep(len(chunk) / (1024 * 1024))
                view = memoryview(chunk)
                while view:
                    view = view[os.write(destination, view):]
        except OSError:
            pass

    threading.Thread(target=disconnect, daemon=True).start()
    threading.Thread(target=relay, args=(0, child.stdin.fileno()), daemon=True).start()
    try:
        relay(child.stdout.fileno(), 1)
    finally:
        if child.poll() is None:
            child.terminate()
        child.wait(timeout=5)


def run(command):
    sshd = shutil.which("sshd") or "/usr/sbin/sshd"
    server = next((path for path in ["/usr/lib/openssh/sftp-server", "/usr/lib/ssh/sftp-server"] if Path(path).is_file()), None)
    if not Path(sshd).is_file() or server is None:
        raise RuntimeError("Install openssh-server before running the SFTP fixture")
    username = pwd.getpwuid(os.getuid()).pw_name
    with tempfile.TemporaryDirectory(prefix="omniterm-sftp-e2e-") as temporary:
        root = Path(temporary)
        # sshd's Subsystem setting has its own command parser: its generated paths have
        # no whitespace, even when this repository's checkout path does.
        proxy = root / "proxy.py"
        shutil.copyfile(__file__, proxy)
        for name in ["host_key", "client_key"]:
            subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(root / name)], check=True)
        shutil.copyfile(root / "client_key.pub", root / "authorized_keys")
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        public = (root / "host_key.pub").read_text().split()
        (root / "known_hosts").write_text(f"[127.0.0.1]:{port} {public[0]} {public[1]}\n")
        config = root / "sshd_config"
        config.write_text(f"""Port {port}
ListenAddress 127.0.0.1
HostKey {root}/host_key
PidFile {root}/sshd.pid
AuthorizedKeysFile {root}/authorized_keys
StrictModes no
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
UsePAM no
PermitRootLogin no
AllowUsers {username}
AcceptEnv OMNITERM_SFTP_CONTROL
Subsystem sftp {sys.executable} {proxy} --subsystem {server} {root}
LogLevel ERROR
""")
        subprocess.run([sshd, "-t", "-f", str(config)], check=True)
        fixture = root / "fixture.json"
        fixture.write_text(json.dumps({"root": str(root), "port": port, "key": str(root / "client_key"), "known_hosts": str(root / "known_hosts"), "destination": f"{username}@127.0.0.1"}))
        with (root / "sshd.log").open("w+") as log:
            service = subprocess.Popen([sshd, "-D", "-e", "-f", str(config)], stdout=log, stderr=log)
            try:
                deadline = time.monotonic() + 5
                while True:
                    try:
                        with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                            break
                    except OSError:
                        if time.monotonic() >= deadline or service.poll() is not None:
                            log.seek(0)
                            raise RuntimeError("Fixture sshd failed: " + log.read())
                        time.sleep(0.05)
                env = os.environ.copy()
                env["OMNITERM_SFTP_E2E"] = str(fixture)
                print("SFTP fixture: isolated loopback SSH, private keys, 1 MiB/s in each direction", flush=True)
                result = subprocess.run(command, env=env, timeout=180).returncode
                if result:
                    log.seek(0)
                    print("Fixture sshd diagnostics:\n" + log.read(), file=sys.stderr)
                return result
            finally:
                if service.poll() is None:
                    service.terminate()
                service.wait(timeout=5)
                registry = root / "processes.jsonl"
                for line in registry.read_text().splitlines() if registry.exists() else []:
                    pid, start = json.loads(line)
                    if start and process_start(pid) == start:
                        try:
                            os.kill(pid, signal.SIGTERM)
                        except ProcessLookupError:
                            pass


if __name__ == "__main__":
    if sys.argv[1:2] == ["--subsystem"]:
        subsystem(sys.argv[2], sys.argv[3])
    else:
        command = sys.argv[1:]
        if command[:1] == ["--"]:
            command = command[1:]
        sys.exit(run(command or ["cargo", "test", "--locked", "sftp_end_to_end", "--", "--ignored", "--nocapture"]))

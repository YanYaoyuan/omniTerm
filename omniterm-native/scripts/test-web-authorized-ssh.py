#!/usr/bin/env python3
# Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
"""Isolated SSH server checks web password, session env and absence of companions.

Usage: OMNITERM_WEB_TEST_APP=/path/to/omniterm-app python3 test-web-authorized-ssh.py -- cargo test ...
Only synthetic credentials and loopback networking are used; no personal files.
"""
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import threading
import time
import paramiko


class Server(paramiko.ServerInterface):
    def __init__(self):
        self.session = False
        self.finished = threading.Event()
        self.password_attempts = 0

    def get_allowed_auths(self, username):
        return "password"

    def check_auth_password(self, username, password):
        self.password_attempts += 1
        return paramiko.AUTH_SUCCESSFUL if username == "alice" and password == "fixture-credential" else paramiko.AUTH_FAILED

    def check_channel_request(self, kind, channel):
        return paramiko.OPEN_SUCCEEDED if kind == "session" else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED

    def check_channel_pty_request(self, *args):
        return True

    def check_channel_env_request(self, channel, name, value):
        if name == b"SHTERM_WEBSID" and value == b"fixture-session-reference":
            self.session = True
            return True
        return False

    def check_channel_shell_request(self, channel):
        def finish():
            time.sleep(0.2)
            channel.send(b"fixture-web-asset-connected\r\n" if self.session else b"missing-web-session\r\n")
            channel.send_exit_status(0 if self.session else 1)
            channel.close()
            self.finished.set()
        threading.Thread(target=finish, daemon=True).start()
        return True


with tempfile.TemporaryDirectory(prefix="omniterm-web-ssh-fixture-") as temporary:
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen()
    listener.settimeout(30)
    port = listener.getsockname()[1]
    key = paramiko.RSAKey.generate(2048)
    known = Path(temporary) / "known_hosts"
    known.write_text(f"[127.0.0.1]:{port} {key.get_name()} {key.get_base64()}\n")
    errors = []
    server = Server()

    def serve():
        try:
            client, _ = listener.accept()
            transport = paramiko.Transport(client)
            transport.add_server_key(key)
            transport.start_server(server=server)
            channel = transport.accept(20)
            assert channel is not None
            assert server.finished.wait(15)
            time.sleep(0.2)
            transport.close()
        except Exception as error:
            errors.append(type(error).__name__)

    thread = threading.Thread(target=serve, daemon=True)
    thread.start()
    command = sys.argv[1:]
    if command and command[0] == "--":
        command = command[1:]
    result = subprocess.run(command, env={**os.environ,"OMNITERM_WEB_TEST_PORT":str(port),"OMNITERM_WEB_TEST_KNOWN_HOSTS":str(known)})
    thread.join(timeout=3)
    listener.close()
    assert not errors, errors
    assert server.finished.is_set() and server.session
    assert server.password_attempts == 1
    raise SystemExit(result.returncode)

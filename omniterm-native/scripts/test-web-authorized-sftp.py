#!/usr/bin/env python3
# Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
"""Web SFTP fixture: one password, one subsystem, isolated upload/download root.

Run a native ignored test after --. No company endpoints or personal SSH files.
"""
import errno
import base64
import ctypes as C
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import re
import zlib
import paramiko


class Files(paramiko.SFTPServerInterface):
    def __init__(self, server, root):
        super().__init__(server)
        self.server = server
        self.root = Path(root).resolve()

    def local(self, path):
        resolved = (self.root / path.lstrip("/")).resolve()
        if not resolved.is_relative_to(self.root):
            raise OSError(errno.EACCES, "outside fixture root")
        return resolved

    def list_folder(self, path):
        try:
            self.server.listings += 1
            result = []
            for item in self.local(path).iterdir():
                attr = paramiko.SFTPAttributes.from_stat(item.stat())
                attr.filename = item.name
                result.append(attr)
            return result
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno)

    def stat(self, path):
        try:
            return paramiko.SFTPAttributes.from_stat(self.local(path).stat())
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno)

    lstat = stat

    def open(self, path, flags, attr):
        try:
            if flags & (os.O_WRONLY | os.O_RDWR):
                self.server.writes += 1
            fd = os.open(self.local(path), flags, 0o600)
            mode = "r+b" if flags & os.O_RDWR else "wb" if flags & os.O_WRONLY else "rb"
            stream = os.fdopen(fd, mode)
            handle = SlowHandle(flags) if 'slow.bin' in path else paramiko.SFTPHandle(flags)
            handle.readfile = stream
            handle.writefile = stream
            return handle
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno)

    def rename(self, old, new):
        try:
            os.replace(self.local(old), self.local(new))
            return paramiko.SFTP_OK
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno)

    posix_rename = rename

    def mkdir(self, path, attr):
        try:
            self.local(path).mkdir()
            return paramiko.SFTP_OK
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno)

    def rmdir(self, path):
        try:
            self.local(path).rmdir()
            return paramiko.SFTP_OK
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno)

    def remove(self, path):
        try:
            self.local(path).unlink()
            return paramiko.SFTP_OK
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno)


class SlowHandle(paramiko.SFTPHandle):
    def write(self, offset, data):
        time.sleep(0.015)
        return super().write(offset, data)


class Server(paramiko.ServerInterface):
    def __init__(self):
        self.password_attempts = 0
        self.subsystems = []
        self.unexpected_commands = []
        self.listings = 0
        self.writes = 0

    def get_allowed_auths(self, username):
        return "password"

    def check_auth_password(self, username, password):
        self.password_attempts += 1
        return paramiko.AUTH_SUCCESSFUL if username == "alice" and password == "fixture-credential" else paramiko.AUTH_FAILED

    def check_channel_request(self, kind, channel):
        return paramiko.OPEN_SUCCEEDED if kind == "session" else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED

    def check_channel_subsystem_request(self, channel, name):
        self.subsystems.append(name)
        return super().check_channel_subsystem_request(channel, name)

    def check_channel_shell_request(self, channel):
        self.unexpected_commands.append("shell")
        return False

    def check_channel_exec_request(self, channel, command):
        self.unexpected_commands.append("exec")
        return False


def gui(root, port, server, env):
    from PIL import ImageGrab
    readfd, writefd = os.pipe()
    xvfb = subprocess.Popen([os.environ['OMNITERM_XVFB'], '-displayfd', str(writefd), '-screen', '0', '1600x1000x24', '-nolisten', 'tcp'], pass_fds=(writefd,), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    os.close(writefd)
    with os.fdopen(readfd) as stream:
        display = ':' + stream.readline().strip()
    env = {**env, 'DISPLAY': display, 'OMNITERM_DATA_DIR': str(root/'app-data'), 'OMNITERM_AUTOMATION_DIR': str(root/'automation')}
    env.pop('OMNITERM_DISABLE_AUTOMATION', None)
    packet = {'app':'winscp','host':'127.0.0.1','port':str(port),'user':'alice','pwd':'fixture-credential','PWD':'never-use-uppercase','st':'Web SFTP fixture','path':'/','proxy.addr':None}
    wire = 'accessclient://' + base64.b64encode(zlib.compress(json.dumps(packet).encode())).decode()
    with (root/'gui.stderr').open('wb') as log:
        app = subprocess.Popen([env['OMNITERM_WEB_TEST_APP'],'--open-accessclient'], env=env, stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=log)
    app.stdin.write(wire.encode())
    app.stdin.close()
    try:
        deadline = time.monotonic()+20
        while server.listings < 1:
            assert app.poll() is None, 'SFTP window exited'
            assert time.monotonic()<deadline, 'SFTP window did not list the fixture directory'
            time.sleep(.1)
        if os.environ.get('OMNITERM_WEB_CLI') == '1':
            cli_checks(root, server, env)
        time.sleep(1)
        screenshot = Path(os.environ['OMNITERM_WEB_GUI_SCREENSHOT'])
        screenshot.parent.mkdir(parents=True, exist_ok=True)
        ImageGrab.grab(xdisplay=display).save(screenshot)
        x = C.CDLL('libX11.so.6')
        x.XOpenDisplay.argtypes=[C.c_char_p];x.XOpenDisplay.restype=C.c_void_p
        x.XInternAtom.argtypes=[C.c_void_p,C.c_char_p,C.c_int];x.XInternAtom.restype=C.c_ulong
        x.XSendEvent.argtypes=[C.c_void_p,C.c_ulong,C.c_int,C.c_long,C.c_void_p]
        x.XFlush.argtypes=[C.c_void_p];x.XCloseDisplay.argtypes=[C.c_void_p]
        d=x.XOpenDisplay(display.encode());assert d
        class Data(C.Union):
            _fields_=[('l',C.c_long*5)]
        class Message(C.Structure):
            _fields_=[('type',C.c_int),('serial',C.c_ulong),('send_event',C.c_int),('display',C.c_void_p),('window',C.c_ulong),('message_type',C.c_ulong),('format',C.c_int),('data',Data)]
        class Event(C.Union):
            _fields_=[('client',Message),('padding',C.c_long*24)]
        listing=subprocess.check_output(['xwininfo','-root','-tree'],env=env,text=True)
        sent=False
        for wid in dict.fromkeys(re.findall(r'0x[0-9a-fA-F]+',listing)):
            owner=subprocess.run(['xprop','-id',wid,'_NET_WM_PID'],env=env,text=True,capture_output=True).stdout
            if not re.search(r'= '+str(app.pid)+r'\s*$',owner):continue
            event=Event();event.client.type=33;event.client.display=d;event.client.window=int(wid,16)
            event.client.message_type=x.XInternAtom(d,b'WM_PROTOCOLS',0);event.client.format=32
            event.client.data.l[0]=x.XInternAtom(d,b'WM_DELETE_WINDOW',0)
            x.XSendEvent(d,int(wid,16),0,0,C.byref(event));sent=True
        x.XFlush(d);x.XCloseDisplay(d)
        assert sent, 'SFTP window was not mapped'
        code=app.wait(timeout=10)
        assert code==0
        print('PASS native web SFTP window: directory listed, screenshot saved, normal close',flush=True)
        return subprocess.CompletedProcess([],code)
    finally:
        if app.poll() is None:
            app.terminate();app.wait(timeout=5)
        xvfb.terminate();xvfb.wait(timeout=5)


def cli_checks(root, server, env):
    cli = Path(__file__).resolve().parents[2] / 'omniterm-cli/bin/omniterm.js'
    sequence = 0

    def call(action, *arguments, expected=(0,), group='files', request=None):
        nonlocal sequence
        arguments = list(map(str, arguments))
        if action in ('ls', 'stat', 'mkdir', 'rename', 'remove', 'upload', 'download'):
            sequence += 1
            arguments += ['--request-id', request or f'fixture-{sequence}']
        result = subprocess.run(['node', str(cli), group, action, *arguments], env=env, text=True, capture_output=True, timeout=35)
        assert result.returncode in expected, (action, result.returncode, result.stdout, result.stderr)
        return json.loads(result.stdout or result.stderr)

    sessions = call('list')['fileSessions']
    assert len(sessions) == 1, sessions
    reference = sessions[0]['ref']
    identity, generation = reference['fileSessionId'], reference['generation']
    assert call('list', group='session')['terminals'] == [], 'SFTP window must not create a shell'
    binding = root/'file-target.json'
    call('bind', identity, '--generation', generation, '--out', binding)
    assert 'fixture-credential' not in binding.read_text()
    target = ['--target', str(binding)]
    assert call('info', identity, '--generation', 'stale', expected=(1,))['error']
    call('mkdir', *target, '--path', '/CLI 目录', request='mkdir-once')
    call('mkdir', *target, '--path', '/CLI 目录', request='mkdir-once')
    call('remove', *target, '--path', '/CLI 目录', expected=(1,), request='mkdir-once')
    page = call('ls', *target, '--path', '/', '--limit', 1)
    assert page['operation']['result']['truncated'] and len(page['operation']['result']['entries']) == 1, page
    call('ls', *target, '--path', '/CLI 目录')
    assert call('info', *target)['listing']['path'] == '/CLI 目录', 'CLI navigation must update the actual page'
    source = root/'upload with spaces.bin'
    source.write_bytes(b'SFTP CLI exact bytes\0\xff\n')
    upload_args = [*target, '--local', source, '--remote', '/CLI 目录/upload.bin']
    before = server.writes
    uploaded = call('upload', *upload_args, request='upload-once')
    call('upload', *upload_args, request='upload-once')
    assert server.writes == before + 1, 'Repeated request uploaded twice'
    assert uploaded['operation']['bytes'] == source.stat().st_size, uploaded
    queue = call('transfers', *target)['transfers']
    assert any(item['transferId'] == uploaded['operation']['transferId'] for item in queue), queue
    failed = call('upload', *upload_args, expected=(1,))
    assert failed['operation']['status'] == 'failed'
    downloaded = root/'downloaded.bin'
    call('download', *target, '--remote', '/CLI 目录/upload.bin', '--local', downloaded)
    assert downloaded.read_bytes() == source.read_bytes()
    downloaded.write_bytes(b'keep local original')
    call('download', *target, '--remote', '/CLI 目录/upload.bin', '--local', downloaded, expected=(1,))
    assert downloaded.read_bytes() == b'keep local original'
    call('download', *target, '--remote', '/CLI 目录/upload.bin', '--local', downloaded, '--overwrite')
    assert downloaded.read_bytes() == source.read_bytes()
    call('rename', *target, '--path', '/CLI 目录/upload.bin', '--to', '/CLI 目录/renamed.bin')
    metadata = call('stat', *target, '--path', '/CLI 目录/renamed.bin')['operation']['result']
    assert metadata['file'] and metadata['size'] == source.stat().st_size
    call('remove', *target, '--path', '/CLI 目录/renamed.bin')
    call('remove', *target, '--path', '/CLI 目录', '--directory')
    deadline = time.monotonic() + 5
    while call('info', *target)['listing']['path'] != '/':
        assert time.monotonic() < deadline, 'Removing the displayed folder must navigate its page to the parent'
        time.sleep(.05)
    tree = root/'tree'
    (tree/'nested').mkdir(parents=True)
    (tree/'nested/中文.txt').write_bytes(b'nested fixture')
    call('upload', *target, '--local', tree, '--remote', '/tree', expected=(1,))
    call('upload', *target, '--local', tree, '--remote', '/tree', '--recursive')
    call('download', *target, '--remote', '/tree', '--local', root/'tree-copy', '--recursive')
    assert (root/'tree-copy/nested/中文.txt').read_bytes() == b'nested fixture'
    slow = root/'slow.bin'
    slow.write_bytes(b'x' * (8 * 1024 * 1024))
    started = call('upload', *target, '--local', slow, '--remote', '/slow.bin', '--wait-ms', 0, expected=(2,), request='slow-upload')
    assert started['operation']['status'] in ('running', 'queued')
    call('cancel-transfer', *target, '--transfer-id', started['operation']['transferId'], expected=(1, 2))
    cancelled = call('result', 'slow-upload', '--wait-ms', 30000, expected=(1,))
    assert cancelled['operation']['status'] == 'cancelled', cancelled
    assert not (root/'remote/slow.bin').exists()
    assert not list((root/'remote').rglob('*.part')), 'Transfer left a partial behind'
    print('PASS external SFTP CLI: exact page binding, navigation/pagination, mkdir/rename/remove, byte-identical transfers, no-overwrite, recursive directories, deduplication, cancellation, no shell/exec', flush=True)


with tempfile.TemporaryDirectory(prefix="omniterm-web-sftp-fixture-") as temporary:
    root = Path(temporary)
    files = root / "remote"
    files.mkdir()
    (files / "fixture.txt").write_bytes(b"web-file-fixture")
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen()
    listener.settimeout(30)
    port = listener.getsockname()[1]
    key = paramiko.RSAKey.generate(2048)
    known = root / "known_hosts"
    known.write_text(f"[127.0.0.1]:{port} {key.get_name()} {key.get_base64()}\n")
    tools = root / "bin"
    tools.mkdir()
    ssh = shutil.which("ssh")
    wrapper = tools / "ssh"
    wrapper.write_text("#!/usr/bin/python3\nimport os,sys\nos.environ['SSH_ASKPASS']=os.environ['OMNITERM_WEB_TEST_APP']\nos.execv(" + repr(ssh) + ",['ssh','-o'," + repr("UserKnownHostsFile=" + str(known)) + "]+sys.argv[1:])\n")
    wrapper.chmod(0o700)
    server = Server()
    errors = []

    def serve():
        try:
            client, _ = listener.accept()
            transport = paramiko.Transport(client)
            transport.add_server_key(key)
            transport.set_subsystem_handler("sftp", paramiko.SFTPServer, Files, str(files))
            transport.start_server(server=server)
            channel = transport.accept(20)
            assert channel is not None
            deadline = time.monotonic() + 40
            while transport.is_active() and time.monotonic() < deadline:
                time.sleep(0.05)
            transport.close()
        except Exception as error:
            errors.append(type(error).__name__)

    thread = threading.Thread(target=serve, daemon=True)
    thread.start()
    command = sys.argv[1:]
    if command and command[0] == "--":
        command = command[1:]
    env={**os.environ,"PATH":str(tools)+os.pathsep+os.environ['PATH'],"OMNITERM_WEB_TEST_PORT":str(port)}
    if command == ['--cli-gui']:
        env['OMNITERM_WEB_CLI'] = '1'
        os.environ['OMNITERM_WEB_CLI'] = '1'
    result = gui(root,port,server,env) if command in (['--gui'], ['--cli-gui']) else subprocess.run(command,env=env)
    thread.join(timeout=5)
    listener.close()
    assert not errors, errors
    assert server.password_attempts == 1, server.password_attempts
    assert server.subsystems == ["sftp"], server.subsystems
    assert not server.unexpected_commands, server.unexpected_commands
    if command not in (['--gui'], ['--cli-gui']):
        assert (files / "web-upload.txt").read_bytes() == b"web-sftp-fixture-round-trip"
    raise SystemExit(result.returncode)

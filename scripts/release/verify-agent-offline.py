#!/usr/bin/env python3
"""Exercise the bundled code-mode host's real stdio protocol and V8, offline."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import queue
import struct
import subprocess
import tempfile
import threading
import time


MAX_FRAME = 1024 * 1024


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def verify(package: Path):
    manifest = json.loads((package / 'codex-package.json').read_text(encoding='utf-8'))
    suffix = '.exe' if os.name == 'nt' else ''
    executable = package / 'bin' / f'codex-code-mode-host{suffix}'
    require(executable.is_file(), 'The complete Agent package must contain code-mode-host')
    with tempfile.TemporaryDirectory(prefix='omniterm-host-smoke-') as scratch:
        # Preserve only OS loader/runtime requirements, never credentials, proxy
        # configuration, model settings, shell startup hooks, or Node injections.
        env = {name: os.environ[name] for name in ('SystemRoot', 'SYSTEMROOT', 'WINDIR', 'COMSPEC', 'PATHEXT') if name in os.environ}
        env.update({'PATH': os.defpath, 'TEMP': scratch, 'TMP': scratch, 'TMPDIR': scratch, 'LANG': 'C.UTF-8'})
        with tempfile.TemporaryFile() as errors:
            process = subprocess.Popen([str(executable), '--listen', 'stdio'], cwd=scratch, env=env,
                                       stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errors, bufsize=0)
            messages = queue.Queue()

            def read_exact(length):
                result = bytearray()
                while len(result) < length:
                    chunk = process.stdout.read(length - len(result))
                    if not chunk:
                        raise EOFError('Code-mode host closed stdout')
                    result.extend(chunk)
                return bytes(result)

            def read_frames():
                try:
                    while True:
                        length = struct.unpack('<I', read_exact(4))[0]
                        require(0 < length <= MAX_FRAME, f'Invalid code-mode frame length: {length}')
                        messages.put(json.loads(read_exact(length).decode('utf-8')))
                except Exception as error:
                    messages.put(error)

            # select() cannot wait on Windows anonymous pipe handles. A bounded
            # reader thread + queue gives the same deadline on all three OSes.
            reader = threading.Thread(target=read_frames, daemon=True)
            reader.start()

            def send(message):
                payload = json.dumps(message).encode('utf-8')
                process.stdin.write(struct.pack('<I', len(payload)) + payload)
                process.stdin.flush()

            def receive(kind, identifier=None):
                deadline = time.monotonic() + 30
                for _ in range(64):
                    remaining = deadline - time.monotonic()
                    require(remaining > 0, f'Timed out waiting for {kind}')
                    try:
                        message = messages.get(timeout=remaining)
                    except queue.Empty as error:
                        raise RuntimeError(f'Timed out waiting for {kind}') from error
                    if isinstance(message, Exception):
                        raise message
                    require(message.get('type') != 'connection/rejected', f'Handshake rejected: {message}')
                    result = message.get('result', {})
                    require(result.get('status') != 'error', f'Host operation failed: {message}')
                    if message.get('type') == kind and (identifier is None or message.get('id') == identifier):
                        return message
                raise RuntimeError('Too many unrelated host frames')

            def request(identifier, method, **fields):
                send({'type': 'operation/request', 'id': identifier,
                      'request': {'method': method, 'sessionId': 'omniterm-release-smoke', **fields}})

            try:
                send({'type': 'connection/hello', 'supportedVersions': [1], 'requiredCapabilities': [], 'optionalCapabilities': []})
                require(receive('connection/ready').get('selectedVersion') == 1, 'Unexpected protocol version')
                request(1, 'session/open')
                require(receive('operation/response', 1)['result']['value']['type'] == 'session/ready', 'Session did not open')
                request(2, 'session/execute', request={'tool_call_id': 'offline-release-smoke', 'enabled_tools': [],
                        'source': 'text("OMNITERM_CODE_MODE_HOST_" + (6 * 7));', 'yield_time_ms': 10000, 'max_output_tokens': 256})
                require(receive('operation/response', 2)['result']['value']['type'] == 'execution/started', 'Execution did not start')
                execution = receive('execute/initialResponse', 2)['result']['value'].get('Result')
                require(execution is not None and not execution.get('error_text'), f'V8 execution failed: {execution}')
                output = ''.join(item.get('text', '') for item in execution['content_items'])
                require('OMNITERM_CODE_MODE_HOST_42' in output, f'Wrong V8 result: {output!r}')
                request(3, 'session/shutdown')
                require(receive('operation/response', 3)['result']['value']['type'] == 'session/closed', 'Session did not close')
                process.stdin.close()
                require(process.wait(timeout=10) == 0, 'Code-mode host did not exit successfully')
                print(json.dumps({'passed': True, 'target': manifest['target'], 'protocolVersion': 1,
                                  'v8Result': 'OMNITERM_CODE_MODE_HOST_42', 'cleanExit': True,
                                  'modelRequestAttempted': False, 'toolsEnabled': False}))
            except Exception as error:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=3)
                errors.seek(0)
                detail = errors.read().decode('utf-8', errors='replace')[-4000:]
                raise RuntimeError(f'{error}\n{detail}') from error
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=3)
                if not process.stdin.closed:
                    process.stdin.close()
                reader.join(timeout=2)
                process.stdout.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('package', type=Path, nargs='?', default=Path(__file__).resolve().parents[2] / 'dist/omniterm-agent')
    args = parser.parse_args()
    verify(args.package.resolve(strict=True))


if __name__ == '__main__':
    main()

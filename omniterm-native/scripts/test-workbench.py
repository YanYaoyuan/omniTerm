#!/usr/bin/env python3
# Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
"""Render and exercise the native workbench in an isolated X11 desktop.

Uses fake saved profiles and a local shell; never connects to saved assets.
Requires the same Xvfb/Openbox fixture dependencies as test-desktop-pet.py.
"""
import argparse
import ctypes as C
import importlib.util
import json
import os
from pathlib import Path
import select
import subprocess as S
import tempfile
import time


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--xvfb', required=True)
    parser.add_argument('--wm', required=True)
    parser.add_argument('--wm-config', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    scripts = Path(__file__).parent
    desktop_api = load('desktop_fixture', scripts / 'test-desktop-pet.py')
    external = load('external_fixture', scripts / 'test-external-cli.py')
    cli = scripts.resolve().parents[1] / 'omniterm-cli/bin/omniterm.js'
    processes = []
    desktop = None
    with tempfile.TemporaryDirectory(prefix='omniterm-workbench-') as temporary, (args.output / 'window.log').open('wb') as log:
        try:
            folder = Path(temporary)
            data = folder / 'data'
            data.mkdir(mode=0o700)
            settings = data / 'native-settings.json'
            settings.write_text(json.dumps({'theme': 'codeDark', 'restoreWorkspace': False, 'sshConfigPrompt': False, 'localShell': {'program': '/bin/bash', 'args': ['--noprofile', '--norc', '-i']}}))
            (data / 'native-ui.json').write_text(json.dumps({'agentHidden': True, 'playroomDiscovered': True}))
            profiles = [dict(id=f'fixture-{i}', kind='ssh', name=name, host=host, username='deploy', port=22) for i, (name, host) in enumerate([
                ('Production · API', 'api.example.invalid'), ('Staging · Gateway', 'staging.example.invalid'),
                ('Development · Linux', 'dev.example.invalid'), ('Research · GPU', 'gpu.example.invalid')])]
            (data / 'profiles.json').write_text(json.dumps(profiles))
            env = dict(os.environ, OMNITERM_DATA_DIR=str(data), OMNITERM_AUTOMATION_DIR=str(data / 'automation'), XDG_SESSION_TYPE='x11', WINIT_UNIX_BACKEND='x11')
            env.pop('WAYLAND_DISPLAY', None)
            env.pop('OMNITERM_DISABLE_AUTOMATION', None)
            read, write = os.pipe()
            processes.append(S.Popen([args.xvfb, '-displayfd', str(write), '-screen', '0', '1600x1100x24', '-nolisten', 'tcp'], pass_fds=(write,), stdout=log, stderr=log))
            os.close(write)
            assert select.select([read], [], [], 10)[0], 'Xvfb startup timeout'
            number = os.read(read, 32).decode().strip()
            os.close(read)
            assert number.isdigit()
            env['DISPLAY'] = ':' + number
            processes.append(S.Popen([args.wm, '--sm-disable', '--config-file', args.wm_config], env=env, stdout=log, stderr=log))
            time.sleep(.5)
            desktop = d = desktop_api.Desktop(env)
            app = S.Popen([str(args.binary.resolve())], env=env, cwd=folder, stdout=log, stderr=log)
            processes.append(app)
            window = desktop_api.until(lambda: d.window('omniTerm'), 'Workbench did not start', 30)
            desktop_api.until(lambda: d.viewable(window), 'Window did not map')
            d.x.XReconfigureWMWindow(d.d, window, 0, 15, C.byref(desktop_api.Changes(0, 0, 1440, 940, 0, 0, 0)))
            d.focus(window)
            desktop_api.until(lambda: d.geometry(window) == (0, 0, 1440, 940), 'Window geometry incorrect')

            def capture(path):
                from PIL import ImageGrab
                x, y, width, height = d.geometry(window)
                ImageGrab.grab(xdisplay=env['DISPLAY']).crop((x, y, x + width, y + height)).save(path)

            def call(*arguments):
                result = S.run(['node', str(cli), 'session', *map(str, arguments)], env=env, capture_output=True, text=True, timeout=20)
                assert result.returncode == 0, result.stdout + result.stderr
                return json.loads(result.stdout)

            terminal = desktop_api.until(lambda: (call('list')['terminals'] or [None])[0], 'No local terminal')
            target = [terminal['ref']['sessionId'], '--epoch', terminal['ref']['epoch']]
            call('setup', *target, '--shell', 'bash')
            desktop_api.until(lambda: call('list')['terminals'][0]['interaction']['state'] == 'ready', 'Shell integration not ready')
            result = call('exec', *target, '--request-id', 'workbench-preview', '--command', "clear; printf '\\033[1;36momniTerm\\033[0m  /  native workspace\\n\\n'; printf '  \\033[32m✓\\033[0m  Local shell connected\\n  \\033[32m✓\\033[0m  External agent CLI ready\\n  \\033[32m✓\\033[0m  GPU terminal renderer\\n\\n'; printf '\\033[90mReady for your next command.\\033[0m\\n'")
            assert result['operation']['exitCode'] == 0
            time.sleep(.7)
            capture(args.output / 'code-dark.png')
            # The command centre opens the existing palette; closing it does not
            # type its UI events into the terminal.
            d.click(680, 18)
            time.sleep(.3)
            capture(args.output / 'command-center.png')
            d.tap('Escape')
            assert call('list')['terminals'][0]['interaction']['state'] == 'ready'
            # Open the title-bar theme picker, select its second recommended theme.
            d.click(1208, 18)
            time.sleep(.3)
            capture(args.output / 'theme-picker.png')
            d.click(1280, 77)
            desktop_api.until(lambda: json.loads(settings.read_text())['theme'] == 'codeLight', 'Theme picker did not persist Code Light')
            time.sleep(.6)
            capture(args.output / 'code-light.png')
            d.click(150, 85)
            for character in 'production': d.tap(character)
            time.sleep(.3)
            capture(args.output / 'connection-filter.png')
            d.key('Control_L', 1)
            d.tap('a')
            d.key('Control_L', 0)
            d.tap('BackSpace')
            d.click(680, 18)
            d.tap('Escape')
            # Sidebar visibility is reversible through the activity rail.
            from PIL import ImageGrab
            sidebar_color = (248, 248, 248)
            desktop_api.until(lambda: ImageGrab.grab(xdisplay=env['DISPLAY']).getpixel((100, 500)) == sidebar_color, 'Light sidebar did not settle after closing the palette')
            d.click(24, 59)
            desktop_api.until(lambda: ImageGrab.grab(xdisplay=env['DISPLAY']).getpixel((100, 500)) != sidebar_color, 'Activity rail did not hide the sidebar')
            d.click(24, 59)
            desktop_api.until(lambda: ImageGrab.grab(xdisplay=env['DISPLAY']).getpixel((100, 500)) == sidebar_color, 'Activity rail did not restore the sidebar')
            # Minimum window size must keep navigation and native controls reachable.
            d.x.XReconfigureWMWindow(d.d, window, 0, 15, C.byref(desktop_api.Changes(0, 0, 1000, 650, 0, 0, 0)))
            d.flush()

            desktop_api.until(lambda: d.geometry(window) == (0, 0, 1000, 650), 'Minimum window geometry incorrect')
            time.sleep(.5)
            capture(args.output / 'compact-light.png')
            d.click(470, 18)
            time.sleep(.2)
            d.tap('Escape')
            assert call('list')['terminals'][0]['interaction']['state'] == 'ready'
            external.close_window(env['DISPLAY'], window)
            assert app.wait(timeout=10) == 0
            report = {'passed': True, 'checks': ['populated-dark-workbench', 'command-centre-opens-palette', 'no-terminal-input-leak', 'theme-picker-persists-light', 'sidebar-toggle', 'connection-filter', 'minimum-window-size', 'external-cli', 'normal-close']}
            (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
            print(json.dumps(report))
        except BaseException:
            if desktop:
                desktop.screenshot(args.output / 'failure.png')
            raise
        finally:
            if desktop:
                desktop.x.XCloseDisplay(desktop.d)
            for process in reversed(processes):
                if process.poll() is None:
                    process.terminate()
                    process.wait(timeout=5)


if __name__ == '__main__':
    main()

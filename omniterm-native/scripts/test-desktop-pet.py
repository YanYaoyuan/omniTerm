#!/usr/bin/env python3
"""Test the real desktop companion in a newly created isolated X11 desktop.

Requires Xvfb, xcompmgr, Openbox, X11/XTest, Pillow, Node and a built app.
The native window manager verifies stacking, focus and actual minimization.
"""
import argparse
import ctypes as C
import importlib.util
import json
import math
import os
from pathlib import Path
import re
import select
import shutil
import subprocess as S
import tempfile
import time


class Changes(C.Structure):
    _fields_ = [(n, C.c_int) for n in ('x', 'y', 'width', 'height', 'border_width')] + [('sibling', C.c_ulong), ('stack_mode', C.c_int)]


class MessageData(C.Union):
    _fields_ = [('l', C.c_long * 5)]


class Message(C.Structure):
    _fields_ = [('type', C.c_int), ('serial', C.c_ulong), ('send_event', C.c_int), ('display', C.c_void_p), ('window', C.c_ulong), ('message_type', C.c_ulong), ('format', C.c_int), ('data', MessageData)]


class Event(C.Union):
    _fields_ = [('client', Message), ('padding', C.c_long * 24)]


class Desktop:
    def __init__(self, env):
        self.env = env
        self.x = C.CDLL('libX11.so.6')
        self.xt = C.CDLL('libXtst.so.6')
        specs = {
            'XSendEvent': ([C.c_void_p, C.c_ulong, C.c_int, C.c_long, C.c_void_p], C.c_int),
            'XInternAtom': ([C.c_void_p, C.c_char_p, C.c_int], C.c_ulong),
            'XOpenDisplay': ([C.c_char_p], C.c_void_p),
            'XDefaultRootWindow': ([C.c_void_p], C.c_ulong),
            'XFlush': ([C.c_void_p], C.c_int),
            'XReconfigureWMWindow': ([C.c_void_p, C.c_ulong, C.c_int, C.c_uint, C.POINTER(Changes)], C.c_int),
            'XSetInputFocus': ([C.c_void_p, C.c_ulong, C.c_int, C.c_ulong], C.c_int),
            'XGetInputFocus': ([C.c_void_p, C.POINTER(C.c_ulong), C.POINTER(C.c_int)], C.c_int),
            'XStringToKeysym': ([C.c_char_p], C.c_ulong),
            'XKeysymToKeycode': ([C.c_void_p, C.c_ulong], C.c_uint),
            'XMapWindow': ([C.c_void_p, C.c_ulong], C.c_int),
            'XRaiseWindow': ([C.c_void_p, C.c_ulong], C.c_int),
            'XIconifyWindow': ([C.c_void_p, C.c_ulong, C.c_int], C.c_int),
            'XCreateSimpleWindow': ([C.c_void_p, C.c_ulong, C.c_int, C.c_int, C.c_uint, C.c_uint, C.c_uint, C.c_ulong, C.c_ulong], C.c_ulong),
            'XStoreName': ([C.c_void_p, C.c_ulong, C.c_char_p], C.c_int),
            'XSelectInput': ([C.c_void_p, C.c_ulong, C.c_long], C.c_int),
            'XPending': ([C.c_void_p], C.c_int),
            'XNextEvent': ([C.c_void_p, C.c_void_p], C.c_int),
            'XCloseDisplay': ([C.c_void_p], C.c_int),
        }
        for name, (arguments, result) in specs.items():
            function = getattr(self.x, name)
            function.argtypes, function.restype = arguments, result
        self.xt.XTestFakeMotionEvent.argtypes = [C.c_void_p, C.c_int, C.c_int, C.c_int, C.c_ulong]
        self.xt.XTestFakeButtonEvent.argtypes = [C.c_void_p, C.c_uint, C.c_int, C.c_ulong]
        self.xt.XTestFakeKeyEvent.argtypes = [C.c_void_p, C.c_uint, C.c_int, C.c_ulong]
        self.d = self.x.XOpenDisplay(env['DISPLAY'].encode())
        assert self.d, 'Cannot open isolated display'

    def flush(self):
        self.x.XFlush(self.d)

    def move(self, x, y):
        self.xt.XTestFakeMotionEvent(self.d, 0, x, y, 0)
        self.flush()

    def click(self, x, y, button=1):
        self.move(x, y)
        time.sleep(.06)
        for pressed in [1, 0]:
            self.xt.XTestFakeButtonEvent(self.d, button, pressed, 0)
            self.flush()
            time.sleep(.07)

    def key(self, name, down):
        code = self.x.XKeysymToKeycode(self.d, self.x.XStringToKeysym(name.encode()))
        self.xt.XTestFakeKeyEvent(self.d, code, down, 0)
        self.flush()

    def tap(self, name):
        self.key(name, 1)
        self.key(name, 0)
        time.sleep(.06)

    def focus(self, window):
        # Ask the window manager to activate AND raise its managed frame.
        # XSetInputFocus / XRaiseWindow on a reparented client cannot do that.
        event = Event()
        event.client.type = 33
        event.client.display = self.d
        event.client.window = window
        event.client.message_type = self.x.XInternAtom(self.d, b'_NET_ACTIVE_WINDOW', 0)
        event.client.format = 32
        event.client.data.l[0] = 2
        self.x.XSendEvent(self.d, self.x.XDefaultRootWindow(self.d), 0, (1 << 20) | (1 << 19), C.byref(event))
        self.flush()
        until(lambda: self.focused() == window, 'Window manager did not activate the test window')
        time.sleep(.1)

    def focused(self):
        window, revert = C.c_ulong(), C.c_int()
        self.x.XGetInputFocus(self.d, C.byref(window), C.byref(revert))
        return window.value

    def window(self, title):
        tree = S.check_output(['xwininfo', '-root', '-tree'], env=self.env, text=True)
        match = re.search(r'(0x\w+) "' + re.escape(title) + '"', tree)
        return int(match[1], 16) if match else None

    def viewable(self, window):
        info = S.check_output(['xwininfo', '-id', hex(window)], env=self.env, text=True)
        return 'Map State: IsViewable' in info

    def geometry(self, window):
        info = S.check_output(['xwininfo', '-id', hex(window)], env=self.env, text=True)
        return tuple(int(re.search(name + r':\s+(-?\d+)', info)[1]) for name in ['Absolute upper-left X', 'Absolute upper-left Y', 'Width', 'Height'])

    def cat_rect(self):
        from PIL import ImageGrab, ImageChops
        image = ImageGrab.grab(xdisplay=self.env['DISPLAY']).convert('RGB')
        mask = None
        for band, value in zip(image.split(), (255, 235, 197)):
            selected = band.point([255 if i == value else 0 for i in range(256)])
            mask = selected if mask is None else ImageChops.multiply(mask, selected)
        rect = mask.getbbox()
        return rect

    def settle(self, pet, point):
        self.move(*point)
        def close_enough():
            rect = self.cat_rect()
            if rect is None:
                return False
            left, top, right, bottom = rect
            return math.dist(((left + right) / 2, (top + bottom) / 2), point) < 135
        until(close_enough, 'Cat did not follow global mouse', 8)
        return self.cat_rect()

    def screenshot(self, path):
        from PIL import ImageGrab
        ImageGrab.grab(xdisplay=self.env['DISPLAY']).save(path)


def until(check, message, seconds=10):
    end = time.monotonic() + seconds
    while True:
        value = check()
        if value:
            return value
        assert time.monotonic() < end, message
        time.sleep(.10)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    root = Path(__file__).resolve().parents[2]
    parser.add_argument('--binary', type=Path, default=root / 'omniterm-native/target/release/omniterm-app')
    parser.add_argument('--xvfb', default=shutil.which('Xvfb'), required=not shutil.which('Xvfb'))
    parser.add_argument('--compositor', default=shutil.which('xcompmgr'), required=not shutil.which('xcompmgr'))
    parser.add_argument('--wm', default=shutil.which('openbox'), required=not shutil.which('openbox'))
    parser.add_argument('--wm-config')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    processes = []
    desktop = None
    with tempfile.TemporaryDirectory(prefix='omniterm-pet-') as temp, (args.output / 'window.log').open('wb') as log:
        try:
            folder = Path(temp)
            data = folder / 'data'
            data.mkdir(mode=0o700)
            env = os.environ.copy()
            env.update(OMNITERM_DATA_DIR=str(data), OMNITERM_AUTOMATION_DIR=str(data / 'automation'), XDG_SESSION_TYPE='x11', WINIT_UNIX_BACKEND='x11')
            env.pop('WAYLAND_DISPLAY', None)
            env.pop('OMNITERM_DISABLE_AUTOMATION', None)
            (data / 'native-settings.json').write_text(json.dumps({'theme': 'github', 'restoreWorkspace': False, 'sshConfigPrompt': False, 'localShell': {'program': '/bin/bash', 'args': ['--noprofile', '--norc', '-i']}}))
            (data / 'native-ui.json').write_text(json.dumps({'agentHidden': True, 'playroomDiscovered': True}))
            read, write = os.pipe()
            processes.append(S.Popen([args.xvfb, '-displayfd', str(write), '-screen', '0', '2000x1100x24', '-nolisten', 'tcp'], pass_fds=(write,), stdout=log, stderr=log))
            os.close(write)
            assert select.select([read], [], [], 10)[0], 'Xvfb startup timeout'
            number = os.read(read, 32).decode().strip()
            os.close(read)
            assert number.isdigit()
            env['DISPLAY'] = ':' + number
            processes.append(S.Popen([args.compositor, '-n'], env=env, stdout=log, stderr=log))
            wm = [args.wm, '--sm-disable']
            if args.wm_config:
                wm += ['--config-file', args.wm_config]
            processes.append(S.Popen(wm, env=env, stdout=log, stderr=log))
            time.sleep(.6)
            desktop = d = Desktop(env)
            under = d.x.XCreateSimpleWindow(d.d, d.x.XDefaultRootWindow(d.d), 0, 0, 2000, 1100, 0, 0, 0xD9E5F2)
            d.x.XStoreName(d.d, under, b'Desktop test application')
            d.x.XSelectInput(d.d, under, 4)  # ButtonPressMask
            d.x.XMapWindow(d.d, under)
            d.flush()
            app = S.Popen([str(args.binary.resolve())], env=env, cwd=folder, stdout=log, stderr=log)
            processes.append(app)
            main_window = until(lambda: d.window('omniTerm'), 'Native app did not start', 30)
            until(lambda: d.viewable(main_window), 'Native window did not map')
            d.x.XReconfigureWMWindow(d.d, main_window, 0, 15, C.byref(Changes(0, 0, 1000, 650, 0, 0, 0)))
            d.focus(main_window)
            until(lambda: d.geometry(main_window) == (0, 0, 1000, 650), 'Window was not resized')
            time.sleep(.6)
            d.key('Control_L', 1)
            d.key('Shift_L', 1)
            d.tap('p')
            d.key('Shift_L', 0)
            d.key('Control_L', 0)
            time.sleep(.2)
            for char in 'byte':
                d.tap(char)
            d.tap('Return')
            time.sleep(.9)
            d.click(320, 540)  # 放到桌面追鼠标, validated at the minimum window size
            pet_title = 'BYTE · omniTerm desktop companion'
            pet = until(lambda: d.window(pet_title), 'Desktop cat was not created')
            assert d.focused() == main_window, 'Cat stole focus at creation'
            props = S.check_output(['xprop', '-id', hex(pet), '_NET_WM_WINDOW_TYPE', '_NET_WM_STATE'], env=env, text=True)
            assert '_NET_WM_WINDOW_TYPE_DOCK' in props and '_NET_WM_STATE_ABOVE' in props, props
            position = d.settle(pet, (1700, 850))
            assert position[0] > 1000 and position[1] > 650, position
            assert d.geometry(pet) == (0, 0, 2000, 1100), 'GPU canvas should stay fixed across the desktop'
            d.focus(under)
            d.x.XRaiseWindow(d.d, under)
            d.flush()
            d.settle(pet, (1500, 950))
            assert d.focused() == under, 'Cat stole focus while following'
            d.x.XIconifyWindow(d.d, main_window, 0)
            d.flush()
            d.settle(pet, (400, 900))
            d.move(1650, 600)
            time.sleep(.8)
            d.screenshot(args.output / 'minimized-follow.png')
            d.settle(pet, (1650, 900))
            # CLI remains usable while the main window is minimized and cat runs.
            def cli(*arguments):
                result = S.run(['node', str(root / 'omniterm-cli/bin/omniterm.js'), 'session', *map(str, arguments)], env=env, capture_output=True, text=True, timeout=20)
                assert result.returncode == 0, result.stderr + result.stdout
                return json.loads(result.stdout)
            terminal = cli('list')['terminals'][0]
            target = [terminal['ref']['sessionId'], '--epoch', terminal['ref']['epoch']]
            cli('setup', *target, '--shell', 'bash')
            until(lambda: cli('list')['terminals'][0]['interaction']['state'] == 'ready', 'Shell integration not ready')
            result = cli('exec', *target, '--request-id', 'desktop-pet-test', '--command', "printf '\\033[2J\\033[H'; printf 'DESKTOP_PET_CLI_OK\\n'")
            assert result['operation']['exitCode'] == 0 and 'DESKTOP_PET_CLI_OK' in '\n'.join(result['output']['lines']), result
            d.x.XMapWindow(d.d, main_window)
            d.flush()
            until(lambda: d.viewable(main_window), 'Native window did not restore')
            d.x.XRaiseWindow(d.d, main_window)
            d.focus(main_window)
            time.sleep(.9)
            d.click(894, 637)  # status-bar cat
            time.sleep(.9)
            d.click(312, 540)  # pause
            time.sleep(.2)
            d.tap('Escape')
            paused = until(d.cat_rect, 'Paused cat is not visible')
            d.move(1800, 150)
            time.sleep(.6)
            assert d.cat_rect() == paused, 'Paused cat moved'
            d.x.XRaiseWindow(d.d, under)
            d.focus(under)
            time.sleep(.2)
            event = (C.c_long * 24)()
            while d.x.XPending(d.d):
                d.x.XNextEvent(d.d, C.byref(event))
            d.click((paused[0] + paused[2]) // 2, (paused[1] + paused[3]) // 2)
            clicked = False
            while d.x.XPending(d.d):
                d.x.XNextEvent(d.d, C.byref(event))
                clicked |= C.cast(event, C.POINTER(C.c_int))[0] == 4 and event[4] == under
            assert clicked and d.focused() == under, 'Cat intercepted click or focus'
            # Alpha-zero corners must reveal the blue application below the cat.
            d.screenshot(args.output / 'click-through.png')
            from PIL import Image
            screenshot = Image.open(args.output / 'click-through.png').convert('RGB')
            assert screenshot.getpixel((1900, 1000)) == (217, 229, 242), 'Opaque GPU canvas background'
            assert d.geometry(pet) == (0, 0, 2000, 1100), 'The canvas moved during the chase'
            d.x.XRaiseWindow(d.d, main_window)
            d.focus(main_window)
            d.click(894, 637)
            time.sleep(.9)
            d.screenshot(args.output / 'resume-controls.png')
            d.click(312, 540)  # resume
            time.sleep(.9)
            d.screenshot(args.output / 'resumed.png')
            d.tap('Escape')
            d.settle(pet, (1700, 850))
            d.move(1350, 700)
            time.sleep(.35)
            d.screenshot(args.output / 'desktop-follow.png')
            d.focus(main_window)
            d.click(894, 637)
            time.sleep(.9)
            d.click(405, 540)  # recall
            until(lambda: not d.window(pet_title), 'Recalled cat remained')
            d.click(320, 540)  # start again, checking cached art and viewport reuse
            until(lambda: d.window(pet_title), 'Cat did not restart')
            module = importlib.util.spec_from_file_location('external_cli', Path(__file__).with_name('test-external-cli.py'))
            external_cli = importlib.util.module_from_spec(module)
            module.loader.exec_module(external_cli)
            external_cli.close_window(env['DISPLAY'], main_window)
            assert app.wait(timeout=10) == 0
            assert not d.window(pet_title), 'Pet survived application exit'
            report = {'passed': True, 'checks': ['global-follow-outside-main-window', 'always-on-top', 'no-focus-stealing', 'follow-while-main-minimized', 'external-cli-while-following', 'pause-resume', 'click-through-to-another-application', 'transparent-background', 'recall-restart', 'normal-exit-closes-pet', 'fixed-gpu-canvas']}
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
                    try:
                        process.wait(timeout=5)
                    except S.TimeoutExpired:
                        process.kill()
                        process.wait()


if __name__ == '__main__':
    main()

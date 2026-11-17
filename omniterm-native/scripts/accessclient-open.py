#!/usr/bin/env python3
# Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
"""Open supported portal SSH/SFTP requests; never execute packet URLs.

The OS supplies its URI to this receiver. Children receive it only on stdin.
Unsupported client types retain the existing offline diagnostic receiver.
"""

import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time


def notify(message):
    if os.environ.get("OMNITERM_NO_NOTIFY") != "1" and shutil.which("zenity"):
        subprocess.run(["zenity", "--error", "--title=omniTerm", "--text=" + message], check=False)


def receive(arguments):
    if len(arguments) != 1 or not arguments[0].startswith("accessclient://"):
        notify("网页启动请求格式无效。")
        return 2
    try:
        uri = arguments[0].encode()
    except UnicodeError:
        notify("网页启动请求编码无效。")
        return 2
    if len(uri) > 4 * 1024 * 1024:
        notify("网页启动请求过大。")
        return 2
    sibling = Path(__file__).resolve().with_name("omniterm-app")
    default = sibling if sibling.is_file() else Path(__file__).resolve().parents[2] / "dist/local-native/omniterm-app"
    app = Path(os.environ.get("OMNITERM_APP", str(default)))
    try:
        inspected = subprocess.run([str(app), "--inspect-accessclient"], input=uri, capture_output=True, timeout=15)
        if inspected.returncode != 0:
            notify("网页启动请求无法解码。")
            return 2
        summary = json.loads(inspected.stdout)
        if not isinstance(summary, dict):
            raise ValueError("invalid inspection result")
        if summary.get("requestedClients") not in (["putty"], ["winscp"]):
            spec = importlib.util.spec_from_file_location("omniterm_accessclient_inspect", Path(__file__).with_name("accessclient-inspect.py"))
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            os.environ["OMNITERM_APP"] = str(app)
            return module.receive(arguments)
        validated = subprocess.run([str(app), "--validate-accessclient"], input=uri, capture_output=True, timeout=15)
        if validated.returncode != 0:
            # No raw packet or child stderr is echoed into notifications or logs.
            notify("这组网页连接参数暂不受支持，未发起连接。请保留 Web 会话，并查看接入说明。")
            return 2
        child = subprocess.Popen([str(app), "--open-accessclient"], stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        try:
            child.stdin.write(uri)
            child.stdin.close()
        except (BrokenPipeError, OSError):
            child.wait(timeout=5)
            notify("OmniTerm 未能读取网页启动请求。")
            return 2
        time.sleep(0.2)
        if child.poll() not in (None, 0):
            notify("OmniTerm 窗口启动失败。")
            return 2
        return 0
    except (OSError, ValueError, subprocess.TimeoutExpired):
        notify("无法启动 OmniTerm，请检查本机安装。")
        return 2


if __name__ == "__main__":
    raise SystemExit(receive(sys.argv[1:]))

#!/usr/bin/env python3
# Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
"""Optional diagnostic URI receiver; no network requests or client execution.

Not registered automatically. A desktop entry can pass one accessclient:// URI
after the user chooses to enable this temporary receiver. By default only a
redacted summary is written. A separate one-shot local capture flag explicitly
requests saving the next SFTP envelope's full JSON. The URI reaches the native
decoder through stdin, not its argv/env.
"""

import base64
import datetime
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import uuid
from urllib.parse import unquote_to_bytes
import zlib

CAPTURE_FLAG = "accessclient-capture-next.json"
PARAMETERS_FILE = "accessclient-connection-parameters.json"
COMPARE_FLAG = "accessclient-compare-next.json"
COMPARISON_FILE = "accessclient-route-comparison.json"


def write_private_json(directory, name, payload):
    target = directory / name
    temporary = directory / (".accessclient-" + uuid.uuid4().hex + ".tmp")
    try:
        fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as output:
            json.dump(payload, output, ensure_ascii=False, indent=2)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, target)
    finally:
        temporary.unlink(missing_ok=True)
    return target


def capture_next(state, uri, summary):
    """Only an explicit local flag may save values; consume it once, for SFTP."""
    if not summary["decodeSucceeded"] or not set(summary.get("requestedClients", [])) & {"winscp", "filezilla", "xftp"}:
        return
    flag = state / CAPTURE_FLAG
    try:
        if flag.stat().st_size > 4096 or json.loads(flag.read_text()).get("enabled") is not True:
            return
    except (OSError, ValueError, AttributeError):
        return
    claim = state / (".accessclient-capture-" + uuid.uuid4().hex + ".json")
    try:
        # Two simultaneous launches must not both consume the same authorization.
        flag.rename(claim)
    except FileNotFoundError:
        return
    try:
        # The native decoder has already validated this exact envelope. Repeat
        # decoding locally to write JSON directly to a private file, never stdout.
        encoded = unquote_to_bytes(uri.strip()[len("accessclient://"):])
        compressed = base64.b64decode(encoded + b"=" * (-len(encoded) % 4), altchars=b"-_", validate=True)
        decoder = zlib.decompressobj()
        decoded = decoder.decompress(compressed, 2 * 1024 * 1024 + 1)
        if len(decoded) > 2 * 1024 * 1024 or not decoder.eof or decoder.unused_data or decoder.unconsumed_tail:
            raise ValueError("invalid compressed envelope")
        parameters = json.loads(decoded.decode("utf-8"))
        if not isinstance(parameters, dict):
            raise ValueError("capture requires one connection object")
        path = write_private_json(state, PARAMETERS_FILE, {
            "schemaVersion": 1,
            "receivedAt": summary["receivedAt"],
            "mode": "user-authorized-local-capture",
            "parameters": parameters,
        })
        summary.update(parametersSaved=True, parametersFile=str(path))
    except (OSError, ValueError, zlib.error):
        summary["captureError"] = "local-parameter-save-failed"
        # Restore the unused authorization without overwriting a newer request.
        try:
            os.link(claim, flag)
        except OSError:
            pass
    finally:
        claim.unlink(missing_ok=True)


def compare_next(state, uri, summary):
    """Compare one SSH launch with local expectations; persist booleans only."""
    if not summary["decodeSucceeded"] or not {client.lower() for client in summary.get("requestedClients", [])} & {"putty", "scrt", "securecrt", "xshell", "mobaxterm", "terminal", "iterm2"}:
        return
    flag = state / COMPARE_FLAG
    try:
        if flag.stat().st_size > 8192:
            return
        config = json.loads(flag.read_text())
        expected = config.get("expected")
        if config.get("enabled") is not True or not isinstance(expected, dict):
            return
        required = {"gatewayHost", "gatewayPort", "routeUser", "gatewayUser", "assetHost", "assetUser"}
        if set(expected) != required or not all(isinstance(v, (str, int)) and not isinstance(v, bool) for v in expected.values()):
            return
    except (OSError, ValueError, AttributeError):
        return
    claim = state / (".accessclient-compare-" + uuid.uuid4().hex + ".json")
    try:
        flag.rename(claim)
    except FileNotFoundError:
        return
    try:
        encoded = unquote_to_bytes(uri.strip()[len("accessclient://"):])
        compressed = base64.b64decode(encoded + b"=" * (-len(encoded) % 4), altchars=b"-_", validate=True)
        decoder = zlib.decompressobj()
        decoded = decoder.decompress(compressed, 2 * 1024 * 1024 + 1)
        if len(decoded) > 2 * 1024 * 1024 or not decoder.eof or decoder.unused_data or decoder.unconsumed_tail:
            raise ValueError("invalid compressed envelope")
        packet = json.loads(decoded.decode("utf-8"))
        if not isinstance(packet, dict):
            raise ValueError("comparison requires one connection object")

        def matches(field, name):
            value = packet.get(field)
            return str(value) == str(expected[name]) if isinstance(value, (str, int)) and not isinstance(value, bool) else None

        comparison = {
            "schemaVersion": 1,
            "receivedAt": summary["receivedAt"],
            "gatewayHostMatches": matches("hn", "gatewayHost"),
            "gatewayPortMatches": matches("pn", "gatewayPort"),
            "transportUserMatchesRoute": matches("un", "routeUser"),
            "transportUserMatchesGatewayUser": matches("un", "gatewayUser"),
            "transportUserMatchesAssetUser": matches("un", "assetUser"),
            "sessionHostFieldMatchesAsset": matches("sh", "assetHost"),
            "sessionAccountFieldMatchesAsset": matches("sa", "assetUser"),
            "hasWebSessionReference": isinstance(packet.get("ws"), str) and bool(packet["ws"]),
            "hasAuthenticationMaterial": summary.get("containsAuthenticationMaterial", False),
            "rawValuesSaved": False,
            "credentialsCompared": False,
        }
        write_private_json(state, COMPARISON_FILE, comparison)
        summary["routeComparisonSaved"] = True
    except (OSError, ValueError, zlib.error):
        summary["comparisonError"] = "local-route-comparison-failed"
        try:
            os.link(claim, flag)
        except OSError:
            pass
    finally:
        claim.unlink(missing_ok=True)


def receive(arguments):
    if len(arguments) != 1 or not arguments[0].startswith("accessclient://"):
        print("需要一个齐治启动链接；诊断工具不会打印链接正文。", file=sys.stderr)
        return 2
    repository = Path(__file__).resolve().parents[2]
    app = Path(
        os.environ.get("OMNITERM_APP", repository / "dist/local-native/omniterm-accessclient-inspector")
    )
    summary = {
        "schemaVersion": 1,
        "mode": "offline-inspection-only",
        "decodeSucceeded": False,
    }
    if len(arguments[0].encode("utf-8")) > 4 * 1024 * 1024:
        summary["error"] = "launch-envelope-too-large"
    else:
        try:
            result = subprocess.run(
                [str(app), "--inspect-accessclient"],
                input=arguments[0].encode("utf-8"),
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                timeout=15,
                check=False,
            )
            if result.returncode == 0:
                decoded = json.loads(result.stdout)
                if not isinstance(decoded, dict) or decoded.get("mode") != "offline-inspection-only":
                    raise ValueError("unexpected diagnostic output")
                summary.update(decoded, decodeSucceeded=True)
            else:
                summary["error"] = "invalid-or-unsupported-launch-envelope"
        except (OSError, ValueError, subprocess.TimeoutExpired):
            summary["error"] = "native-decoder-unavailable"
    summary["receivedAt"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    state = (
        Path(os.environ.get("XDG_STATE_HOME", Path.home() / ".local/state"))
        / "omniTerm"
    )
    state.mkdir(mode=0o700, parents=True, exist_ok=True)
    state.chmod(0o700)
    capture_next(state, arguments[0], summary)
    compare_next(state, arguments[0], summary)
    report = write_private_json(state, "accessclient-diagnostics.json", summary)
    if os.environ.get("OMNITERM_NO_NOTIFY") != "1" and shutil.which("zenity"):
        message = (
            "已收到齐治启动请求。当前接收器仅诊断格式，未建立资产连接。\n\n脱敏报告：\n"
            + str(report)
        )
        if summary.get("parametersSaved"):
            message = "完整连接参数已保存到本机（含认证字段），尚未建立资产连接。\n\n参数文件：\n" + summary["parametersFile"]
        elif summary.get("captureError"):
            message = "已收到启动请求，但完整参数保存失败。请检查本机状态目录的写入权限。"
        subprocess.run(
            [
                "zenity",
                "--info",
                "--title=omniTerm · 齐治连接诊断",
                "--text=" + message,
            ],
            check=False,
        )
    return 0 if summary["decodeSucceeded"] else 2


if __name__ == "__main__":
    raise SystemExit(receive(sys.argv[1:]))

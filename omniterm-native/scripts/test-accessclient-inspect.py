#!/usr/bin/env python3
# Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
"""Exercise the built native decoder and optional URI receiver without a network."""

import base64
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import unittest
import zlib

ROOT = Path(__file__).resolve().parents[2]
APP = Path(
    os.environ.get("OMNITERM_APP", ROOT / "omniterm-native/target/release/omniterm-app")
)
RECEIVER = ROOT / "omniterm-native/scripts/accessclient-inspect.py"


def wire(packet):
    compressed = zlib.compress(json.dumps(packet).encode("utf-8"))
    return "accessclient://" + base64.b64encode(compressed).decode("ascii")


class Diagnostics(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not APP.is_file():
            raise RuntimeError("Build the native app first, or set OMNITERM_APP")

    def test_native_diagnostics_never_echo_values(self):
        encoded = wire(
            {
                "app": "putty",
                "host": "private-host.invalid",
                "token": "private-ticket",
                "args": {"Proto": "ssh", "Password": "private-password"},
                "pw": "OTP1:private-ticket",
                "proxy.addr": "",
                "private-key": "private-value",
            }
        )
        result = subprocess.run(
            [str(APP), "--inspect-accessclient"],
            input=encoded.encode(),
            capture_output=True,
            timeout=10,
        )
        self.assertEqual(result.returncode, 0)
        self.assertNotIn(b"private", result.stdout + result.stderr)
        summary = json.loads(result.stdout)
        self.assertTrue(summary["containsAuthenticationMaterial"])
        self.assertEqual(summary["requestedClients"], ["putty"])
        self.assertEqual(summary["schemaVersion"], 2)
        self.assertEqual(summary["recognizedFieldTypes"]["pw"], "string")
        self.assertEqual(summary["credentialHints"], ["otp1-ticket"])
        self.assertNotIn("proxy.addr", summary["nonemptyRecognizedFields"])
        self.assertFalse(summary["referencesProxy"])
        self.assertEqual(summary["unknownFieldCount"], 1)

    def test_receiver_writes_only_private_redacted_reports(self):
        with tempfile.TemporaryDirectory(
            prefix="omniterm-accessclient-test-"
        ) as temporary:
            env = {
                **os.environ,
                "OMNITERM_APP": str(APP),
                "XDG_STATE_HOME": temporary,
                "OMNITERM_NO_NOTIFY": "1",
            }
            encoded = wire(
                {
                    "app": "xftp",
                    "header": "private-cookie",
                    "url_path": "/private-session",
                    "proxy_protocol": "websocket",
                }
            )
            result = subprocess.run(
                [sys.executable, str(RECEIVER), encoded],
                env=env,
                capture_output=True,
                timeout=10,
            )
            self.assertEqual(result.returncode, 0)
            report = Path(temporary) / "omniTerm/accessclient-diagnostics.json"
            output = report.read_text()
            self.assertNotIn("private", output)
            self.assertNotIn(encoded, output)
            summary = json.loads(output)
            self.assertTrue(summary["decodeSucceeded"])
            self.assertFalse((report.parent / "accessclient-connection-parameters.json").exists())
            self.assertTrue(
                summary["referencesWebRequest"] and summary["referencesProxy"]
            )
            if os.name == "posix":
                self.assertEqual(stat.S_IMODE(report.stat().st_mode), 0o600)
                self.assertEqual(stat.S_IMODE(report.parent.stat().st_mode), 0o700)
            result = subprocess.run(
                [sys.executable, str(RECEIVER), "accessclient://private-invalid-token"],
                env=env,
                capture_output=True,
                timeout=10,
            )
            self.assertEqual(result.returncode, 2)
            self.assertNotIn(b"private", result.stdout + result.stderr)
            self.assertNotIn("private", report.read_text())
            self.assertFalse(json.loads(report.read_text())["decodeSucceeded"])

    def test_explicit_local_capture_saves_one_sftp_request_without_echoing_secrets(self):
        with tempfile.TemporaryDirectory(prefix="omniterm-parameter-capture-test-") as temporary:
            state = Path(temporary) / "omniTerm"
            state.mkdir(mode=0o700)
            flag = state / "accessclient-capture-next.json"
            flag.write_text(json.dumps({"enabled": True}))
            flag.chmod(0o600)
            env = {**os.environ, "OMNITERM_APP": str(APP), "XDG_STATE_HOME": temporary, "OMNITERM_NO_NOTIFY": "1"}

            def receive(uri):
                result = subprocess.run([sys.executable, str(RECEIVER), uri], env=env, capture_output=True, timeout=10)
                self.assertNotIn(b"private", result.stdout + result.stderr)
                report = json.loads((state / "accessclient-diagnostics.json").read_text())
                self.assertNotIn("private", json.dumps(report))
                return result, report

            # Malformed requests and unrelated SSH launches cannot consume the opt-in.
            result, _ = receive("accessclient://private-invalid-ticket")
            self.assertEqual(result.returncode, 2)
            self.assertTrue(flag.exists())
            result, _ = receive(wire({"app": "putty", "pw": "private-ssh-ticket"}))
            self.assertEqual(result.returncode, 0)
            self.assertTrue(flag.exists())
            packet = {"app": "winscp", "host": "private-host.invalid", "port": "2222", "user": "private-user", "pwd": "private-ticket", "private-extra-field": "private-value"}
            result, report = receive(wire(packet))
            self.assertEqual(result.returncode, 0)
            self.assertTrue(report["parametersSaved"])
            saved = state / "accessclient-connection-parameters.json"
            self.assertEqual(json.loads(saved.read_text())["parameters"], packet)
            self.assertEqual(Path(report["parametersFile"]), saved)
            self.assertFalse(flag.exists())
            if os.name == "posix":
                self.assertEqual(stat.S_IMODE(saved.stat().st_mode), 0o600)
                self.assertEqual(stat.S_IMODE(state.stat().st_mode), 0o700)
            first = saved.read_bytes()
            result, report = receive(wire({**packet, "pwd": "private-later-ticket"}))
            self.assertEqual(result.returncode, 0)
            self.assertNotIn("parametersSaved", report)
            self.assertEqual(saved.read_bytes(), first, "a one-shot capture cannot overwrite on later launches")
            self.assertFalse(list(state.glob(".accessclient-*.json")))

    def test_argument_errors_do_not_expose_uri(self):
        result = subprocess.run(
            [str(APP), "--inspect-accessclient", "accessclient://private-ticket"],
            capture_output=True,
            timeout=10,
        )
        self.assertEqual(result.returncode, 2)
        self.assertNotIn(b"private", result.stdout + result.stderr)

        result = subprocess.run(
            [sys.executable, str(RECEIVER), "accessclient://private-ticket", "private-extra"],
            capture_output=True,
            timeout=10,
        )
        self.assertEqual(result.returncode, 2)
        self.assertNotIn(b"private", result.stdout + result.stderr)

    def test_one_shot_route_comparison_saves_only_boolean_matches(self):
        with tempfile.TemporaryDirectory(prefix="omniterm-route-comparison-test-") as temporary:
            state = Path(temporary) / "omniTerm"
            state.mkdir(mode=0o700)
            expected = {"gatewayHost": "private-gateway.invalid", "gatewayPort": 60022, "routeUser": "private-user/192.0.2.10/root", "gatewayUser": "private-user", "assetHost": "192.0.2.10", "assetUser": "root"}
            flag = state / "accessclient-compare-next.json"
            flag.write_text(json.dumps({"enabled": True, "expected": expected}))
            env = {**os.environ, "OMNITERM_APP": str(APP), "XDG_STATE_HOME": temporary, "OMNITERM_NO_NOTIFY": "1"}

            def receive(packet):
                result = subprocess.run([sys.executable, str(RECEIVER), wire(packet)], env=env, capture_output=True, timeout=10)
                self.assertEqual(result.returncode, 0)
                self.assertNotIn(b"private", result.stdout + result.stderr)

            result = subprocess.run([sys.executable, str(RECEIVER), "accessclient://private-invalid-ticket"], env=env, capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 2)
            self.assertTrue(flag.exists(), "invalid launches cannot consume comparison")
            receive({"app": "winscp", "pwd": "private-ticket"})
            self.assertTrue(flag.exists(), "file launches cannot consume SSH comparison")
            packet = {"app": "putty", "hn": expected["gatewayHost"], "pn": "60022", "un": "private-session-user", "pw": "private-ticket", "ws": "private-session", "sh": expected["assetHost"], "sa": expected["assetUser"]}
            receive(packet)
            comparison_path = state / "accessclient-route-comparison.json"
            text = comparison_path.read_text()
            self.assertNotIn("private", text)
            self.assertNotIn("192.0.2.10", text)
            report = json.loads(text)
            self.assertTrue(report["gatewayHostMatches"] and report["gatewayPortMatches"])
            self.assertFalse(report["transportUserMatchesRoute"])
            self.assertFalse(report["transportUserMatchesGatewayUser"])
            self.assertTrue(report["sessionHostFieldMatchesAsset"] and report["sessionAccountFieldMatchesAsset"])
            self.assertTrue(report["hasWebSessionReference"])
            self.assertFalse(report["rawValuesSaved"])
            self.assertFalse(flag.exists())
            self.assertFalse((state / "accessclient-connection-parameters.json").exists())
            first = comparison_path.read_bytes()
            receive({**packet, "un": expected["routeUser"]})
            self.assertEqual(comparison_path.read_bytes(), first)
            if os.name == "posix":
                self.assertEqual(stat.S_IMODE(comparison_path.stat().st_mode), 0o600)
            self.assertFalse(list(state.glob(".accessclient-compare-*.json")))


if __name__ == "__main__":
    unittest.main()

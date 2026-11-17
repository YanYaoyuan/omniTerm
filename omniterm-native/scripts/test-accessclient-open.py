#!/usr/bin/env python3
# Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
"""Receiver boundary tests: launch material goes through stdin, never argv/env."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import textwrap
import unittest

RECEIVER = Path(__file__).with_name("accessclient-open.py")


class Receiver(unittest.TestCase):
    def test_stdin_handoff_and_failed_validation_never_launches(self):
        with tempfile.TemporaryDirectory(prefix="omniterm-web-launch-test-") as temporary:
            root = Path(temporary)
            app = root / "fake app"
            app.write_text(textwrap.dedent('''\
                #!/usr/bin/python3
                import json,os,pathlib,sys
                incoming=sys.stdin.read()
                assert incoming not in json.dumps(sys.argv)
                assert incoming not in json.dumps(dict(os.environ))
                if sys.argv[1]=='--inspect-accessclient':
                    print(json.dumps({'requestedClients':[os.environ.get('CLIENT','putty')]}))
                elif sys.argv[1]=='--validate-accessclient':
                    raise SystemExit(2 if os.environ.get('REJECT')=='1' else 0)
                elif sys.argv[1]=='--open-accessclient':
                    pathlib.Path(os.environ['RECORD']).write_text(incoming)
            '''))
            app.chmod(0o700)
            record = root / "received"
            uri = "accessclient://fixture-only-authentication-material"
            env = {**os.environ, "OMNITERM_APP": str(app), "OMNITERM_NO_NOTIFY": "1", "RECORD": str(record)}
            result = subprocess.run([sys.executable, str(RECEIVER), uri], env=env, capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 0)
            self.assertEqual(record.read_text(), uri)
            self.assertNotIn(uri.encode(), result.stdout + result.stderr)
            record.unlink()
            result = subprocess.run([sys.executable, str(RECEIVER), uri], env={**env,"CLIENT":"winscp"}, capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 0)
            self.assertEqual(record.read_text(), uri)
            record.unlink()
            result = subprocess.run([sys.executable, str(RECEIVER), uri], env={**env,"REJECT":"1"}, capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 2)
            self.assertFalse(record.exists())

    def test_invalid_invocations_do_not_print_the_link(self):
        for args in [[], ["private-invalid"], ["accessclient://private", "private-extra"]]:
            result = subprocess.run([sys.executable, str(RECEIVER), *args], env={**os.environ,"OMNITERM_NO_NOTIFY":"1"}, capture_output=True, timeout=5)
            self.assertEqual(result.returncode, 2)
            self.assertNotIn(b"private", result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()

# Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('validate_release', Path(__file__).with_name('validate-release.py'))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.commit = 'a' * 40
        self.fingerprint = 'b' * 64
        for filename, platform, arch, target in [
            ('omniTerm-0.1.0-windows-x64.zip', 'win32', 'x64', 'x86_64-pc-windows-msvc'),
            ('omniTerm-0.1.0-macos-arm64.zip', 'darwin', 'arm64', 'aarch64-apple-darwin'),
            ('omniterm_0.1.0-1_amd64.deb', 'linux', 'x64', 'x86_64-unknown-linux-gnu'),
        ]:
            artifact = self.directory / filename
            artifact.write_bytes(platform.encode())
            checksum = release.digest(artifact)
            manifest = dict(schemaVersion=1, version='0.1.0', desktopSourceCommit=self.commit,
                            sourceDirty=False, platform=platform, arch=arch,
                            agent=dict(target=target, agentSourceFingerprint=self.fingerprint,
                                       desktopSourceCommit=self.commit, agentBuildSourceCommit=self.commit,
                                       binarySha256={'bin/codex': 'c' * 64}),
                            artifact=dict(filename=filename, bytes=artifact.stat().st_size, sha256=checksum))
            (self.directory / (filename + '.manifest.json')).write_text(json.dumps(manifest))
            (self.directory / (filename + '.sha256')).write_text(f'{checksum}  {filename}\n')
            if platform == 'linux':
                (self.directory / filename.replace('.deb', '.build.json')).write_text(json.dumps(dict(
                    version='0.1.0-1', sourceCommit=self.commit, sourceDirty=False, debSha256=checksum)))

    def run_validation(self):
        return release.validate(self.directory, '0.1.0', self.commit, self.fingerprint)

    def test_complete_set_and_repeatable_checksums(self):
        self.assertEqual(self.run_validation(), self.run_validation())
        self.assertEqual(len((self.directory / 'SHA256SUMS').read_text().splitlines()), 10)

    def test_missing_platform_rejected(self):
        (self.directory / 'omniTerm-0.1.0-macos-arm64.zip').unlink()
        with self.assertRaises(FileNotFoundError):
            self.run_validation()

    def test_payload_tampering_rejected(self):
        (self.directory / 'omniTerm-0.1.0-windows-x64.zip').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'hash/size'):
            self.run_validation()

    def test_stale_or_wrong_source_metadata_rejected(self):
        for field, value in [('version', '0.0.1'), ('desktopSourceCommit', 'd' * 40), ('sourceDirty', True)]:
            with self.subTest(field=field):
                path = self.directory / 'omniTerm-0.1.0-windows-x64.zip.manifest.json'
                original = path.read_text()
                manifest = json.loads(original)
                manifest[field] = value
                path.write_text(json.dumps(manifest))
                with self.assertRaises(ValueError):
                    self.run_validation()
                path.write_text(original)

    def test_wrong_agent_source_rejected(self):
        with self.assertRaisesRegex(ValueError, 'Agent target/source'):
            release.validate(self.directory, '0.1.0', self.commit, 'd' * 64)

    def test_checksum_tampering_rejected(self):
        (self.directory / 'omniTerm-0.1.0-windows-x64.zip.sha256').write_text('wrong')
        with self.assertRaisesRegex(ValueError, 'checksum file'):
            self.run_validation()


if __name__ == '__main__':
    unittest.main()

# Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import hashlib
import importlib.util
from pathlib import Path
import tarfile
import tempfile
import unittest
import zipfile
import io

spec = importlib.util.spec_from_file_location('native_release', Path(__file__).with_name('validate-native-release.py'))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class PayloadTests(unittest.TestCase):
    def test_changed_missing_and_duplicate_zip_payloads_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'package.zip'
            expected = {'app/cli.js': hashlib.sha256(b'good').hexdigest()}
            for names, error in [([('app/cli.js', b'changed')], 'changed'),
                                 ([('app/other', b'good')], 'missing'),
                                 ([('app/cli.js', b'good'), ('app/cli.js', b'good')], 'duplicate')]:
                with zipfile.ZipFile(path, 'w') as archive:
                    for name, value in names:
                        archive.writestr(name, value)
                with self.assertRaisesRegex(ValueError, error):
                    release.verify_payload(path, expected)

    def test_streamed_tar_checks_payload_without_extracting_paths(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'package.tar.gz'
            with tarfile.open(path, 'w:gz') as archive:
                for name, value in [('./app/cli.js', b'good'), ('../../escape', b'ignored')]:
                    entry = tarfile.TarInfo(name)
                    entry.size = len(value)
                    archive.addfile(entry, io.BytesIO(value))
            release.verify_payload(path, {'app/cli.js': hashlib.sha256(b'good').hexdigest()})
            self.assertFalse((Path(directory) / 'escape').exists())


if __name__ == '__main__':
    unittest.main()

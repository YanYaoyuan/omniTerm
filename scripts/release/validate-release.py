# Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
"""Validate the complete native release set and write SHA256SUMS."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[2]
SOURCE_PATHS = ['codex-rs', 'scripts/codex_package', 'scripts/build_codex_package.py', 'third_party/v8']


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def validate(directory, version, commit, fingerprint):
    directory = Path(directory)
    platforms = [
        (f'omniTerm-{version}-windows-x64.zip', 'win32', 'x64', 'x86_64-pc-windows-msvc'),
        (f'omniTerm-{version}-macos-arm64.zip', 'darwin', 'arm64', 'aarch64-apple-darwin'),
        (f'omniterm_{version}-1_amd64.deb', 'linux', 'x64', 'x86_64-unknown-linux-gnu'),
    ]
    expected = set()
    for filename, platform, arch, target in platforms:
        artifact = directory / filename
        manifest = json.loads((directory / (filename + '.manifest.json')).read_text(encoding='utf-8'))
        agent = manifest.get('agent', {})
        required = {'schemaVersion': 1, 'version': version, 'desktopSourceCommit': commit,
                    'sourceDirty': False, 'platform': platform, 'arch': arch}
        for key, value in required.items():
            if manifest.get(key) != value:
                raise ValueError(f'{filename}: incorrect {key}')
        if agent.get('target') != target or agent.get('agentSourceFingerprint') != fingerprint:
            raise ValueError(f'{filename}: Agent target/source mismatch')
        if agent.get('desktopSourceCommit') != commit or not re.fullmatch(r'[0-9a-f]{40}', agent.get('agentBuildSourceCommit', '')):
            raise ValueError(f'{filename}: missing Agent build provenance')
        hashes = agent.get('binarySha256', {})
        if not hashes or not all(re.fullmatch(r'[0-9a-f]{64}', value) for value in hashes.values()):
            raise ValueError(f'{filename}: missing Agent binary hashes')
        actual = digest(artifact)
        if manifest.get('artifact') != {'filename': filename, 'bytes': artifact.stat().st_size, 'sha256': actual}:
            raise ValueError(f'{filename}: artifact hash/size mismatch')
        if (directory / (filename + '.sha256')).read_text(encoding='utf-8').strip() != f'{actual}  {filename}':
            raise ValueError(f'{filename}: checksum file mismatch')
        expected.update([filename, filename + '.manifest.json', filename + '.sha256'])
        if platform == 'linux':
            build_name = filename.removesuffix('.deb') + '.build.json'
            build = json.loads((directory / build_name).read_text(encoding='utf-8'))
            if (build.get('version') != version + '-1' or build.get('sourceCommit') != commit
                    or build.get('sourceDirty') is not False or build.get('debSha256') != actual):
                raise ValueError(f'{filename}: Debian build record mismatch')
            expected.add(build_name)
    actual_names = {p.name for p in directory.iterdir()}
    if actual_names - expected - {'SHA256SUMS'}:
        raise ValueError('Unexpected release files: ' + ', '.join(sorted(actual_names - expected - {'SHA256SUMS'})))
    sums = ''.join(f'{digest(directory / name)}  {name}\n' for name in sorted(expected))
    (directory / 'SHA256SUMS').write_text(sums, encoding='utf-8', newline='\n')
    return sums


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    args = parser.parse_args()
    version = json.loads((ROOT / 'omniterm-desktop/package.json').read_text(encoding='utf-8'))['version']
    commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    tree = subprocess.check_output(['git', 'ls-tree', '-r', 'HEAD', '--', *SOURCE_PATHS], cwd=ROOT)
    if not tree:
        raise ValueError('Missing Agent source tree')
    validate(args.directory, version, commit, hashlib.sha256(tree).hexdigest())
    print(f'Validated all three platforms for v{version} at {commit}; SHA256SUMS written.')


if __name__ == '__main__':
    main()

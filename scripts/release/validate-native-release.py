#!/usr/bin/env python3
# Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
"""Verify all native release payloads and the standalone agent Skill."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import tarfile
import zipfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('desktop_release', Path(__file__).with_name('validate-release.py'))
desktop = importlib.util.module_from_spec(spec)
spec.loader.exec_module(desktop)


def file_hash(stream):
    return hashlib.file_digest(stream, 'sha256').hexdigest()


def verify_payload(artifact, expected):
    """Read expected payloads without extracting archives or trusting their paths."""
    found = set()
    def check(name, stream):
        if name in found or file_hash(stream) != expected[name]:
            raise ValueError(f'{artifact.name}: duplicate or changed payload: {name}')
        found.add(name)
    if artifact.suffix == '.zip':
        with zipfile.ZipFile(artifact) as archive:
            for item in archive.infolist():
                if item.filename in expected:
                    with archive.open(item) as stream:
                        check(item.filename, stream)
    else:
        process = None
        try:
            if artifact.suffix == '.deb':
                process = subprocess.Popen(['dpkg-deb', '--fsys-tarfile', str(artifact)], stdout=subprocess.PIPE)
                archive = tarfile.open(fileobj=process.stdout, mode='r|')
            else:
                archive = tarfile.open(artifact, mode='r|gz')
            with archive:
                for item in archive:
                    name = item.name.removeprefix('./')
                    if name in expected:
                        if not item.isfile():
                            raise ValueError(f'{artifact.name}: expected regular file: {name}')
                        with archive.extractfile(item) as stream:
                            check(name, stream)
            if process:
                process.stdout.close()
                if process.wait() != 0:
                    raise ValueError(f'Cannot read {artifact.name}')
        finally:
            if process and process.poll() is None:
                process.stdout.close()
                process.terminate()
                process.wait()
    if found != set(expected):
        raise ValueError(f'{artifact.name}: missing payloads: {sorted(set(expected) - found)}')


def verify_report(directory, filename, version, commit):
    artifact = directory / filename
    report = json.loads((directory / (filename + '.manifest.json')).read_text())
    for key, value in dict(schemaVersion=1, version=version, desktopSourceCommit=commit, sourceDirty=False).items():
        if report.get(key) != value:
            raise ValueError(f'{filename}: incorrect {key}')
    checksum = desktop.digest(artifact)
    if report.get('artifact') != dict(filename=filename, bytes=artifact.stat().st_size, sha256=checksum):
        raise ValueError(f'{filename}: artifact hash/size mismatch')
    if (directory / (filename + '.sha256')).read_text().strip() != f'{checksum}  {filename}':
        raise ValueError(f'{filename}: checksum mismatch')
    return artifact, report


def write_sums(directory, expected, name):
    actual = {p.name for p in directory.iterdir()}
    if actual - expected - {name}:
        raise ValueError(f'Unexpected release files in {directory}: {sorted(actual - expected - {name})}')
    (directory / name).write_text(''.join(f'{desktop.digest(directory / p)}  {p}\n' for p in sorted(expected)), newline='\n')


def validate(native_dir, skill_dir):
    version = json.loads((ROOT / 'omniterm-desktop/package.json').read_text())['version']
    commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    tree = subprocess.check_output(['git', 'ls-tree', '-r', 'HEAD', '--', *desktop.SOURCE_PATHS], cwd=ROOT)
    fingerprint = hashlib.sha256(tree).hexdigest()
    cli = ROOT / 'omniterm-cli'
    cli_files = {}
    for name in ['bin', 'src', 'skills', 'package.json', 'README.md', 'LICENSE', 'NOTICE']:
        path = cli / name
        for item in sorted(path.rglob('*')) if path.is_dir() else [path]:
            if item.is_file():
                cli_files[item.relative_to(cli).as_posix()] = desktop.digest(item)
    expected = set()
    for filename, platform, arch, target, prefix in [
        (f'omniTerm-native-{version}-windows-x64.zip', 'win32', 'x64', 'x86_64-pc-windows-msvc', 'omniTerm'),
        (f'omniTerm-native-{version}-macos-arm64.zip', 'darwin', 'arm64', 'aarch64-apple-darwin', 'omniTerm.app/Contents/Resources'),
        (f'omniTerm-native-{version}-linux-x64.tar.gz', 'linux', 'x64', 'x86_64-unknown-linux-gnu', f'omniterm-native-{version}'),
        (f'omniterm-native_{version}-1_amd64.deb', 'linux', 'x64', 'x86_64-unknown-linux-gnu', 'usr/lib/omniterm-native'),
    ]:
        artifact, report = verify_report(native_dir, filename, version, commit)
        agent = report.get('agent') or {}
        if (report.get('app') != 'omniterm-native' or report.get('platform') != platform or report.get('arch') != arch
                or agent.get('target') != target or agent.get('desktopSourceCommit') != commit
                or agent.get('agentSourceFingerprint') != fingerprint):
            raise ValueError(f'{filename}: native/Agent source or target mismatch')
        hashes = report.get('packagedSignedBinarySha256') if platform == 'darwin' else agent.get('binarySha256')
        if not hashes or set(hashes) != set(agent.get('binarySha256', {})):
            raise ValueError(f'{filename}: missing Agent binary hashes')
        payloads = {f'{prefix}/omniterm-agent/{p}': checksum for p, checksum in hashes.items()}
        payloads.update({f'{prefix}/omniterm-cli/{p}': checksum for p, checksum in cli_files.items()})
        verify_payload(artifact, payloads)
        expected.update([filename, filename + '.manifest.json', filename + '.sha256'])
    write_sums(native_dir, expected, 'SHA256SUMS-native')
    filename = f'omniterm-control-skill-{version}.zip'
    artifact, report = verify_report(skill_dir, filename, version, commit)
    source = cli / 'skills/omniterm-control'
    files = {p.relative_to(source).as_posix(): desktop.digest(p) for p in source.rglob('*') if p.is_file()}
    if report.get('name') != 'omniterm-control' or report.get('files') != files:
        raise ValueError('Standalone Skill differs from this release source')
    payloads = {f'omniterm-control/{p}': checksum for p, checksum in files.items()}
    payloads.update({name: desktop.digest(ROOT / name) for name in ['LICENSE', 'NOTICE']})
    with zipfile.ZipFile(artifact) as archive:
        if set(archive.namelist()) != set(payloads):
            raise ValueError('Unexpected standalone Skill files')
    verify_payload(artifact, payloads)
    write_sums(skill_dir, {filename, filename + '.manifest.json', filename + '.sha256'}, 'SHA256SUMS-skills')
    print(f'Validated native packages on all three platforms and standalone Skill for v{version} at {commit}.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('native_directory', type=Path)
    parser.add_argument('skill_directory', type=Path)
    args = parser.parse_args()
    validate(args.native_directory, args.skill_directory)

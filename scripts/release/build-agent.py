#!/usr/bin/env python3
"""Build only the complete desktop Agent, or verify an identical-source local reuse."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
os.environ['CODEX_REPO_ROOT'] = str(ROOT)
sys.path.insert(0, str(ROOT / 'scripts'))
from codex_package.targets import TARGET_SPECS
from codex_package.v8 import resolve_codex_v8_cargo_env

SOURCE_PATHS = ['codex-rs', 'scripts/codex_package', 'scripts/build_codex_package.py', 'third_party/v8']


def command(*args, cwd=ROOT, env=None):
    print('+', ' '.join(map(str, args)), flush=True)
    subprocess.run(list(map(str, args)), cwd=cwd, env=env, check=True)


def git(*args):
    return subprocess.check_output(['git', *args], cwd=ROOT, text=True).strip()


def digest(filename):
    with Path(filename).open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def fingerprint(revision):
    tree = subprocess.check_output(['git', 'ls-tree', '-r', revision, '--', *SOURCE_PATHS], cwd=ROOT)
    if not tree:
        raise RuntimeError('Agent source tree is empty')
    return hashlib.sha256(tree).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', choices=TARGET_SPECS, required=True)
    parser.add_argument('--profile', choices=['dev-small', 'release'], default='dev-small')
    parser.add_argument('--package-dir', type=Path, default=ROOT / 'dist/omniterm-agent')
    parser.add_argument('--verify-existing', action='store_true', help='Verify a downloaded Agent artifact against this checkout without recompiling')
    parser.add_argument('--reuse-directory', type=Path)
    parser.add_argument('--reuse-source-commit', help='Required source revision for an existing local Agent; its complete Agent source tree must match HEAD')
    args = parser.parse_args()
    spec = TARGET_SPECS[args.target]
    expected = {('Windows', 'AMD64'): 'x86_64-pc-windows-msvc', ('Linux', 'x86_64'): 'x86_64-unknown-linux-gnu', ('Darwin', 'arm64'): 'aarch64-apple-darwin'}
    if expected.get((platform.system(), platform.machine())) != args.target:
        raise RuntimeError('Use a native Windows x64, Ubuntu x64, or macOS arm64 builder for this target')
    if git('status', '--porcelain', '--untracked-files=normal'):
        raise RuntimeError('Commit source changes before building a release Agent')
    if bool(args.reuse_directory) != bool(args.reuse_source_commit):
        raise RuntimeError('--reuse-directory and --reuse-source-commit must be supplied together')
    package = args.package_dir.resolve()
    commit = git('rev-parse', 'HEAD')
    source_fingerprint = fingerprint(commit)
    reused_from = None
    if args.verify_existing:
        if args.reuse_directory:
            raise RuntimeError('--verify-existing cannot be combined with local reuse')
        existing = json.loads((package / 'omniterm-source.json').read_text())
        if existing.get('target') != args.target or existing.get('agentSourceFingerprint') != source_fingerprint:
            raise RuntimeError('Downloaded Agent target/source fingerprint differs from this checkout')
        reused_from = existing['agentBuildSourceCommit']
        if existing.get('cargoProfile') not in ['dev-small', 'release']:
            raise RuntimeError('Downloaded Agent has an unsupported build profile')
        args.profile = existing['cargoProfile']
    elif args.reuse_directory:
        reused_from = git('rev-parse', args.reuse_source_commit)
        if fingerprint(reused_from) != source_fingerprint:
            raise RuntimeError('The existing Agent source tree differs from this release; rebuild it')
        source = args.reuse_directory.resolve(strict=True)
        manifest = json.loads((source / 'codex-package.json').read_text())
        if manifest.get('target') != args.target:
            raise RuntimeError('Reused Agent target does not match this platform')
        if package.exists():
            raise RuntimeError('Reuse output must be a new directory, never overwrite the existing Agent')
        shutil.copytree(source, package)
    else:
        env = {**os.environ, 'CODEX_REPO_ROOT': str(ROOT), 'CARGO_NET_GIT_FETCH_WITH_CLI': 'true', 'CARGO_INCREMENTAL': '0', 'STABLE_GIT_COMMIT': commit}
        env.setdefault('CARGO_BUILD_JOBS', '2')
        env.setdefault('CARGO_NET_RETRY', '5')
        if spec.is_windows:
            env['LIBSQLITE3_FLAGS'] = 'SQLITE_DISABLE_INTRINSIC'
        target_dir = Path(env.get('CARGO_TARGET_DIR', ROOT / 'codex-rs/target')).resolve()
        env['CARGO_TARGET_DIR'] = str(target_dir)
        env.update(resolve_codex_v8_cargo_env(spec, environ=env))
        outputs = target_dir / args.target / args.profile
        build = ['cargo', 'build', '--locked', '--target', args.target, '--profile', args.profile]
        resources = []
        if spec.is_linux:
            command(*build, '-p', 'codex-bwrap', '--bin', 'bwrap', cwd=ROOT / 'codex-rs', env=env)
            env['CODEX_BWRAP_SHA256'] = digest(outputs / 'bwrap')
            resources += ['--bwrap-bin', str(outputs / 'bwrap')]
        binaries = ['codex', 'codex-code-mode-host']
        if spec.is_windows:
            binaries += ['codex-command-runner', 'codex-windows-sandbox-setup']
            resources += ['--codex-command-runner-bin', str(outputs / 'codex-command-runner.exe'), '--codex-windows-sandbox-setup-bin', str(outputs / 'codex-windows-sandbox-setup.exe')]
        command(*build, *[value for binary in binaries for value in ['--bin', binary]], cwd=ROOT / 'codex-rs', env=env)
        suffix = '.exe' if spec.is_windows else ''
        command(sys.executable, ROOT / 'scripts/build_codex_package.py', '--target', args.target, '--cargo-profile', args.profile,
                '--package-dir', package, '--force', '--entrypoint-bin', outputs / f'codex{suffix}',
                '--code-mode-host-bin', outputs / f'codex-code-mode-host{suffix}', *resources, env=env)
    suffix = '.exe' if spec.is_windows else ''
    required = [f'bin/codex{suffix}', f'bin/codex-code-mode-host{suffix}', f'codex-path/rg{suffix}']
    if spec.is_linux:
        required += ['codex-resources/bwrap']
    if spec.is_windows:
        required += ['codex-resources/codex-command-runner.exe', 'codex-resources/codex-windows-sandbox-setup.exe']
    hashes = {name: digest(package / name) for name in required}
    if args.verify_existing and any(existing.get('binarySha256', {}).get(name) != value for name, value in hashes.items()):
        raise RuntimeError('Downloaded Agent required binary hash is missing or does not match')
    manifest = json.loads((package / 'codex-package.json').read_text())
    if manifest.get('target') != args.target or manifest.get('variant') != 'codex':
        raise RuntimeError('Invalid final Agent manifest')
    provenance = {'schemaVersion': 1, 'desktopSourceCommit': commit, 'agentBuildSourceCommit': reused_from or commit,
                  'agentSourceFingerprint': source_fingerprint, 'target': args.target, 'cargoProfile': args.profile,
                  'reusedIdenticalSource': reused_from is not None, 'binarySha256': hashes}
    if args.verify_existing:
        provenance['reusedArtifactBuildSourceCommit'] = existing['agentBuildSourceCommit']
    (package / 'omniterm-source.json').write_text(json.dumps(provenance, indent=2) + '\n')
    command(package / f'bin/codex{suffix}', '--version')
    command(package / f'codex-path/rg{suffix}', '--version')
    print(f'Complete source-built Agent: {package}', flush=True)


if __name__ == '__main__':
    main()

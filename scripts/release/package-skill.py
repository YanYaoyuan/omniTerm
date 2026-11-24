#!/usr/bin/env python3
# Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
"""Package the complete standalone omniTerm Skill with source and file hashes."""
import hashlib
import json
from pathlib import Path
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[2]
SKILL = ROOT / 'omniterm-cli/skills/omniterm-control'


def main():
    version = json.loads((ROOT / 'omniterm-desktop/package.json').read_text())['version']
    if subprocess.check_output(['git', 'status', '--porcelain', '--untracked-files=normal'], cwd=ROOT):
        raise ValueError('Skill releases require a clean source checkout')
    commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    output = ROOT / 'dist/release-skills'
    output.mkdir(parents=True, exist_ok=True)
    artifact = output / f'omniterm-control-skill-{version}.zip'
    files = {p.relative_to(SKILL).as_posix(): p.read_bytes() for p in sorted(SKILL.rglob('*')) if p.is_file()}
    entry = files['SKILL.md'].decode()
    if not entry.startswith('---\nname: omniterm-control\ndescription: ') or '\n---\n' not in entry[4:]:
        raise ValueError('Missing Skill name/description frontmatter')
    for required in ['scripts/omniterm.mjs', 'references/terminal.md', 'references/sftp.md', 'references/recovery.md']:
        if required not in files:
            raise ValueError(f'Missing Skill resource: {required}')
    with zipfile.ZipFile(artifact, 'w', compression=zipfile.ZIP_DEFLATED) as archive:
        for relative, payload in files.items():
            info = zipfile.ZipInfo(f'omniterm-control/{relative}', date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o100644 << 16
            archive.writestr(info, payload)
        for name in ['LICENSE', 'NOTICE']:
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o100644 << 16
            archive.writestr(info, (ROOT / name).read_bytes())
    checksum = hashlib.sha256(artifact.read_bytes()).hexdigest()
    artifact.with_name(artifact.name + '.sha256').write_text(f'{checksum}  {artifact.name}\n', newline='\n')
    report = dict(schemaVersion=1, name='omniterm-control', version=version, desktopSourceCommit=commit,
                  sourceDirty=False, files={p: hashlib.sha256(b).hexdigest() for p, b in files.items()},
                  artifact=dict(filename=artifact.name, bytes=artifact.stat().st_size, sha256=checksum))
    artifact.with_name(artifact.name + '.manifest.json').write_text(json.dumps(report, indent=2) + '\n', newline='\n')
    print(f'Skill: {artifact}\nSHA-256: {checksum}')


if __name__ == '__main__':
    main()

// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
// Assemble downloadable, unsigned artifacts from the existing native app bundle.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdir, open, readFile, stat, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const desktop = fileURLToPath(new URL('../', import.meta.url));
const repository = path.resolve(desktop, '..');
const output = path.join(repository, 'dist/release');
const command = (exe, args, options = {}) => execFileSync(exe, args, { cwd: repository, stdio: 'inherit', ...options });
const git = (...args) => execFileSync('git', args, { cwd: repository, encoding: 'utf8', maxBuffer: 16 * 1024 * 1024 }).trim();
async function sha256(filename) {
  const hash = createHash('sha256');
  const file = await open(filename, 'r');
  try { for await (const chunk of file.createReadStream()) hash.update(chunk); } finally { await file.close(); }
  return hash.digest('hex');
}
const metadata = JSON.parse(await readFile(path.join(desktop, 'package.json'), 'utf8'));
if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(metadata.version)) throw new Error('Invalid release version');
if (git('status', '--porcelain', '--untracked-files=normal')) throw new Error('Release packaging requires a clean source checkout');
const sourceCommit = git('rev-parse', 'HEAD');
const releaseTag = process.env.GITHUB_REF_TYPE === 'tag' ? process.env.GITHUB_REF_NAME : null;
if (releaseTag && releaseTag !== `v${metadata.version}`) throw new Error('Release tag does not match the desktop version');
const bundle = path.join(repository, `dist/desktop/omniTerm-${process.platform}-${process.arch}`);
const resourceRoot = process.platform === 'darwin' ? path.join(bundle, 'omniTerm.app/Contents/Resources') : path.join(bundle, 'resources');
const agentRoot = path.join(resourceRoot, 'omniterm-agent');
const agent = JSON.parse(await readFile(path.join(agentRoot, 'omniterm-source.json'), 'utf8'));
const tree = execFileSync('git', ['ls-tree', '-r', 'HEAD', '--', 'codex-rs', 'scripts/codex_package', 'scripts/build_codex_package.py', 'third_party/v8'], { cwd: repository, maxBuffer: 16 * 1024 * 1024 });
if (createHash('sha256').update(tree).digest('hex') !== agent.agentSourceFingerprint) throw new Error('Bundled Agent was built from a different Agent source tree');
for (const [relative, expected] of Object.entries(agent.binarySha256)) {
  if (path.isAbsolute(relative) || relative.split(/[\\/]/).includes('..')) throw new Error('Invalid Agent provenance path');
  if (await sha256(path.join(agentRoot, relative)) !== expected) throw new Error(`Bundled Agent hash mismatch: ${relative}`);
}
if (!Object.keys(agent.binarySha256).length) throw new Error('Agent provenance contains no binaries');
await mkdir(output, { recursive: true });
let artifact;
if (process.platform === 'darwin' && process.arch === 'arm64') {
  artifact = path.join(output, `omniTerm-${metadata.version}-macos-arm64.zip`);
  command('/usr/bin/ditto', ['-c', '-k', '--sequesterRsrc', '--keepParent', path.join(bundle, 'omniTerm.app'), artifact]);
} else if (process.platform === 'win32' && process.arch === 'x64') {
  artifact = path.join(output, `omniTerm-${metadata.version}-windows-x64.zip`);
  // Python's standard ZIP implementation handles large portable bundles and Unicode paths.
  command('python', ['-c', 'import pathlib,sys,zipfile; root=pathlib.Path(sys.argv[1]); out=pathlib.Path(sys.argv[2]); z=zipfile.ZipFile(out,"w",zipfile.ZIP_DEFLATED,compresslevel=6); [z.write(p,p.relative_to(root.parent)) for p in sorted(root.rglob("*")) if p.is_file()]; z.close()', bundle, artifact]);
} else if (process.platform === 'linux' && process.arch === 'x64') {
  artifact = path.join(output, `omniterm_${metadata.version}-1_amd64.deb`);
  command(process.execPath, [path.join(desktop, 'scripts/package-deb.mjs'), '--app-dir', bundle, '--output-dir', output, '--version', `${metadata.version}-1`]);
} else throw new Error('Supported releases: macOS arm64, Windows x64, Ubuntu 22.04 x64');
const filename = path.basename(artifact);
const checksum = await sha256(artifact);
const report = { schemaVersion: 1, version: metadata.version, desktopSourceCommit: sourceCommit, sourceDirty: false, platform: process.platform, arch: process.arch, agent, signed: false, notarized: false, artifact: { filename, bytes: (await stat(artifact)).size, sha256: checksum }, electronVersion: metadata.devDependencies.electron, buildProfile: agent.cargoProfile, linuxLauncherUtf8Fallback: process.platform === 'linux' };
await writeFile(`${artifact}.sha256`, `${checksum}  ${filename}\n`);
await writeFile(`${artifact}.manifest.json`, `${JSON.stringify(report, null, 2)}\n`);
console.log(`Release artifact: ${artifact}\nSHA-256: ${checksum}\nUnsigned preview; no signing or notarization credentials were used.`);

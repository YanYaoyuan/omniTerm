// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
// Test the packaged runtime and packaged native PTY, without model credentials.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { mkdtempSync, realpathSync, rmSync } from 'node:fs';
import { createRequire } from 'node:module';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { stripVTControlCharacters } from 'node:util';

const script = fileURLToPath(import.meta.url);
const inside = process.argv[2] === '--inside';
const repository = path.resolve(path.dirname(script), '../..');
const bundle = path.resolve(process.argv[inside ? 3 : 2] || path.join(repository, `dist/desktop/omniTerm-${process.platform}-${process.arch}`));
const executable = process.platform === 'darwin' ? path.join(bundle, 'omniTerm.app/Contents/MacOS/omniTerm') : path.join(bundle, process.platform === 'win32' ? 'omniTerm.exe' : 'omniTerm');
const resources = process.platform === 'darwin' ? path.join(bundle, 'omniTerm.app/Contents/Resources') : path.join(bundle, 'resources');
if (!inside) {
  const child = spawn(executable, [script, '--inside', bundle], { stdio: 'inherit', env: { ...process.env, ELECTRON_RUN_AS_NODE: '1' } });
  child.once('error', error => { console.error(error); process.exitCode = 1; });
  child.once('exit', code => { process.exitCode = code ?? 1; });
} else {
  assert.ok(process.versions.electron, 'Must use the packaged Electron runtime');
  assert.equal(realpathSync(process.execPath), realpathSync(executable));
  const require = createRequire(path.join(resources, 'app/package.json'));
  const ptyEntry = realpathSync(require.resolve('node-pty'));
  const expectedRoot = realpathSync(path.join(resources, 'app/node_modules/node-pty'));
  assert.ok(ptyEntry.startsWith(expectedRoot + path.sep), 'Native PTY must come from the application');
  const pty = require(ptyEntry);
  const scratch = mkdtempSync(path.join(os.tmpdir(), 'omniterm-release-pty-'));
  const env = { ...process.env, TERM: 'xterm-256color' };
  delete env.ELECTRON_RUN_AS_NODE; delete env.NODE_OPTIONS; delete env.NODE_PATH;
  if (process.platform !== 'win32') env.LANG = env.LC_ALL = process.platform === 'darwin' ? 'en_US.UTF-8' : 'C.UTF-8';
  const live = new Set();
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
  const open = (file, args) => {
    const terminal = { handle: pty.spawn(file, args, { cols: 120, rows: 32, cwd: scratch, env, name: 'xterm-256color' }), output: '', exited: false };
    terminal.handle.onData(data => { terminal.output += data; });
    terminal.handle.onExit(event => { terminal.exited = true; terminal.exitCode = event.exitCode; live.delete(terminal); });
    live.add(terminal); return terminal;
  };
  const wait = async (predicate, label, terminal) => {
    const deadline = Date.now() + 20000;
    while (!predicate()) {
      if (Date.now() >= deadline) throw new Error(`${label}\n${terminal.output.slice(-2500)}`);
      await delay(30);
    }
  };
  try {
    const windows = process.platform === 'win32';
    const terminal = open(windows ? 'powershell.exe' : '/bin/bash', windows ? ['-NoLogo', '-NoProfile', '-NoExit'] : ['--noprofile', '--norc', '-i']);
    // Windows node-pty loads conpty.node lazily when the first PTY is spawned.
    const nativeFiles = Object.keys(require.cache).filter(file => file.endsWith('.node'));
    assert.ok(nativeFiles.length > 0, 'PTY spawn must load a packaged native module');
    for (const file of nativeFiles) assert.ok(realpathSync(file).startsWith(expectedRoot + path.sep));
    const token = randomBytes(5).toString('hex');
    const marker = name => `__OMNITERM_${name}_${token}__`;
    const print = name => windows ? `Write-Output ('__OMNITERM_' + '${name}_${token}__')` : `printf '%s%s\\n' '__OMNITERM_' '${name}_${token}__'`;
    const contains = value => stripVTControlCharacters(terminal.output).includes(value);
    terminal.handle.write(`${print('READY')}; ${windows ? '(Get-Location).Path' : 'pwd'}\r`);
    await wait(() => contains(marker('READY')) && contains(scratch), 'Shell input and working directory', terminal);
    terminal.handle.write(`${print('中文')}\r`);
    await wait(() => contains(marker('中文')), 'Unicode shell input', terminal);
    terminal.handle.resize(112, 37);
    terminal.handle.write(`${print('SLEEP')}; ${windows ? 'Start-Sleep -Seconds 30' : 'sleep 30'}\r`);
    await wait(() => contains(marker('SLEEP')), 'Interrupt target started', terminal);
    await delay(250); terminal.handle.write('\x03'); await delay(200);
    terminal.handle.write(`${print('INTERRUPT')}\r`);
    await wait(() => contains(marker('INTERRUPT')), 'Ctrl+C returned a usable shell', terminal);
    terminal.handle.kill();
    await wait(() => terminal.exited, 'PTY close', terminal);
    const agent = open(path.join(resources, 'omniterm-agent/bin', windows ? 'codex.exe' : 'codex'), ['--version']);
    await wait(() => agent.exited, 'Bundled Agent --version', agent);
    assert.equal(agent.exitCode, 0, agent.output);
    assert.match(stripVTControlCharacters(agent.output), /codex-cli\s+\S+/);
    console.log(`PASS: packaged Electron ${process.versions.electron}, native PTY, shell/pwd/Unicode, resize request, Ctrl+C, close, and bundled Agent --version (${process.platform}/${process.arch})`);
  } finally {
    for (const terminal of live) { try { terminal.handle.kill(); } catch {} }
    rmSync(scratch, { recursive: true, force: true });
  }
}

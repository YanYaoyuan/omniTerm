// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
// Test the packaged runtime and packaged terminal host, without model credentials.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { mkdtempSync, realpathSync, rmSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
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
  const hostBinary = path.join(resources, 'terminal-host', process.platform === 'win32' ? 'omniterm-terminal-host.exe' : 'omniterm-terminal-host');
  const { TerminalHost } = await import(pathToFileURL(path.join(repository, 'omniterm-desktop/electron/terminal-host.mjs')).href);
  // Shells report the long path; Windows runners set TEMP to an 8.3 short path.
  const scratch = realpathSync.native(mkdtempSync(path.join(os.tmpdir(), 'omniterm-release-pty-')));
  const env = { ...process.env, TERM: 'xterm-256color' };
  delete env.ELECTRON_RUN_AS_NODE; delete env.NODE_OPTIONS; delete env.NODE_PATH;
  if (process.platform !== 'win32') env.LANG = env.LC_ALL = process.platform === 'darwin' ? 'en_US.UTF-8' : 'C.UTF-8';
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
  const terminals = new Map();
  const host = new TerminalHost({
    executable: hostBinary,
    onData: ({ id, data }) => { const terminal = terminals.get(id); if (terminal) terminal.output += data; },
    onEvent: (event, params) => { const terminal = terminals.get(params?.id); if (event === 'session.exit' && terminal) { terminal.exited = true; terminal.exitCode = params.exitCode; } },
  });
  const open = async (file, args) => {
    const { id } = await host.request('session.create', { executable: file, args, cwd: scratch, env, kind: 'shell', title: 'verify' });
    const terminal = { id, output: '', exited: false };
    terminals.set(id, terminal);
    await host.request('session.start', { id });
    return terminal;
  };
  const write = (terminal, data) => host.request('session.write', { id: terminal.id, data });
  const wait = async (predicate, label, terminal) => {
    const deadline = Date.now() + 20000;
    while (!predicate()) {
      if (Date.now() >= deadline) throw new Error(`${label}\n${terminal.output.slice(-2500)}`);
      await delay(30);
    }
  };
  try {
    const windows = process.platform === 'win32';
    const terminal = await open(windows ? 'powershell.exe' : '/bin/bash', windows ? ['-NoLogo', '-NoProfile', '-NoExit'] : ['--noprofile', '--norc', '-i']);
    const token = randomBytes(5).toString('hex');
    const marker = name => `__OMNITERM_${name}_${token}__`;
    const print = name => windows ? `Write-Output ('__OMNITERM_' + '${name}_${token}__')` : `printf '%s%s\\n' '__OMNITERM_' '${name}_${token}__'`;
    const contains = value => stripVTControlCharacters(terminal.output).includes(value);
    write(terminal, `${print('READY')}; ${windows ? '(Get-Location).Path' : 'pwd'}\r`);
    await wait(() => contains(marker('READY')) && contains(scratch), 'Shell input and working directory', terminal);
    write(terminal, `${print('中文')}\r`);
    await wait(() => contains(marker('中文')), 'Unicode shell input', terminal);
    await host.request('session.resize', { id: terminal.id, cols: 112, rows: 37 });
    write(terminal, `${print('SLEEP')}; ${windows ? 'Start-Sleep -Seconds 30' : 'sleep 30'}\r`);
    await wait(() => contains(marker('SLEEP')), 'Interrupt target started', terminal);
    await delay(250); write(terminal, '\x03'); await delay(200);
    write(terminal, `${print('INTERRUPT')}\r`);
    await wait(() => contains(marker('INTERRUPT')), 'Ctrl+C returned a usable shell', terminal);
    await write(terminal, windows ? 'exit\r' : 'exit\r');
    await wait(() => terminal.exited, 'Shell exit', terminal);
    await host.request('session.close', { id: terminal.id });
    const agent = await open(path.join(resources, 'omniterm-agent/bin', windows ? 'codex.exe' : 'codex'), ['--version']);
    await wait(() => agent.exited, 'Bundled Agent --version', agent);
    assert.equal(agent.exitCode, 0, agent.output);
    assert.match(stripVTControlCharacters(agent.output), /codex-cli\s+\S+/);
    console.log(`PASS: packaged Electron ${process.versions.electron}, terminal host, shell/pwd/Unicode, resize request, Ctrl+C, exit, and bundled Agent --version (${process.platform}/${process.arch})`);
  } finally {
    host.dispose();
    // Windows holds the directory briefly after its last shell exits.
    rmSync(scratch, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  }
  process.exit(0);
}

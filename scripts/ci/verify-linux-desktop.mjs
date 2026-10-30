#!/usr/bin/env node
// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
// Artifact-only smoke test. No source modules or development dependencies are
// imported: the bundle supplies both Electron and the node-pty native module.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { accessSync, constants, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, statSync } from "node:fs";
import { createRequire } from "node:module";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { stripVTControlCharacters } from "node:util";

const scriptPath = fileURLToPath(import.meta.url);
const childFlag = "--inside-packaged-electron";
const help = `Usage: node scripts/ci/verify-linux-desktop.mjs APP_DIRECTORY

Examples:
  node scripts/ci/verify-linux-desktop.mjs /opt/omniterm
  node scripts/ci/verify-linux-desktop.mjs dist/desktop/omniTerm-linux-x64

Runs the packaged Electron in Node mode and loads the packaged node-pty only.
Checks renderer assets, native ABI, real Bash input/UTF-8 editing, resize,
Ctrl+C, process cleanup, and the bundled Agent's --version through a PTY.
No graphical display, remote host, or model credentials are required.
`;

function inside(root, candidate) {
  const relative = path.relative(root, candidate);
  return relative === "" || (!path.isAbsolute(relative) && relative !== ".." && !relative.startsWith(`..${path.sep}`));
}

function bundledFile(root, relative, executable = false) {
  const filename = realpathSync(path.join(root, relative));
  assert.ok(inside(root, filename), `Bundle asset escapes the application directory: ${relative}`);
  assert.ok(statSync(filename).isFile(), `Not a file: ${filename}`);
  accessSync(filename, executable ? constants.X_OK : constants.R_OK);
  return filename;
}

function verifyAssets(root) {
  const executable = bundledFile(root, "omniTerm", true);
  const appRoot = path.join(root, "resources/app");
  const packageJson = bundledFile(root, "resources/app/package.json");
  const metadata = JSON.parse(readFileSync(packageJson, "utf8"));
  assert.equal(metadata.name, "omniterm-desktop", "Unexpected packaged application");
  assert.ok(metadata.version, "Application version is missing");
  for (const relative of ["resources/app/build/main.mjs", "resources/app/build/preload.cjs", "LICENSE", "LICENSES.chromium.html"]) bundledFile(root, relative);

  const renderer = bundledFile(root, "resources/app/build-renderer/index.html");
  const rendererRoot = path.dirname(renderer);
  const assets = [...readFileSync(renderer, "utf8").matchAll(/(?:src|href)=["']([^"']+)["']/g)].map((match) => match[1]);
  assert.ok(assets.some((asset) => /\.js(?:\?|$)/.test(asset)), "Renderer has no JavaScript asset");
  assert.ok(assets.some((asset) => /\.css(?:\?|$)/.test(asset)), "Renderer has no stylesheet asset");
  for (const asset of assets) {
    const url = new URL(asset, pathToFileURL(renderer));
    assert.equal(url.protocol, "file:", `Renderer asset is not local: ${asset}`);
    const filename = realpathSync(fileURLToPath(url));
    assert.ok(inside(rendererRoot, filename), `Renderer asset escapes its packaged directory: ${asset}`);
    assert.ok(statSync(filename).isFile(), `Missing renderer asset: ${asset}`);
  }

  const agentRoot = path.join(root, "resources/omniterm-agent");
  const manifest = JSON.parse(readFileSync(bundledFile(root, "resources/omniterm-agent/codex-package.json"), "utf8"));
  assert.equal(manifest.layoutVersion, 1, "Unsupported Agent package layout");
  assert.equal(manifest.variant, "codex", "Unexpected Agent package variant");
  assert.equal(manifest.entrypoint, "bin/codex", "Unexpected Agent entrypoint");
  const architecture = { x64: "x86_64", arm64: "aarch64" }[process.arch];
  assert.ok(architecture, `Unsupported Linux architecture: ${process.arch}`);
  assert.equal(manifest.target, `${architecture}-unknown-linux-gnu`, "Agent target must match this Linux runtime");
  const agent = bundledFile(root, "resources/omniterm-agent/bin/codex", true);
  for (const relative of ["bin/codex-code-mode-host", "codex-path/rg", "codex-resources/bwrap"]) bundledFile(root, `resources/omniterm-agent/${relative}`, true);
  return { executable, appRoot, packageJson, metadata, agentRoot, agent, assets };
}

const delay = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));
const outputLines = (text) => stripVTControlCharacters(text).replaceAll("\r", "").split("\n");
const shellPrint = (parts) => `printf '${"%s".repeat(parts.length)}\\n' ${parts.map((part) => `'${part}'`).join(" ")}`;

async function waitFor(condition, label, terminal, timeout = 15000) {
  const deadline = Date.now() + timeout;
  while (!condition()) {
    if (Date.now() >= deadline) throw new Error(`${label}\nLast PTY output:\n${terminal?.output.slice(-4000) || "(none)"}`);
    await delay(25);
  }
}

async function verifyInPackagedRuntime(root) {
  assert.equal(process.platform, "linux", "This smoke test must run on Linux");
  const bundle = verifyAssets(root);
  assert.ok(process.versions.electron, "The test is not running inside Electron");
  assert.equal(realpathSync(process.execPath), bundle.executable, "The runtime is not this application's packaged Electron");

  // resolve() can otherwise walk up to a development node_modules. Validate
  // the real path before executing the module, including any symlink target.
  const expectedPtyRoot = path.join(bundle.appRoot, "node_modules/node-pty");
  assert.equal(realpathSync(expectedPtyRoot), expectedPtyRoot, "Packaged node-pty must not be a symlink to development dependencies");
  const require = createRequire(bundle.packageJson);
  const ptyEntry = realpathSync(require.resolve("node-pty"));
  assert.ok(inside(expectedPtyRoot, ptyEntry), `node-pty resolved outside the package: ${ptyEntry}`);
  const pty = require(ptyEntry);
  const nativeModules = Object.keys(require.cache).filter((filename) => filename.endsWith(".node")).map((filename) => realpathSync(filename));
  assert.ok(nativeModules.length, "node-pty did not load a native module");
  for (const filename of nativeModules) assert.ok(inside(expectedPtyRoot, filename), `Native module loaded outside packaged node-pty: ${filename}`);
  console.log(`Runtime: ${bundle.executable}`);
  console.log(`Electron ${process.versions.electron}; Node ${process.versions.node}; ABI ${process.versions.modules}; ${process.arch}`);
  console.log(`Packaged node-pty: ${ptyEntry}`);
  console.log(`Packaged native module: ${nativeModules.join(", ")}`);
  console.log(`PASS: packaged application ${bundle.metadata.version}, renderer assets (${bundle.assets.length}), and Agent resources`);

  const scratch = mkdtempSync(path.join(os.tmpdir(), "omniterm-packaged-pty-"));
  const live = new Set();
  const env = {
    ...process.env,
    PATH: "/usr/bin:/bin",
    TERM: "xterm-256color",
    COLORTERM: "truecolor",
    TERM_PROGRAM: "omniTerm",
    LANG: "C.UTF-8",
    LC_ALL: "C.UTF-8",
    // --version needs no credentials. Any incidental Agent state stays in
    // this test's disposable directory, away from a real user account.
    CODEX_HOME: path.join(scratch, "agent-home"),
  };
  delete env.ELECTRON_RUN_AS_NODE;
  delete env.NODE_OPTIONS;
  delete env.NODE_PATH;
  mkdirSync(env.CODEX_HOME, { mode: 0o700 });
  const stop = (terminal) => {
    if (terminal.exited) return;
    try { process.kill(-terminal.pty.pid, "SIGHUP"); } catch { /* Already gone. */ }
    try { terminal.pty.kill(); } catch { /* Already gone. */ }
  };
  const cleanup = () => { for (const terminal of live) stop(terminal); };
  const interrupt = () => { cleanup(); process.exitCode = 130; };
  const terminate = () => { cleanup(); process.exitCode = 143; };
  process.once("SIGINT", interrupt);
  process.once("SIGTERM", terminate);
  const open = (executable, args) => {
    const terminal = { pty: pty.spawn(executable, args, { name: "xterm-256color", cols: 120, rows: 32, cwd: scratch, env }), output: "", exited: false, exitCode: null, signal: null };
    live.add(terminal);
    terminal.pty.onData((data) => { terminal.output += data; });
    terminal.pty.onExit(({ exitCode, signal }) => { terminal.exited = true; terminal.exitCode = exitCode; terminal.signal = signal; live.delete(terminal); });
    return terminal;
  };
  try {
    const terminal = open("/bin/bash", ["--noprofile", "--norc", "-i"]);
    const token = randomUUID().replaceAll("-", "").slice(0, 12);
    const marker = (name) => `__OMNITERM_${name}_${token}__`;
    const print = (name) => shellPrint(["__OMNITERM_", `${name}_${token}__`]);
    const hasLine = (line) => outputLines(terminal.output).includes(line);

    // No full marker appears in the input command, so command echo cannot pass
    // any assertion, with or without bracketed-paste control sequences.
    terminal.pty.write(`${print("READY")}; pwd\r`);
    await waitFor(() => hasLine(marker("READY")) && hasLine(scratch), "Packaged PTY must execute English input and pwd", terminal);
    terminal.pty.write(`${print("中文输入")}\r`);
    await waitFor(() => hasLine(marker("中文输入")), "Packaged PTY must preserve Chinese input", terminal);
    terminal.pty.write(`${shellPrint(["__OMNITERM_中文退格错\x7f", `_${token}__`])}\r`);
    await waitFor(() => hasLine(marker("中文退格")), "Backspace must delete a complete Chinese character", terminal);
    console.log("PASS: real Bash input, working directory, Chinese input and Unicode backspace");

    terminal.pty.resize(112, 37);
    terminal.pty.write("stty size\r");
    await waitFor(() => hasLine("37 112"), "Packaged PTY resize must reach the shell", terminal);
    terminal.pty.write(`${print("SLEEP_STARTED")}; sleep 30\r`);
    await waitFor(() => hasLine(marker("SLEEP_STARTED")), "The interrupt target must start", terminal);
    await delay(200);
    terminal.pty.write("\x03");
    await delay(150);
    terminal.pty.write(`${print("INTERRUPT_OK")}\r`);
    await waitFor(() => hasLine(marker("INTERRUPT_OK")), "Ctrl+C must interrupt sleep and return to a usable shell", terminal);
    stop(terminal);
    await waitFor(() => terminal.exited, "Closing the PTY must report its shell's exit", terminal);
    await waitFor(() => {
      try { process.kill(terminal.pty.pid, 0); return false; }
      catch (error) { if (error.code === "ESRCH") return true; throw error; }
    }, "Closing the PTY must remove its shell process", terminal);
    console.log("PASS: resize, Ctrl+C, PTY close, and shell process cleanup");

    const agent = open(bundle.agent, ["--version"]);
    await waitFor(() => agent.exited, "Bundled Agent --version must exit through the packaged PTY", agent);
    assert.equal(agent.exitCode, 0, `Bundled Agent failed: ${agent.output}`);
    const version = outputLines(agent.output).find((line) => /^codex-cli\s+\S+/.test(line));
    assert.ok(version, `Bundled Agent did not report its version: ${agent.output}`);
    console.log(`PASS: bundled Agent through packaged PTY: ${version}`);
    console.log(`PASS: Linux desktop artifact smoke completed for ${root}`);
  } finally {
    cleanup();
    process.removeListener("SIGINT", interrupt);
    process.removeListener("SIGTERM", terminate);
    rmSync(scratch, { recursive: true, force: true });
  }
}

async function main() {
  const args = process.argv.slice(2);
  if (args.length === 1 && ["--help", "-h"].includes(args[0])) { console.log(help); return; }
  const child = args[0] === childFlag;
  if (child) args.shift();
  assert.equal(args.length, 1, help);
  assert.equal(process.platform, "linux", "Run this script on the Linux machine where the application artifact will run");
  const root = realpathSync(path.resolve(args[0]));
  assert.ok(statSync(root).isDirectory(), "Application path must be a directory");
  if (child) { await verifyInPackagedRuntime(root); return; }
  const executable = bundledFile(root, "omniTerm", true);
  const env = { ...process.env, ELECTRON_RUN_AS_NODE: "1" };
  delete env.NODE_OPTIONS;
  delete env.NODE_PATH;
  // No require("electron") here: the executable is always from the artifact.
  const runtime = spawn(executable, [scriptPath, childFlag, root], { env, stdio: "inherit", shell: false });
  const interrupt = () => runtime.kill("SIGINT");
  const terminate = () => runtime.kill("SIGTERM");
  process.on("SIGINT", interrupt);
  process.on("SIGTERM", terminate);
  try {
    process.exitCode = await new Promise((resolve, reject) => {
      runtime.once("error", reject);
      runtime.once("exit", (code, signal) => resolve(code ?? (signal === "SIGINT" ? 130 : 1)));
    });
  } finally {
    process.removeListener("SIGINT", interrupt);
    process.removeListener("SIGTERM", terminate);
  }
}

main().catch((error) => { console.error(`FAIL: ${error.message}`); process.exitCode = 1; });

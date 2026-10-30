import assert from "node:assert/strict";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { buildCommandPlan } from "../electron/command-plan.mjs";
import { SessionManager } from "../electron/session-manager.mjs";
import { TerminalHost, resolveTerminalHost } from "../electron/terminal-host.mjs";
import { CURSOR_QUERY, CURSOR_REPLY, hasOutputLine, printParts } from "./pty-output.mjs";

// Drives the built host through a real PTY or ConPTY, without Electron.
const here = fileURLToPath(import.meta.url);
const desktopRoot = path.resolve(path.dirname(here), "..");
const repositoryRoot = path.resolve(desktopRoot, "..");
const packagedOutput = path.join(repositoryRoot, "dist/terminal-host", process.platform === "win32" ? "omniterm-terminal-host.exe" : "omniterm-terminal-host");
const env = { ...process.env };
let executable;
try { executable = resolveTerminalHost({ desktopRoot, env: { ...env, OMNITERM_TERMINAL_HOST: packagedOutput } }); }
catch { executable = resolveTerminalHost({ desktopRoot, env }); }
let output = "";
const exits = [];
const host = new TerminalHost({ executable, onData: ({ id, data }) => { output += data; if (data.includes(CURSOR_QUERY)) host.request("session.write", { id, data: CURSOR_REPLY, reply: true }).catch(() => {}); }, onEvent: (event, params) => { if (event === "session.exit") exits.push(params); } });
const manager = new SessionManager({ host, planner: (request) => buildCommandPlan(request, { repositoryRoot, defaultCwd: repositoryRoot, env }) });
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const waitFor = async (predicate, label) => {
  const deadline = Date.now() + 15000;
  while (!predicate() && Date.now() < deadline) await delay(50);
  assert.ok(predicate(), `${label}\n${output.slice(-2500)}`);
};
try {
  const session = await manager.create({ kind: "shell" });
  await manager.resize({ id: session.id, cols: 100, rows: 30 });
  const running = await manager.start(session.id);
  const write = (data) => manager.write({ id: session.id, data });
  const windows = process.platform === "win32";
  await write(`${printParts(["__OMNITERM_", "PTY_READY__"], windows)}; ${windows ? "(Get-Location).Path" : "pwd"}\r`);
  await waitFor(() => hasOutputLine(output, "__OMNITERM_PTY_READY__") && hasOutputLine(output, repositoryRoot), "The shell must execute input and report its working directory");
  await manager.resize({ id: session.id, cols: 112, rows: 37 });
  if (!windows) {
    await write("stty size\r");
    await waitFor(() => hasOutputLine(output, "37 112"), "PTY resize must reach the shell");
  }
  await write(`${printParts(["__OMNITERM_", "中文__"], windows)}\r`);
  await waitFor(() => hasOutputLine(output, "__OMNITERM_中文__"), "Unicode must survive the host");
  await write(windows ? "Start-Sleep -Seconds 30\r" : "sleep 30\r");
  await delay(300);
  await write("\x03");
  await delay(200);
  await write(`${printParts(["__OMNITERM_", "INTERRUPT_OK__"], windows)}\r`);
  await waitFor(() => hasOutputLine(output, "__OMNITERM_INTERRUPT_OK__"), "Ctrl+C must return to a usable shell");
  await write("exit\r");
  await waitFor(() => exits.length === 1, "Shell exit must reach the client");
  const second = await manager.create({ kind: "shell" });
  const started = await manager.start(second.id);
  await manager.close(second.id);
  if (!windows) {
    const stopped = (pid) => { try { process.kill(pid, 0); return false; } catch (error) { return error.code === "ESRCH"; } };
    await waitFor(() => stopped(started.pid), "Closing a session must stop its shell");
    assert.ok(stopped(running.pid), "An exited shell must be gone");
  }
  console.log("PASS: real PTY input, working directory, resize, Unicode, Ctrl+C, exit, and process cleanup");
} finally {
  manager.dispose();
}

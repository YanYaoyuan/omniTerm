import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { buildCommandPlan } from "../electron/command-plan.mjs";
import { SessionManager } from "../electron/session-manager.mjs";
import { shellIntegrationScript } from "../electron/shell-integration.mjs";
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
const operations = new Map();
const exits = [];
const host = new TerminalHost({ executable, onData: ({ id, data }) => { output += data; if (data.includes(CURSOR_QUERY)) host.request("session.write", { id, data: CURSOR_REPLY, reply: true }).catch(() => {}); }, onEvent: (event, params) => { if (event === "session.exit") exits.push(params); if (event === "operation.update") operations.set(params.operationId, params); } });
const manager = new SessionManager({ host, planner: (request) => buildCommandPlan(request, { repositoryRoot, defaultCwd: repositoryRoot, env }) });
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const waitFor = async (predicate, label) => {
  const deadline = Date.now() + 15000;
  while (!predicate() && Date.now() < deadline) await delay(50);
  assert.ok(predicate(), `${label}\n${output.slice(-2500)}`);
};
const interaction = async (id) => (await host.request("workspace.list")).terminals.find((terminal) => terminal.ref.sessionId === id).interaction.state;

// A shell owning the foreground is not at a prompt: `read` must not receive a command.
async function verifyReadiness() {
  const session = await host.request("session.create", { executable: "/bin/bash", args: ["--noprofile", "--norc", "-i"], cwd: repositoryRoot, env: { PATH: env.PATH, TERM: "xterm-256color", PS1: "$ " }, kind: "shell", title: "readiness", connection: {} });
  const { id, shellNonce } = session;
  await host.request("session.start", { id });
  const send = (data) => host.request("session.write", { id, data });
  const proposed = async (call, command, mode) => {
    const { operationId } = await host.request("operation.propose", { taskId: "readiness", callId: call, terminal: id, command });
    await host.request("operation.approve", { operationId, scope: "once", mode });
    await host.request("operation.execute", { operationId });
    await waitFor(() => ["blocked", "completed", "invalidated", "outcomeUnknown", "staged"].includes(operations.get(operationId)?.status), `Operation ${call} must finish`);
    return operations.get(operationId);
  };
  await waitFor(() => output.includes("$ "), "The readiness shell must show a prompt");
  await host.request("task.begin", { taskId: "readiness", context: [], targets: [{ sessionId: id, epoch: 1 }] });
  await send("read -r -p 'ASK> ' answer; printf 'REPLY=<%s>\\n' \"$answer\"\r");
  await waitFor(() => output.includes("ASK> "), "The builtin must be waiting");
  await delay(200);
  assert.notEqual(await interaction(id), "ready", "A running builtin must not count as idle without marks");
  const blocked = await proposed("builtin", "printf 'BROKER_%s\\n' EXECUTED", "run");
  assert.equal(blocked.status, "blocked");
  assert.ok(!hasOutputLine(output, "BROKER_EXECUTED") && !output.includes("REPLY=<printf"), "The command must not reach the builtin");
  await send("no\r");
  await waitFor(() => output.includes("REPLY=<no>"), "The builtin must still take the user's answer");
  await delay(200);
  await send(`${shellIntegrationScript(shellNonce)}\r`);
  await delay(500);
  // Stock macOS bash predates PS0: it must refuse marks and stay manual.
  const [major, minor] = execFileSync("/bin/bash", ["-c", "echo ${BASH_VERSINFO[0]} ${BASH_VERSINFO[1]}"], { encoding: "utf8" }).trim().split(" ").map(Number);
  if (major < 4 || (major === 4 && minor < 4)) {
    assert.equal(await interaction(id), "unknown", "A bash without PS0 must stay manual");
    return;
  }
  assert.equal(await interaction(id), "ready", "Marks must report the prompt");
  await send("read -r -p 'AGAIN> ' answer\r");
  await waitFor(() => output.includes("AGAIN> "), "The builtin must be waiting under marks");
  await delay(200);
  assert.equal(await interaction(id), "busy", "Marks must report a running builtin as busy");
  await send("yes\r");
  await delay(300);
  assert.equal(await interaction(id), "ready", "Marks must report the returned prompt");
  const done = await proposed("marked", "(exit 7)", "run");
  assert.deepEqual([done.status, done.exitCode], ["completed", 7]);
}

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
  if (!windows) await verifyReadiness();
  console.log("PASS: real PTY input, working directory, resize, Unicode, Ctrl+C, exit, process cleanup, and command readiness");
} finally {
  manager.dispose();
}

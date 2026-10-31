import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
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
const waitForAsync = async (predicate, label) => {
  const deadline = Date.now() + 15000;
  while (Date.now() < deadline) { if (await predicate()) return; await delay(50); }
  assert.fail(`${label}\n${output.slice(-1500)}`);
};
const interaction = async (id) => (await host.request("workspace.list")).terminals.find((terminal) => terminal.ref.sessionId === id).interaction.state;

const FINAL_OPERATION = ["blocked", "cancelled", "completed", "invalidated", "outcomeUnknown", "staged"];

// A shell owning the foreground is not at a prompt: `read` must not receive a command, and a
// continuation prompt must not inherit the primary prompt before it.
async function verifyReadiness(executable, args, supportsMarks) {
  const taskId = `readiness-${executable}`;
  const before = output.length;
  const recent = () => output.slice(before);
  const session = await host.request("session.create", { executable, args, cwd: repositoryRoot, env: { PATH: env.PATH, TERM: "xterm-256color", PS1: "$ ", PROMPT: "$ " }, kind: "shell", title: "readiness", connection: {} });
  const { id, shellNonce } = session;
  await host.request("session.start", { id });
  const send = (data) => host.request("session.write", { id, data });
  const state = (expected, label) => waitForAsync(async () => await interaction(id) === expected, `${label}: expected ${expected}`);
  const approved = async (call, command) => {
    const { operationId } = await host.request("operation.propose", { taskId, callId: call, terminal: id, command });
    await host.request("operation.approve", { operationId, scope: "once", mode: "run" });
    await host.request("operation.execute", { operationId });
    return operationId;
  };
  const finished = async (operationId, label) => {
    await waitFor(() => FINAL_OPERATION.includes(operations.get(operationId)?.status), `${label} must finish`);
    return operations.get(operationId);
  };
  await waitFor(() => recent().includes("$ "), "The readiness shell must show a prompt");
  await host.request("task.begin", { taskId, context: [], targets: [{ sessionId: id, epoch: 1 }] });
  await send(`${executable.endsWith("zsh") ? "read -r 'answer?ASK> '" : "read -r -p 'ASK> ' answer"}; printf 'REPLY=<%s>\\n' "$answer"\r`);
  await waitFor(() => recent().includes("ASK> "), "The builtin must be waiting");
  await delay(200);
  assert.notEqual(await interaction(id), "ready", "A running builtin must not count as idle without marks");
  const blocked = await finished(await approved("builtin", "printf 'BROKER_%s\\n' EXECUTED"), "The unmarked command");
  assert.equal(blocked.status, "blocked");
  assert.ok(!hasOutputLine(recent(), "BROKER_EXECUTED") && !recent().includes("REPLY=<printf"), "The command must not reach the builtin");
  await send("no\r");
  await waitFor(() => recent().includes("REPLY=<no>"), "The builtin must still take the user's answer");
  await delay(200);
  await send(`${shellIntegrationScript(shellNonce)}\r`);
  await delay(500);
  if (!supportsMarks) {
    assert.equal(await interaction(id), "unknown", "A bash without PS0 must stay manual");
    return;
  }
  await state("ready", "Marks must report the prompt");
  await send(`${executable.endsWith("zsh") ? "read -r 'answer?AGAIN> '" : "read -r -p 'AGAIN> ' answer"}\r`);
  await waitFor(() => recent().includes("AGAIN> "), "The builtin must be waiting under marks");
  await state("busy", "Marks must report a running builtin");
  await send("yes\r");
  await state("busy", "An answer could be type-ahead, so the next prompt is not trusted");
  await send("\r");
  await state("ready", "One line at the prompt must restore readiness");

  // Each stays busy at its secondary prompt, and only its real end brings a new prompt back.
  const unfinished = [
    ["an unclosed quote", "echo 'open\r", ["more\r", "close'\r"]],
    ["a line continuation", "echo joined\\\r", ["line\r"]],
    ["a heredoc", "cat <<OMNITERM_END\r", ["body\r", "OMNITERM_END\r"]],
  ];
  let index = 0;
  for (const [name, opening, rest] of unfinished) {
    const marker = `${++index}`;
    await send(opening);
    await delay(200);
    await state("busy", `${name} must not look idle`);
    const held = await approved(`held-${marker}`, `printf 'BROKER_%s\\n' HELD${marker}`);
    await delay(300);
    assert.equal((await host.request("operation.result", { operationId: held })).operation.status, "queued", `${name} must hold the command back`);
    assert.equal((await host.request("operation.cancel", { operationId: held })).status, "cancelled");
    assert.ok(!recent().includes(`HELD${marker}`), `${name} must not receive the command`);
    for (const line of rest) await send(line);
    await state("ready", `${name} must recover after it ends`);
  }
  await send("echo 'interrupted\r");
  await delay(200);
  await state("busy", "An interrupted quote must start out busy");
  await send("\x03");
  await state("ready", "Ctrl+C at a secondary prompt must return to the primary prompt");

  const done = await finished(await approved("marked", "(exit 7)"), "The marked command");
  assert.deepEqual([done.status, done.exitCode], ["completed", 7]);
  await state("ready", "A finished command must return to the prompt");
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
  if (!windows) {
    const [major, minor] = execFileSync("/bin/bash", ["-c", "echo ${BASH_VERSINFO[0]} ${BASH_VERSINFO[1]}"], { encoding: "utf8" }).trim().split(" ").map(Number);
    await verifyReadiness("/bin/bash", ["--noprofile", "--norc", "-i"], major > 4 || (major === 4 && minor >= 4));
    for (const zsh of ["/bin/zsh", "/usr/bin/zsh"]) if (existsSync(zsh)) { await verifyReadiness(zsh, ["-f", "-i"], true); break; }
  }
  console.log("PASS: real PTY input, working directory, resize, Unicode, Ctrl+C, exit, process cleanup, and command readiness");
} finally {
  manager.dispose();
}

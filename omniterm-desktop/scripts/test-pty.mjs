import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { appendFileSync, existsSync, mkdtempSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { buildCommandPlan } from "../electron/command-plan.mjs";
import { MonitorService } from "../electron/monitor-service.mjs";
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
const monitorForward = new Set();
const exits = [];
const host = new TerminalHost({ executable, onData: ({ id, data }) => { output += data; if (data.includes(CURSOR_QUERY)) host.request("session.write", { id, data: CURSOR_REPLY, reply: true }).catch(() => {}); }, onEvent: (event, params) => { if (event === "session.exit") exits.push(params); if (event === "operation.update") { operations.set(params.operationId, params); for (const listener of monitorForward) listener(params); } } });
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

// SSH is judged by session kind, so a plain shell stands in for the remote end here.
async function verifySshStaging() {
  const taskId = "ssh-staging";
  const windows = process.platform === "win32";
  const before = output.length;
  const plan = await buildCommandPlan({ kind: "shell" }, { repositoryRoot, defaultCwd: repositoryRoot, env });
  const { id } = await host.request("session.create", { executable: plan.executable, args: plan.args, cwd: plan.cwd, env: plan.env, kind: "ssh", title: "ssh-staging", connection: {} });
  await host.request("session.start", { id });
  await delay(1500);
  await host.request("task.begin", { taskId, context: [], targets: [{ sessionId: id, epoch: 1 }] });
  const propose = (callId, command) => host.request("operation.propose", { taskId, callId, terminal: id, command });
  const command = printParts(["__OMNITERM_", "STAGED__"], windows);
  const first = await propose("first", command);
  await assert.rejects(host.request("operation.approve", { operationId: first.operationId, scope: "once", mode: "run" }), /never submitted/);
  await host.request("operation.approve", { operationId: first.operationId, scope: "once", mode: "stage" });
  await host.request("operation.execute", { operationId: first.operationId });
  await waitFor(() => operations.get(first.operationId)?.status === "staged", "The command must be staged");
  await delay(500);
  assert.ok(!hasOutputLine(output.slice(before), "__OMNITERM_STAGED__"), "A staged command must not run");
  const second = await propose("second", "printf second");
  await host.request("operation.approve", { operationId: second.operationId, scope: "once", mode: "stage" });
  await host.request("operation.execute", { operationId: second.operationId });
  await waitFor(() => operations.get(second.operationId)?.status === "blocked", "Staged text must block what follows it");
  await host.request("session.write", { id, data: "\r" });
  await waitFor(() => hasOutputLine(output.slice(before), "__OMNITERM_STAGED__"), "Enter must submit the staged command");
  assert.ok(!output.slice(before).includes("printf second"), "The second command must never join the first");
  await host.request("session.close", { id });
}

const FINAL_OPERATION = ["blocked", "cancelled", "completed", "delivered", "invalidated", "outcomeUnknown", "staged"];

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
  await state("busy", "A prompt right after an answer cannot vouch for input still in flight");
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
  // Queued lines run one by one, and the last may be unfinished when the prompts stop.
  const holdsBack = async (name, call) => {
    await state("busy", `${name} must not look idle`);
    const held = await approved(call, `printf 'BROKER_%s\\n' HELD_${call}`);
    await delay(300);
    assert.equal((await host.request("operation.result", { operationId: held })).operation.status, "queued", `${name} must hold the command back`);
    assert.equal((await host.request("operation.cancel", { operationId: held })).status, "cancelled");
    assert.ok(!recent().includes(`HELD_${call}`), `${name} must not receive the command`);
  };
  await send("sleep 1\r");
  await delay(200);
  await send("true\r");
  await send("echo 'queued\r");
  await delay(1500);
  await holdsBack("Input queued behind a running command", "queue");
  await send("close'\r");
  await state("ready", "Queued input must recover once it ends");

  // Input can also be queued before the command-start mark, behind a slow preexec hook.
  if (executable.endsWith("zsh")) {
    await send("__slow() { sleep 1; }; preexec_functions=(__slow $preexec_functions)\r");
    await state("ready", "The slow hook must install");
    await send("true\r");
    await delay(200);
    await send("echo 'slow\r");
    await delay(2000);
    await holdsBack("Input queued behind a slow hook", "slow");
    await send("close'\r");
    await delay(1500);
    await send("preexec_functions=(${preexec_functions:#__slow})\r");
    await delay(1500);
    await state("ready", "The slow hook must uninstall");
  }

  // An answer the program already consumed leaves nothing queued, so no extra Enter is needed.
  await send(`${executable.endsWith("zsh") ? "read -r 'answer?LATER> '" : "read -r -p 'LATER> ' answer"}; sleep 2\r`);
  await waitFor(() => recent().includes("LATER> "), "The answered builtin must be waiting");
  await send("yes\r");
  await delay(2500);
  await state("ready", "An answer long before the prompt must need no extra Enter");

  // Program input goes to a running program, and only there.
  const sendText = async (call, text, mode) => {
    const { operationId } = await host.request("operation.proposeInput", { taskId, callId: call, terminal: id, text, appendEnter: mode === "run" });
    await host.request("operation.approve", { operationId, scope: "once", mode });
    await host.request("operation.execute", { operationId });
    return finished(operationId, `Program input ${call}`);
  };
  const atPrompt = await sendText("at-prompt", "printf 'SHELL_%s\\n' RAN", "run");
  assert.equal(atPrompt.status, "blocked", "Program input must never reach a shell prompt");
  assert.ok(!recent().includes("SHELL_RAN"));
  await send("cat\r");
  await state("busy", "A running program must hold the terminal");
  assert.equal((await sendText("typed", "partial", "stage")).status, "delivered");
  assert.equal((await sendText("entered", " line", "run")).status, "delivered");
  await waitFor(() => hasOutputLine(recent(), "partial line"), "The program must receive the line, and echo it back");
  await send("\x04");
  await delay(1700);
  await send("\r");
  await state("ready", "The shell must return after the program ends");

  // A monitor reads the host, and its approved action goes through the broker on its own.
  const log = path.join(mkdtempSync(path.join(os.tmpdir(), "omniterm-monitor-")), "app.log");
  writeFileSync(log, "start\n");
  const monitors = new MonitorService({ host, run: async (_target, script) => execFileSync("sh", ["-c", script], { encoding: "utf8" }) });
  const forward = monitorForward.add((operation) => monitors.operationChanged(operation));
  const monitor = monitors.propose({ host: "local", target: { kind: "log", path: log }, conditions: [{ field: "text", op: "contains", value: "ERROR" }], intervalSeconds: 1, durationMinutes: 1, action: { command: "printf 'ACTION_%s\\n' RAN", terminal: { sessionId: id, epoch: 1 }, maxRuns: 1 } });
  await monitors.approve(monitor.id);
  await delay(1300);
  assert.ok(!recent().includes("ACTION_RAN"), "Nothing may happen before the condition holds");
  appendFileSync(log, "an ERROR line\n");
  await waitFor(() => hasOutputLine(recent(), "ACTION_RAN"), "A trigger must run the approved command in the approved terminal");
  await waitFor(() => monitors.get(monitor.id).history.some(({ message }) => message.includes("completed")), "The action's result must reach the history");
  await monitors.stop(monitor.id);
  monitorForward.delete(forward);
  await state("ready", "The shell must be ready after the action");

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
  await verifySshStaging();
  if (!windows) {
    const [major, minor] = execFileSync("/bin/bash", ["-c", "echo ${BASH_VERSINFO[0]} ${BASH_VERSINFO[1]}"], { encoding: "utf8" }).trim().split(" ").map(Number);
    await verifyReadiness("/bin/bash", ["--noprofile", "--norc", "-i"], major > 4 || (major === 4 && minor >= 4));
    for (const zsh of ["/bin/zsh", "/usr/bin/zsh"]) if (existsSync(zsh)) { await verifyReadiness(zsh, ["-f", "-i"], true); break; }
  }
  console.log("PASS: real PTY input, working directory, resize, Unicode, Ctrl+C, exit, process cleanup, and command readiness");
} finally {
  manager.dispose();
}

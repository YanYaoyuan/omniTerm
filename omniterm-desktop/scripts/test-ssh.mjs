// Real ssh, a private sshd and an injected-latency proxy, exercising approved execution over
// SSH. Unix only; needs sshd, ssh-keygen and a built terminal host. Nothing leaves loopback.
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { chmodSync, copyFileSync, existsSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { shellIntegrationScript } from "../electron/shell-integration.mjs";
import { TerminalHost, resolveTerminalHost } from "../electron/terminal-host.mjs";
import { hasOutputLine } from "./pty-output.mjs";

const desktopRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const packaged = path.resolve(desktopRoot, "../dist/terminal-host/omniterm-terminal-host");
const sshd = ["/usr/sbin/sshd", "/usr/bin/sshd"].find(existsSync);
if (!sshd) { console.log("SKIP: sshd is not installed"); process.exit(0); }
const SSHD_PORT = 22000 + (process.pid % 1000);
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const scratch = mkdtempSync(path.join(os.tmpdir(), "omniterm-ssh-"));
chmodSync(scratch, 0o700);
const file = (name) => path.join(scratch, name);
for (const name of ["host", "client"]) execFileSync("ssh-keygen", ["-q", "-t", "ed25519", "-N", "", "-f", file(name)]);
copyFileSync(file("client.pub"), file("authorized"));
writeFileSync(file("config"), `Port ${SSHD_PORT}\nListenAddress 127.0.0.1\nHostKey ${file("host")}\nAuthorizedKeysFile ${file("authorized")}\nStrictModes no\nPasswordAuthentication no\nUsePAM no\nPidFile ${file("pid")}\nLogLevel ERROR\n`);
const server = spawn(sshd, ["-D", "-f", file("config"), "-E", file("log")], { stdio: "ignore" });

// Forwards in order, holding each chunk for the current latency of its direction.
function createProxy() {
  const state = { up: 0, down: 0, holdDown: false, held: [], sockets: new Set() };
  const relay = (from, to, direction) => {
    let last = 0;
    from.on("data", (chunk) => {
      const forward = () => { if (!to.destroyed) to.write(chunk); };
      if (direction === "down" && state.holdDown) { state.held.push(forward); return; }
      const due = Math.max(Date.now() + state[direction], last);
      last = due;
      setTimeout(forward, due - Date.now());
    });
  };
  const listener = net.createServer((client) => {
    const upstream = net.connect(SSHD_PORT, "127.0.0.1");
    for (const socket of [client, upstream]) { state.sockets.add(socket); socket.on("error", () => {}); socket.on("close", () => { client.destroy(); upstream.destroy(); }); }
    relay(client, upstream, "up");
    relay(upstream, client, "down");
  });
  state.release = () => { state.holdDown = false; for (const forward of state.held.splice(0)) forward(); };
  state.disconnect = () => { for (const socket of state.sockets) socket.destroy(); };
  state.listen = () => new Promise((resolve) => listener.listen(0, "127.0.0.1", () => resolve(listener.address().port)));
  state.close = () => listener.close();
  return state;
}

const output = new Map();
const exits = new Set();
const operations = new Map();
const host = new TerminalHost({
  executable: resolveTerminalHost({ desktopRoot, env: existsSync(packaged) ? { ...process.env, OMNITERM_TERMINAL_HOST: packaged } : process.env }),
  onData: ({ id, data }) => output.set(id, (output.get(id) ?? "") + data),
  onEvent: (event, params) => { if (event === "operation.update") operations.set(params.operationId, params); if (event === "session.exit") exits.add(params.id); },
});
const until = async (predicate, label, timeoutMs = 20_000) => {
  const deadline = Date.now() + timeoutMs;
  for (;;) { const value = await predicate(); if (value) return value; if (Date.now() > deadline) throw new Error(`Timed out: ${label}`); await delay(40); }
};
const FINAL = ["blocked", "cancelled", "completed", "invalidated", "outcomeUnknown", "staged", "takenOver"];
let calls = 0;

async function connect(shellArgs, port) {
  const remote = shellArgs.join(" ");
  const { id, shellNonce } = await host.request("session.create", { executable: "ssh", args: ["-F", "none", "-tt", "-p", String(port), "-i", file("client"), "-o", "IdentitiesOnly=yes", "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null", "-o", "LogLevel=ERROR", `${os.userInfo().username}@127.0.0.1`, `env PS1='$ ' PROMPT='$ ' ${remote}`], cwd: scratch, env: { PATH: process.env.PATH, HOME: os.homedir(), TERM: "xterm-256color" }, kind: "ssh", title: "ssh", connection: {} });
  await host.request("session.start", { id });
  const terminal = async () => (await host.request("workspace.list")).terminals.find((item) => item.ref.sessionId === id);
  const state = async () => (await terminal()).interaction.state;
  const send = (data) => host.request("session.write", { id, data });
  const text = () => output.get(id) ?? "";
  await until(() => text().includes("$ "), "the remote prompt");
  await send(`${shellIntegrationScript(shellNonce)}\r`);
  await until(async () => await state() === "ready", "readiness marks");
  const taskId = `task-${id}`;
  await host.request("task.begin", { taskId, context: [], targets: [{ sessionId: id, epoch: 1 }] });
  const run = async (command) => {
    const { operationId } = await host.request("operation.propose", { taskId, callId: `call-${++calls}`, terminal: id, command });
    await host.request("operation.approve", { operationId, scope: "once", mode: "run" });
    await host.request("operation.execute", { operationId });
    return operationId;
  };
  const status = async (operationId) => (await host.request("operation.result", { operationId })).operation.status;
  const finished = (operationId, timeoutMs) => until(async () => { const value = await status(operationId); return FINAL.includes(value) ? value : null; }, "the operation must end", timeoutMs);
  const cancel = (operationId) => host.request("operation.cancel", { operationId });
  return { id, send, state, terminal, text, run, status, finished, cancel };
}

const failures = [];
async function check(name, body) {
  try { await body(); console.log(`PASS  ${name}`); }
  catch (error) { failures.push(name); console.log(`FAIL  ${name}: ${error.message.slice(0, 300)}`); }
}
const marker = () => `M${Math.random().toString(36).slice(2, 10).toUpperCase()}`;
const printMark = (mark) => `printf '%s_%s\\n' HELD ${mark}`;

// The command must not reach the shell while the construct is open, however it ends.
async function holdsBack(terminal, mark) {
  const operationId = await terminal.run(printMark(mark));
  await delay(700);
  const status = await terminal.status(operationId);
  assert.ok(["queued", "blocked"].includes(status), `status ${status}`);
  if (status === "queued") await terminal.cancel(operationId);
  assert.ok(!terminal.text().includes(mark), "the command reached the terminal");
}

async function scenarios(shellName, shellArgs, proxy, port, latency) {
  const label = `${shellName} ${latency}ms`;
  proxy.up = proxy.down = latency;
  const ready = async (terminal) => { await until(async () => await terminal.state() === "ready", "ready"); };
  const a = await connect(shellArgs, port);
  await check(`${label}: without the opt-in a run approval is refused`, async () => {
    assert.equal((await a.terminal()).autoSubmit, false);
    await assert.rejects(a.run("true"), /never submitted/);
  });
  await host.request("session.allowAutoSubmit", { id: a.id, enabled: true });
  await check(`${label}: approved commands run automatically and report status`, async () => {
    assert.equal((await a.terminal()).autoSubmit, true);
    const mark = marker();
    const operationId = await a.run(`printf '%s_%s\\n' RAN ${mark}; (exit 3)`);
    assert.equal(await a.finished(operationId), "completed");
    await until(() => hasOutputLine(a.text(), `RAN_${mark}`), "the output must arrive", 3000);
    assert.equal((await host.request("operation.result", { operationId })).operation.exitCode, 3);
  });
  for (const [name, open, close] of [
    ["an unclosed quote", "echo 'open\r", "close'\r"],
    ["a line continuation", "echo joined\\\r", "line\r"],
    ["a heredoc", "cat <<OMNITERM_END\r", "OMNITERM_END\r"],
  ]) {
    await check(`${label}: ${name} holds the command back`, async () => {
      await a.send(open);
      await delay(400 + latency * 2);
      try { await holdsBack(a, marker()); } finally { await a.send(close); }
      await ready(a);
    });
  }
  await check(`${label}: queued input holds the command back`, async () => {
    await a.send("sleep 1\r");
    await delay(200 + latency * 2);
    await a.send("true\r");
    await a.send("echo 'queued\r");
    await delay(1500 + latency * 2);
    try { await holdsBack(a, marker()); } finally { await a.send("close'\r"); }
    await delay(1700 + latency * 2);
    await a.send("\r");
    await ready(a);
  });
  await check(`${label}: a read builtin holds the command back`, async () => {
    await a.send(`${shellName === "zsh" ? "read -r 'answer?ASK> '" : "read -r -p 'ASK> ' answer"}\r`);
    await until(() => a.text().includes("ASK> "), "the builtin");
    await delay(latency * 2 + 100);
    try { await holdsBack(a, marker()); } finally { await a.send("done\r"); }
    await delay(1700 + latency * 2);
    await a.send("\r");
    await ready(a);
  });
  await check(`${label}: a full-screen program blocks without a probe`, async () => {
    await a.send("printf '\\033[?1049h'; sleep 2; printf '\\033[?1049l'\r");
    await until(async () => await a.state() === "fullscreen", "full screen");
    const operationId = await a.run("true");
    assert.equal(await a.finished(operationId), "blocked");
    await delay(2200 + latency * 2);
    await a.send("\r");
    await ready(a);
  });
  await check(`${label}: a nested shell holds the command back`, async () => {
    await a.send("/bin/sh\r");
    await delay(400 + latency * 2);
    try {
      assert.equal(await a.state(), "busy");
      await holdsBack(a, marker());
    } finally { await a.send("exit\r"); }
    await delay(1700 + latency * 2);
    await a.send("\r");
    await ready(a);
  });
  await check(`${label}: a lost reply blocks, and a late one changes nothing`, async () => {
    proxy.holdDown = true;
    const mark = marker();
    try {
      const operationId = await a.run(printMark(mark));
      assert.equal(await a.finished(operationId, 9000), "blocked");
    } finally { proxy.release(); }
    await delay(400 + latency * 2);
    assert.ok(!a.text().includes(`HELD_${mark}`));
    const next = marker();
    const again = await a.run(`printf '%s_%s\\n' NEXT ${next}`);
    assert.equal(await a.finished(again), "completed");
    await until(() => hasOutputLine(a.text(), `NEXT_${next}`), "the output must arrive", 3000);
  });
  if (shellName === "zsh") {
    await check(`${label}: input behind a slow preexec holds the command back`, async () => {
      await a.send("__slow() { sleep 1; }; preexec_functions=(__slow $preexec_functions)\r");
      await ready(a);
      await a.send("true\r");
      await delay(150);
      await a.send("echo 'slow\r");
      await delay(2200 + latency * 2);
      try { await holdsBack(a, marker()); } finally { await a.send("close'\r"); }
      await delay(1700 + latency * 2);
      await a.send("preexec_functions=(${preexec_functions:#__slow})\r");
      await delay(2400 + latency * 2);
      await a.send("\r");
      await ready(a);
    });
  }
  await check(`${label}: a vi keymap falls back to staging`, async () => {
    await a.send(`${shellName === "zsh" ? "setopt vi" : "set -o vi"}\r`);
    await until(async () => (await a.terminal()).autoSubmit === false, "the capability must drop");
    await assert.rejects(a.run("true"), /never submitted/);
    await a.send(`${shellName === "zsh" ? "setopt emacs" : "set -o emacs"}\r`);
    await until(async () => (await a.terminal()).autoSubmit === true, "the capability must return");
  });
  await host.request("session.close", { id: a.id });
}

async function inFlightGap(shellName, shellArgs, proxy, port) {
  // Sent after the shell's own queue check, and later than the in-flight guard can see.
  proxy.up = proxy.down = 0;
  const a = await connect(shellArgs, port);
  await host.request("session.allowAutoSubmit", { id: a.id, enabled: true });
  await check(`${shellName}: input landing after the queue check cannot be overwritten`, async () => {
    proxy.up = 3000;
    await a.send("sleep 2\r");
    await until(async () => await a.state() === "busy", "the command must start", 8000);
    await delay(3100);
    await a.send("echo 'late\r");
    await until(async () => await a.state() === "ready", "the prompt must claim readiness", 8000);
    const mark = marker();
    const operationId = await a.run(printMark(mark));
    // The late line reaches the shell before the probe does, and its secondary prompt ends
    // readiness, so the operation waits as for any busy terminal.
    await until(async () => await a.state() === "busy", "the secondary prompt must be seen", 8000);
    await delay(4000);
    assert.equal(await a.status(operationId), "queued");
    await a.cancel(operationId);
    assert.ok(!a.text().includes(mark), "the command reached the unfinished line");
  });
  proxy.up = 0;
  await host.request("session.close", { id: a.id });
}

async function disconnect(shellName, shellArgs, proxy, port) {
  proxy.up = proxy.down = 0;
  const a = await connect(shellArgs, port);
  await host.request("session.allowAutoSubmit", { id: a.id, enabled: true });
  await check(`${shellName}: a disconnect during the handshake never replays`, async () => {
    proxy.holdDown = true;
    const mark = marker();
    const operationId = await a.run(printMark(mark));
    await delay(300);
    proxy.held.length = 0;
    proxy.holdDown = false;
    proxy.disconnect();
    await until(() => exits.has(a.id), "the session must end");
    assert.ok(["blocked", "invalidated"].includes(await a.finished(operationId, 9000)));
    assert.ok(!a.text().includes(`HELD_${mark}`));
  });
  await host.request("session.close", { id: a.id });
}

try {
  await until(() => new Promise((resolve) => { const socket = net.connect(SSHD_PORT, "127.0.0.1"); socket.on("connect", () => { socket.destroy(); resolve(true); }); socket.on("error", () => resolve(false)); }), "sshd must listen");
  const proxy = createProxy();
  const port = await proxy.listen();
  const shells = [["bash", ["/bin/bash", "--noprofile", "--norc", "-i"]]];
  const bashVersion = execFileSync("/bin/bash", ["-c", "echo ${BASH_VERSINFO[0]}${BASH_VERSINFO[1]}"], { encoding: "utf8" }).trim();
  if (Number(bashVersion) < 44) shells.length = 0;
  const zsh = ["/bin/zsh", "/usr/bin/zsh"].find(existsSync);
  if (zsh) shells.push(["zsh", [zsh, "-f", "-i"]]);
  for (const [name, args] of shells) {
    for (const latency of [0, 250]) await scenarios(name, args, proxy, port, latency);
    await inFlightGap(name, args, proxy, port);
    await disconnect(name, args, proxy, port);
  }
  proxy.close();
  if (failures.length) { console.log(`\n${failures.length} failed`); process.exitCode = 1; }
  else console.log("\nPASS: approved execution over real SSH");
} finally {
  host.dispose();
  server.kill();
  rmSync(scratch, { recursive: true, force: true });
}

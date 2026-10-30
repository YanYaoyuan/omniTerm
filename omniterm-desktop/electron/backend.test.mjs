// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import test from "node:test";
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, readdirSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { buildCommandPlan, externalUrl, resolveDesktopAgent, validateProfile } from "./command-plan.mjs";
import { SessionManager, terminatePty } from "./session-manager.mjs";
import { ProfileStore } from "./profile-store.mjs";
import { hostSummary, readSshHosts } from "./ssh-config.mjs";

const options = { platform: "linux", env: { PATH: "/usr/bin", SHELL: "/bin/zsh", CUSTOM: "preserved" }, defaultCwd: "/work", repositoryRoot: "/repo", isDirectory: () => true, isExecutable: (name) => ["/usr/bin/ssh", "/usr/bin/sftp", "/usr/bin/python3", "/bin/zsh", "/repo/dist/omniterm-agent/bin/codex"].includes(name) };

test("shell uses a login shell and preserves the environment", async () => {
  const plan = await buildCommandPlan({ kind: "shell" }, options);
  assert.deepEqual([plan.executable, plan.args, plan.cwd], ["/bin/zsh", ["-l"], "/work"]);
  assert.equal(plan.env.CUSTOM, "preserved");
  assert.equal(plan.env.TERM, "xterm-256color");
});

test("Agent uses built native package and disables daemon and alternate screen", async () => {
  const plan = await buildCommandPlan({ kind: "agent" }, options);
  assert.equal(plan.executable, "/repo/dist/omniterm-agent/bin/codex");
  assert.deepEqual(plan.args, ["--no-daemon", "--no-alt-screen"]);
});

test("packaged Agent is preferred over override and missing resource is explicit", () => {
  const packaged = { ...options, packaged: true, resourcesPath: "/app/resources", env: { ...options.env, OMNITERM_CODEX_BIN: "/vscode/codex" } };
  assert.equal(resolveDesktopAgent({ ...packaged, isExecutable: (name) => name === "/app/resources/omniterm-agent/bin/codex" }).executable, "/app/resources/omniterm-agent/bin/codex");
  assert.throws(() => resolveDesktopAgent(packaged), /bundled Agent is missing/);
});

test("Windows native Agent and PowerShell arguments do not use a command shell", async () => {
  const windows = { ...options, platform: "win32", env: { Path: "C:\\tools" }, defaultCwd: "C:\\work", packaged: true, resourcesPath: "C:\\app\\resources", isExecutable: (name) => ["C:\\tools\\pwsh.exe", "C:\\app\\resources\\omniterm-agent\\bin\\codex.exe"].includes(name) };
  const agent = await buildCommandPlan({ kind: "agent" }, windows);
  const shell = await buildCommandPlan({ kind: "shell" }, windows);
  assert.equal(agent.executable, "C:\\app\\resources\\omniterm-agent\\bin\\codex.exe");
  assert.equal(shell.executable, "C:\\tools\\pwsh.exe");
  assert.deepEqual(shell.args, ["-NoLogo"]);
});

test("SSH and SFTP use their different port flags and literal argv", async () => {
  const profile = { host: "server-alias", username: "alice", port: "2222", keyPath: "/keys/my key;echo bad" };
  const ssh = await buildCommandPlan({ kind: "ssh", profile }, options);
  const sftp = await buildCommandPlan({ kind: "sftp", profile }, options);
  assert.deepEqual(ssh.args, ["-p", "2222", "-i", "/keys/my key;echo bad", "alice@server-alias"]);
  assert.deepEqual(sftp.args, ["-P", "2222", "-i", "/keys/my key;echo bad", "alice@server-alias"]);
  const ipv6 = await buildCommandPlan({ kind: "sftp", profile: { host: "alice@::1" } }, options);
  assert.deepEqual(ipv6.args, ["alice@[::1]"]);
});

test("tunnel L/R/D always defaults to loopback binding", async () => {
  for (const forwardType of ["L", "R", "D"]) {
    const plan = await buildCommandPlan({ kind: "tunnel", profile: { host: "jump", forwardType, bindPort: 8080, targetHost: "::1", targetPort: 80 } }, options);
    assert.deepEqual(plan.args, ["-N", "-o", "ExitOnForwardFailure=yes", `-${forwardType}`, forwardType === "D" ? "127.0.0.1:8080" : "127.0.0.1:8080:[::1]:80", "jump"]);
  }
});

test("serial checks the selected Python environment for pyserial", async () => {
  let probed;
  const request = { kind: "serial", profile: { device: "/dev/ttyUSB0", baud: "9600" } };
  const plan = await buildCommandPlan(request, { ...options, serialProbe: async (python) => { probed = python; return true; } });
  assert.equal(probed.executable, "/usr/bin/python3");
  assert.deepEqual(plan.args, ["-m", "serial.tools.miniterm", "/dev/ttyUSB0", "9600"]);
  await assert.rejects(buildCommandPlan(request, { ...options, serialProbe: async () => false }), /Python 3 with pyserial is required/);
});

test("invalid destinations, secret fields, ports, and cwd are rejected", async () => {
  for (const host of ["-oProxyCommand=bad", "server;touch x", "user@-bad", "server\nname"]) await assert.rejects(buildCommandPlan({ kind: "ssh", profile: { host } }, options));
  assert.throws(() => validateProfile({ kind: "ssh", host: "server", password: "secret" }), /Passwords and secrets/);
  for (const forwardType of ["__proto__", "constructor", "toString"]) assert.throws(() => validateProfile({ forwardType }), /Forward type/);
  assert.throws(() => validateProfile({ host: "user@server", username: "other" }), /not both/);
  await assert.rejects(buildCommandPlan({ kind: "ssh", profile: { host: "server", port: 65536 } }, options), /65535/);
  await assert.rejects(buildCommandPlan({ kind: "shell", cwd: "relative" }, options), /absolute/);
  await assert.rejects(buildCommandPlan({ kind: "shell" }, { ...options, isDirectory: () => false }), /does not exist/);
});

function fixture(planner = async () => ({ executable: "/bin/sh", args: ["-l"], kind: "shell", title: "Terminal", cwd: "/work", env: {} })) {
  const events = { data: [], exit: [], writes: [], sizes: [], kills: 0, spawns: [] };
  let dataCallback, exitCallback;
  const manager = new SessionManager({ planner, terminate: (pty) => pty.kill(), onData: (event) => events.data.push(event), onExit: (event) => events.exit.push(event), spawnPty: (...args) => {
    events.spawns.push(args);
    return { pid: 42, onData: (callback) => { dataCallback = callback; return { dispose() {} }; }, onExit: (callback) => { exitCallback = callback; return { dispose() {} }; }, write: (data) => events.writes.push(data), resize: (...size) => events.sizes.push(size), kill: () => events.kills++ };
  } });
  return { manager, events, emitData: (data) => dataCallback(data), emitExit: (event) => exitCallback(event) };
}

test("create is pending, early resize applies at start, and start is idempotent", async () => {
  const { manager, events, emitData, emitExit } = fixture();
  const session = await manager.create({ kind: "shell" });
  assert.equal(session.status, "pending");
  assert.equal(events.spawns.length, 0);
  manager.resize({ id: session.id, cols: 90, rows: 30 });
  assert.equal(manager.start(session.id).status, "running");
  manager.start(session.id);
  assert.equal(events.spawns.length, 1);
  assert.equal(events.spawns[0][2].cols, 90);
  emitData("hello\r\n");
  manager.write({ id: session.id, data: "ls\r" });
  emitExit({ exitCode: 7, signal: 0 });
  assert.deepEqual(manager.buffer(session.id), { data: "hello\r\n", exited: true, exitCode: 7 });
  assert.deepEqual(events.writes, ["ls\r"]);
  assert.equal(manager.write({ id: session.id, data: "ignored" }), false);
  manager.start(session.id);
  assert.equal(events.spawns.length, 1);
  manager.close(session.id);
  assert.equal(events.kills, 0);
});

test("buffer keeps the latest two MiB without splitting Unicode", async () => {
  const { manager, emitData } = fixture();
  const { id } = await manager.create({});
  manager.start(id);
  emitData("discard this");
  emitData("中文🙂".repeat(300000));
  const data = manager.buffer(id).data;
  assert.ok(Buffer.byteLength(data) <= 2 * 1024 * 1024);
  assert.ok(!data.includes("discard this"));
  assert.ok(!data.includes("�"));
  assert.ok(data.endsWith("中文🙂"));
});

test("output is coalesced per flush and flushed before exit", async () => {
  const events = { data: [], exit: [] };
  const timers = [];
  let dataCallback, exitCallback;
  const manager = new SessionManager({
    planner: async () => ({ executable: "/bin/sh", args: [], kind: "shell", title: "Terminal", cwd: "/work", env: {} }),
    onData: (event) => events.data.push(event), onExit: (event) => events.exit.push({ ...event, dataBefore: events.data.length }),
    schedule: (callback) => { timers.push(callback); return timers.length; }, cancel: () => {},
    spawnPty: () => ({ pid: 1, onData: (callback) => { dataCallback = callback; return { dispose() {} }; }, onExit: (callback) => { exitCallback = callback; return { dispose() {} }; }, write() {}, resize() {}, kill() {} }),
  });
  const { id } = await manager.create({});
  manager.start(id);
  dataCallback("a"); dataCallback("b"); dataCallback("c");
  assert.equal(events.data.length, 0);
  assert.equal(timers.length, 1);
  timers.shift()();
  assert.deepEqual(events.data, [{ id, data: "abc" }]);
  dataCallback("tail");
  exitCallback({ exitCode: 0 });
  assert.deepEqual(events.data.at(-1), { id, data: "tail" });
  assert.equal(events.exit[0].dataBefore, 2);
});

test("session limit cannot be bypassed by concurrent create and disposal kills PTYs", async () => {
  const { manager, events } = fixture();
  const results = await Promise.allSettled(Array.from({ length: 24 }, () => manager.create({})));
  assert.equal(results.filter((result) => result.status === "fulfilled").length, 20);
  for (const result of results) if (result.status === "fulfilled") manager.start(result.value.id);
  manager.dispose();
  assert.equal(events.kills, 20);
  assert.equal(manager.sessions.size, 0);
  await assert.rejects(manager.create({}), /closing/);
});

test("pending async planning cannot add a session after shutdown", async () => {
  let finish;
  const { manager } = fixture(() => new Promise((resolve) => { finish = resolve; }));
  const pending = manager.create({});
  manager.dispose();
  finish({});
  await assert.rejects(pending, /closing/);
  assert.equal(manager.sessions.size, 0);
});

test("invalid input and resize are rejected, closing an unknown session is harmless", async () => {
  const { manager } = fixture();
  const { id } = await manager.create({});
  assert.throws(() => manager.resize({ id, cols: Infinity, rows: 1 }), /Terminal size/);
  assert.throws(() => manager.write({ id, data: Buffer.alloc(10) }), /Terminal input/);
  assert.throws(() => manager.write({ id, data: "a".repeat(1024 * 1024 + 1) }), /Terminal input/);
  assert.equal(manager.close(id), true);
  assert.equal(manager.close(id), false);
});

test("Unix termination signals the PTY process group; Windows uses ConPTY kill", () => {
  const signals = [];
  let killed = 0;
  const pty = { pid: 42, kill: () => killed++ };
  terminatePty(pty, "darwin", (...args) => signals.push(args));
  terminatePty(pty, "win32", (...args) => signals.push(args));
  assert.deepEqual(signals, [[-42, "SIGHUP"]]);
  assert.equal(killed, 2);
});

test("profiles round-trip atomically with generated ids and no passwords", () => {
  const directory = mkdtempSync(path.join(os.tmpdir(), "omniterm-profiles-"));
  try {
    const store = new ProfileStore(path.join(directory, "profiles.json"));
    assert.deepEqual(store.list(), []);
    const [created] = store.save({ kind: "ssh", name: "Dev", host: "dev", port: "2222" });
    assert.ok(created.id);
    assert.equal(store.list()[0].port, 2222);
    store.save({ ...created, name: "Updated" });
    assert.equal(store.list().length, 1);
    assert.equal(store.list()[0].name, "Updated");
    assert.throws(() => store.save({ kind: "ssh", host: "dev", password: "secret" }), /Passwords and secrets/);
    assert.ok(!readFileSync(store.filename, "utf8").includes("secret"));
    assert.deepEqual(readdirSync(directory), ["profiles.json"]);
    assert.deepEqual(store.delete(created.id), []);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});

test("only http and https links can be opened externally", () => {
  assert.equal(externalUrl("https://example.com/a b"), "https://example.com/a%20b");
  assert.equal(externalUrl("http://localhost:3000/"), "http://localhost:3000/");
  for (const value of ["file:///etc/passwd", "javascript:alert(1)", "smb://host/share", "not a url", 42, "https://x/" + "a".repeat(9000)]) assert.throws(() => externalUrl(value));
});

const sshConfig = `# Example from a real board setup
Host dog3
    HostName 192.168.127.2
    User root
    ProxyJump dog3_x5
    IdentityFile ~/.ssh/id_ed25519_evt_board
    IdentitiesOnly yes
Host dog3_x5
    HostName=192.168.10.222
    User "root"
    Port 2222 # trailing comment
Host one two
    HostName %h.example.com
Host web-* !web-admin ?ingle
    User deploy
Match host other
    User nobody
Host after-match
    ProxyJump none
Host *
    ServerAliveInterval 30
    User fallback
`;

test("SSH config lists concrete hosts, applies first-match-wins, and skips patterns", () => {
  const home = path.resolve("ssh-config-home");
  const filename = path.join(home, ".ssh", "config");
  const { hosts, error } = readSshHosts({ home, read: (name) => name === filename ? sshConfig : null });
  assert.equal(error, "");
  assert.deepEqual(hosts, [
    { alias: "dog3", hostname: "192.168.127.2", user: "root", proxyJump: "dog3_x5" },
    { alias: "dog3_x5", hostname: "192.168.10.222", user: "root", port: 2222 },
    { alias: "one", hostname: "one.example.com", user: "fallback" },
    { alias: "two", hostname: "two.example.com", user: "fallback" },
    { alias: "after-match", hostname: "after-match", user: "fallback" },
  ]);
  assert.equal(hostSummary(hosts[0]), "root@192.168.127.2 · 经 dog3_x5");
  assert.equal(hostSummary(hosts[1]), "root@192.168.10.222:2222");
});

test("SSH config follows Include globs relative to ~/.ssh and survives cycles", () => {
  const home = mkdtempSync(path.join(os.tmpdir(), "omniterm-ssh-config-"));
  try {
    mkdirSync(path.join(home, ".ssh", "config.d"), { recursive: true });
    writeFileSync(path.join(home, ".ssh", "config"), "Include config.d/*\nUser outer\nHost main\n  HostName main.example\n");
    writeFileSync(path.join(home, ".ssh", "config.d", "b.conf"), "Host included-b\n  Include ~/.ssh/config\n");
    writeFileSync(path.join(home, ".ssh", "config.d", "a.conf"), "Host included-a\n  User a\n");
    writeFileSync(path.join(home, ".ssh", "config.d", ".hidden"), "Host hidden\n");
    assert.deepEqual(readSshHosts({ home }).hosts.map((host) => [host.alias, host.user]), [["included-a", "a"], ["included-b", "outer"], ["main", "outer"]]);
    assert.deepEqual(readSshHosts({ home: path.join(home, "missing") }), { filename: path.join(home, "missing", ".ssh", "config"), error: "", hosts: [] });
  } finally { rmSync(home, { recursive: true, force: true }); }
});

test("ProxyJump reaches ssh, sftp, tunnels, and the file panel as one literal option", async () => {
  const profile = { host: "board", username: "root", proxyJump: " jump@10.0.0.1:2222 , ssh://second , dog3_x5 " };
  const expected = ["-o", "ProxyJump=jump@10.0.0.1:2222,ssh://second,dog3_x5"];
  const ssh = await buildCommandPlan({ kind: "ssh", profile, companion: true }, { ...options, controlPath: () => "/tmp/c/1" });
  assert.deepEqual(ssh.args.slice(-3), [...expected, "root@board"]);
  assert.deepEqual(ssh.remote.args, expected);
  const sftp = await buildCommandPlan({ kind: "sftp", profile }, options);
  assert.deepEqual(sftp.args.slice(-3), [...expected, "root@board"]);
  const tunnel = await buildCommandPlan({ kind: "tunnel", profile: { ...profile, forwardType: "D", bindPort: 1080 } }, options);
  assert.ok(tunnel.args.join(" ").includes(expected.join(" ")));
});

test("ProxyJump refuses anything ssh could read as an option or a second argument", () => {
  for (const proxyJump of ["-oProxyCommand=x", "host,-bad", "a b", "user@-x", "a,,b", "a;b", "ssh://-x"]) {
    assert.throws(() => validateProfile({ kind: "ssh", host: "h", proxyJump }), /ProxyJump|control/, proxyJump);
  }
});

test("importing SSH hosts skips saved aliases and refuses batches that do not fit", () => {
  const directory = mkdtempSync(path.join(os.tmpdir(), "omniterm-import-"));
  try {
    const store = new ProfileStore(path.join(directory, "profiles.json"));
    store.save({ kind: "ssh", name: "mine", host: "dog3", username: "me" });
    const profiles = store.import([{ kind: "ssh", name: "dog3", host: "dog3" }, { kind: "ssh", name: "dog3_x5", host: "dog3_x5", summary: "root@192.168.10.222" }]);
    assert.deepEqual(profiles.map((profile) => [profile.name, profile.summary]), [["mine", undefined], ["dog3_x5", "root@192.168.10.222"]]);
    assert.equal(new Set(profiles.map((profile) => profile.id)).size, 2);
    const many = Array.from({ length: 199 }, (_, index) => ({ kind: "ssh", name: `h${index}`, host: `h${index}` }));
    assert.throws(() => store.import(many), /maximum of 200/);
    assert.equal(store.list().length, 2);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});

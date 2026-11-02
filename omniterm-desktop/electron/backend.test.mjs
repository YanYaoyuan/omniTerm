// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import test from "node:test";
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, readdirSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { buildCommandPlan, externalUrl, resolveDesktopAgent, validateProfile } from "./command-plan.mjs";
import { SessionManager } from "./session-manager.mjs";
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
  assert.deepEqual(ssh.connection, { host: "server-alias", user: "alice", port: 2222 });
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

// Stands in for the Rust host, whose PTY behavior has its own tests.
function fixture(planner = async () => ({ executable: "/bin/sh", args: ["-l"], kind: "shell", title: "Terminal", cwd: "/work", env: {}, connection: {}, remote: { destination: "x" } })) {
  const requests = [];
  let created = 0;
  const host = {
    request: async (method, params) => {
      requests.push([method, params]);
      if (method === "session.create") return { id: `s${++created}`, title: params.title, kind: params.kind, cwd: params.cwd, status: "pending", epoch: 0, shellNonce: "n" };
      if (method === "session.start") return { id: params.id, title: "Terminal", kind: "shell", cwd: "/work", status: "running", epoch: 1, pid: 42 };
      return true;
    },
    dispose() { this.disposed = true; },
  };
  const started = [];
  const closed = [];
  const manager = new SessionManager({ planner, host, onStart: (id, plan) => started.push([id, plan.remote]), onClose: (id) => closed.push(id) });
  return { manager, host, requests, started, closed };
}

test("sessions are created in the host without leaking main-process plan details", async () => {
  const { manager, requests, started } = fixture();
  const session = await manager.create({ kind: "shell" });
  assert.deepEqual([session.status, session.remote, "shellNonce" in session], ["pending", true, false]);
  assert.deepEqual(Object.keys(requests[0][1]).sort(), ["args", "connection", "cwd", "env", "executable", "kind", "title"]);
  const running = await manager.start(session.id);
  assert.deepEqual([running.status, running.pid, running.epoch], ["running", 42, 1]);
  assert.deepEqual(started, [[session.id, { destination: "x" }]]);
});

test("terminal replies are marked so they do not count as user input", async () => {
  const { manager, requests } = fixture();
  const { id } = await manager.create({});
  await manager.write({ id, data: "ls\r" });
  await manager.write({ id, data: "\x1b[1;1R", reply: true });
  assert.deepEqual(requests.slice(-2).map(([, params]) => params.reply), [false, true]);
});

test("unknown sessions are refused before reaching the host", async () => {
  const { manager, requests } = fixture();
  assert.throws(() => manager.write({ id: "missing", data: "x" }), /not found/);
  assert.throws(() => manager.resize({ id: "missing", cols: 80, rows: 24 }), /not found/);
  assert.equal(await manager.close("missing"), false);
  assert.equal(requests.length, 0);
});

test("closing and disposal release remote state and the host", async () => {
  const { manager, host, closed } = fixture();
  const first = await manager.create({});
  const second = await manager.create({});
  assert.equal(await manager.close(first.id), true);
  assert.equal(await manager.close(first.id), false);
  manager.dispose();
  assert.deepEqual(closed, [first.id, second.id]);
  assert.equal(host.disposed, true);
  await assert.rejects(manager.create({}), /closing/);
});

test("pending async planning cannot add a session after shutdown", async () => {
  let finish;
  const { manager, requests } = fixture(() => new Promise((resolve) => { finish = resolve; }));
  const pending = manager.create({});
  manager.dispose();
  finish({});
  await assert.rejects(pending, /closing/);
  assert.equal(requests.length, 0);
});

test("every tab learns its session ended when the host stops", async () => {
  const { manager } = fixture();
  const { id } = await manager.create({});
  const exits = [];
  manager.hostStopped((event) => exits.push(event));
  assert.deepEqual(exits, [{ id, exitCode: 1, signal: null }]);
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

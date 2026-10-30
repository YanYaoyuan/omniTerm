// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import { EventEmitter } from "node:events";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import os from "node:os";
import path from "node:path";
import { PassThrough } from "node:stream";
import { buildCommandPlan } from "./command-plan.mjs";
import { METRICS_SCRIPT, MetricsTracker, parseSample, takeSamples } from "./remote-metrics.mjs";
import { RemoteManager, safeLocalName } from "./remote-manager.mjs";
import { SessionManager } from "./session-manager.mjs";
import { SftpClient, SftpError } from "./sftp-client.mjs";

const SFTP_SERVER = ["/usr/lib/openssh/sftp-server", "/usr/libexec/sftp-server", "/usr/libexec/openssh/sftp-server", "/usr/lib/ssh/sftp-server"].find(existsSync);
const needsServer = { skip: SFTP_SERVER ? false : "OpenSSH sftp-server is not installed" };
const options = { platform: "linux", env: { PATH: "/usr/bin" }, defaultCwd: "/work", repositoryRoot: "/repo", isDirectory: () => true, isExecutable: (name) => ["/usr/bin/ssh", "/usr/bin/sftp"].includes(name) };
const target = { executable: "/usr/bin/ssh", args: [], destination: "host", controlPath: null, env: {} };

function scratch(t) {
  const directory = mkdtempSync(path.join(os.tmpdir(), "omniterm-remote-test-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  return directory;
}

// Runs the manager against a real sftp-server in place of `ssh -s host sftp`.
async function liveRemote(t) {
  const root = scratch(t);
  const states = [];
  const transfers = [];
  const manager = new RemoteManager({
    spawnProcess: (_executable, args, spawnOptions) => spawn(SFTP_SERVER, [], { ...spawnOptions, cwd: root }),
    onState: (state) => states.push(state),
    onTransfer: (transfer) => transfers.push(transfer),
  });
  t.after(() => manager.dispose());
  manager.attach("s1", target);
  await waitFor(() => states.at(-1)?.state === "ready");
  const finished = async (id) => { await waitFor(() => transfers.some((item) => item.transferId === id && ["done", "failed", "canceled"].includes(item.status))); return transfers.findLast((item) => item.transferId === id); };
  return { manager, root, states, transfers, finished };
}

async function waitFor(condition, timeout = 5000) {
  const deadline = Date.now() + timeout;
  while (!condition()) {
    if (Date.now() > deadline) throw new Error("Timed out waiting for condition.");
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}

test("SSH sessions share their login through a control socket", async () => {
  const plan = await buildCommandPlan({ kind: "ssh", profile: { kind: "ssh", host: "example.com", username: "dev", port: 2222, keyPath: "/k" } }, { ...options, controlPath: () => "/tmp/x/1" });
  assert.deepEqual(plan.args, ["-o", "ControlMaster=auto", "-o", "ControlPath=/tmp/x/1", "-o", "ControlPersist=30", "-p", "2222", "-i", "/k", "dev@example.com"]);
  assert.deepEqual([plan.remote.destination, plan.remote.controlPath, plan.remote.args], ["dev@example.com", "/tmp/x/1", ["-p", "2222", "-i", "/k"]]);
});

test("interactive SFTP tabs keep -P while their companion uses ssh -p", async () => {
  const plan = await buildCommandPlan({ kind: "sftp", profile: { kind: "sftp", host: "::1", port: 2222 } }, { ...options, controlPath: () => "/tmp/x/2" });
  assert.deepEqual(plan.args.slice(-3), ["-P", "2222", "[::1]"]);
  assert.deepEqual([plan.remote.executable, plan.remote.destination, plan.remote.args], ["/usr/bin/ssh", "::1", ["-p", "2222"]]);
});

test("opting out or lacking a control path leaves the ssh command unchanged", async () => {
  const profile = { kind: "ssh", host: "example.com" };
  const off = await buildCommandPlan({ kind: "ssh", companion: false, profile }, { ...options, controlPath: () => assert.fail("must not allocate") });
  assert.deepEqual([off.args, off.remote], [["example.com"], undefined]);
  const fallback = await buildCommandPlan({ kind: "ssh", profile }, { ...options, controlPath: () => null });
  assert.deepEqual([fallback.args, fallback.remote.controlPath], [["example.com"], null]);
  await assert.rejects(buildCommandPlan({ kind: "ssh", companion: "yes", profile }, options), /boolean/);
});

test("tunnels never get a companion connection", async () => {
  const plan = await buildCommandPlan({ kind: "tunnel", profile: { kind: "tunnel", host: "example.com", forwardType: "D", bindPort: 1080 } }, { ...options, controlPath: () => assert.fail("must not allocate") });
  assert.equal(plan.remote, undefined);
});

test("control paths are rejected rather than risking a fatal ssh error", (t) => {
  const manager = new RemoteManager();
  t.after(() => manager.dispose());
  if (process.platform === "win32") {
    assert.equal(manager.controlPath(), null);
    assert.equal(manager.controlDirectory, null);
  } else assert.match(manager.controlPath(), /omniterm-ssh-\w+\/1$/);
  assert.equal(new RemoteManager({ platform: "win32" }).controlPath(), null);
  const long = path.join(scratch(t), "x".repeat(70));
  mkdirSync(long);
  const tooLong = new RemoteManager({ tempRoot: long });
  t.after(() => tooLong.dispose());
  assert.equal(tooLong.controlPath(), null);
  const spaced = path.join(scratch(t), "a b");
  mkdirSync(spaced);
  const unsafe = new RemoteManager({ tempRoot: spaced });
  t.after(() => unsafe.dispose());
  assert.equal(unsafe.controlPath(), null);
});

test("control sockets use private directories and leave room for OpenSSH temporary names", async (t) => {
  assert.equal(new RemoteManager({ platform: "darwin" }).tempRoot, "/tmp");
  const manager = new RemoteManager();
  t.after(() => manager.dispose());
  if (process.platform === "win32") {
    // Windows OpenSSH has no Unix control sockets. Verify the supported fallback
    // rather than expecting a Unix filename or skipping this platform's test.
    assert.equal(manager.controlPath(), null);
    assert.equal(manager.controlDirectory, null);
    assert.ok(manager.channelArgs(target).includes("ControlPath=none"));
    return;
  }
  const sockets = Array.from({ length: 30 }, () => manager.controlPath());
  assert.equal(new Set(sockets).size, 30);
  for (const socket of sockets) {
    assert.equal(typeof socket, "string");
    assert.ok(Buffer.byteLength(socket) <= 86);
  }
  const directory = manager.controlDirectory;
  assert.equal(statSync(directory).mode & 0o777, 0o700);
  assert.equal(statSync(directory).uid, process.getuid());
  const server = createServer();
  try {
    // OpenSSH appends this many bytes while creating its listener. Bind the
    // complete path to check the kernel's limit as well as our own byte limit.
    await new Promise((resolve, reject) => {
      server.once("error", reject);
      server.listen(`${sockets.at(-1)}.abcdefghijklmnop`, resolve);
    });
  } finally {
    if (server.listening) await new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve()));
    manager.dispose();
  }
  assert.equal(existsSync(directory), false);
});

test("the session manager reports start and close to the remote layer", async () => {
  const events = [];
  const host = { request: async (method) => method === "session.close" ? true : { id: "s1", status: "running" }, dispose() {} };
  const plan = { kind: "ssh", title: "x", cwd: "/", args: [], remote: { destination: "h" } };
  const manager = new SessionManager({ planner: async () => plan, host, onStart: (id, started) => events.push(["start", id, started.remote.destination]), onClose: (id) => events.push(["close", id]) });
  const session = await manager.create({});
  assert.equal(session.remote, true);
  await manager.start(session.id);
  await manager.close(session.id);
  assert.deepEqual(events, [["start", session.id, "h"], ["close", session.id]]);
});

test("file names that escape the destination are refused", () => {
  for (const name of ["..", ".", "", "../evil", "a/b", "/etc/passwd", "a\0b", "x".repeat(256)]) assert.equal(safeLocalName(name, "linux"), false, name);
  for (const name of ["..\\evil", "C:evil", "CON", "nul.txt", "trail.", "trail ", "a|b"]) assert.equal(safeLocalName(name, "win32"), false, name);
  assert.equal(safeLocalName("back\\slash", "linux"), true);
  assert.equal(safeLocalName("报告 final.txt", "win32"), true);
});

test("a malicious listing cannot write outside the download folder", async (t) => {
  const base = scratch(t);
  const destination = path.join(base, "downloads");
  mkdirSync(destination);
  const manager = new RemoteManager({ onTransfer: () => {} });
  const directory = { mode: 0o040755 };
  const file = { mode: 0o100644, size: 1 };
  const written = [];
  const sftp = {
    stat: async () => directory,
    readdir: async () => [{ name: "../evil", attributes: file }, { name: "..", attributes: directory }, { name: "good.txt", attributes: file }, { name: "link", attributes: { mode: 0o120777 } }],
    download: async (_remote, local) => { written.push(local); writeFileSync(local, "x"); },
  };
  manager.remotes.set("s1", { id: "s1", state: "ready", sftp, transfers: new Map(), queue: [], running: 0 });
  const result = await manager.download({ id: "s1", paths: ["/data/folder"], destination });
  const events = [];
  manager.onTransfer = (event) => events.push(event);
  await waitFor(() => events.some((event) => event.status === "done"));
  assert.equal(result.transfers.length, 1);
  assert.equal(events.at(-1).skipped, 3);
  assert.equal(written.length, 1);
  assert.ok(written[0].startsWith(path.join(destination, "folder")));
  assert.deepEqual(readdirSync(path.join(destination, "folder")), ["good.txt"]);
  assert.deepEqual(readdirSync(base), ["downloads"]);
  await assert.rejects(manager.download({ id: "s1", paths: ["/data/.."], destination }), /unsafe file name/);
});

test("the client rejects oversized packets instead of buffering them", async () => {
  const input = new PassThrough();
  const output = new PassThrough();
  const client = new SftpClient(input, output);
  const handshake = client.init();
  const packet = Buffer.alloc(4);
  packet.writeUInt32BE(0x7fffffff);
  output.write(packet);
  await assert.rejects(handshake, /invalid packet size/);
});

test("a failed handshake explains when the host has no SFTP subsystem", async () => {
  const states = [];
  const child = Object.assign(new EventEmitter(), { stdin: new PassThrough(), stdout: new PassThrough(), stderr: new PassThrough(), kill() {} });
  const manager = new RemoteManager({ spawnProcess: () => child, onState: (state) => states.push(state) });
  manager.attach("s1", target);
  child.stderr.write("subsystem request failed on channel 0\n");
  await new Promise((resolve) => setImmediate(resolve));
  child.emit("close", 255);
  assert.deepEqual([states.at(-1).state, states.at(-1).reason], ["unavailable", "no-sftp"]);
  manager.detach("s1");
});

test("a silent refusal through a shared connection also means no SFTP", async () => {
  const states = [];
  const child = Object.assign(new EventEmitter(), { stdin: new PassThrough(), stdout: new PassThrough(), stderr: new PassThrough(), kill() {} });
  const manager = new RemoteManager({ exists: () => true, runSync: () => {}, spawnProcess: () => child, onState: (state) => states.push(state) });
  manager.attach("s1", { ...target, controlPath: "/tmp/x/1" });
  child.emit("close", 255);
  assert.deepEqual([states.at(-1).state, states.at(-1).reason], ["unavailable", "no-sftp"]);
  manager.detach("s1");
});

test("the panel waits for the login before opening any channel", async () => {
  let authenticated = false;
  const spawned = [];
  const child = Object.assign(new EventEmitter(), { stdin: new PassThrough(), stdout: new PassThrough(), stderr: new PassThrough(), kill() {} });
  const manager = new RemoteManager({ exists: () => authenticated, pollMs: 2, runSync: () => {}, spawnProcess: (_file, args) => { spawned.push(args); return child; } });
  manager.attach("s1", { ...target, controlPath: "/tmp/x/1" });
  await new Promise((resolve) => setTimeout(resolve, 15));
  assert.equal(manager.state("s1").state, "waiting");
  assert.equal(spawned.length, 0);
  authenticated = true;
  await waitFor(() => spawned.length === 1);
  assert.ok(spawned[0].includes("ControlPath=/tmp/x/1"));
  assert.ok(spawned[0].includes("BatchMode=yes"));
  assert.deepEqual(spawned[0].slice(-3), ["-s", "host", "sftp"]);
  manager.detach("s1");
  assert.deepEqual(spawned.at(-1).slice(-3), ["-O", "exit", "host"]);
});

test("real sftp-server: listing, folders, rename and links", needsServer, async (t) => {
  const { manager, root } = await liveRemote(t);
  writeFileSync(path.join(root, "a file 中文.txt"), "hello");
  mkdirSync(path.join(root, "folder"));
  symlinkSync("folder", path.join(root, "link"));
  symlinkSync("missing", path.join(root, "broken"));
  const listing = await manager.list({ id: "s1", path: root });
  const byName = Object.fromEntries(listing.entries.map((entry) => [entry.name, entry]));
  assert.deepEqual(Object.keys(byName).sort(), ["a file 中文.txt", "broken", "folder", "link"]);
  assert.equal(byName["a file 中文.txt"].size, 5);
  assert.equal(byName.folder.directory, true);
  assert.deepEqual([byName.link.directory, byName.link.symlink], [true, true]);
  assert.deepEqual([byName.broken.directory, byName.broken.symlink], [false, true]);
  await manager.mkdir({ id: "s1", path: path.join(root, "new dir") });
  await manager.rename({ id: "s1", from: path.join(root, "new dir"), to: path.join(root, "renamed") });
  assert.ok(statSync(path.join(root, "renamed")).isDirectory());
  await assert.rejects(manager.list({ id: "s1", path: path.join(root, "absent") }), SftpError);
});

test("real sftp-server: large transfers round-trip byte for byte", needsServer, async (t) => {
  const { manager, root, finished } = await liveRemote(t);
  const local = scratch(t);
  const content = randomBytes(5 * 1024 * 1024 + 123);
  writeFileSync(path.join(local, "big.bin"), content, { mode: 0o750 });
  const up = await manager.upload({ id: "s1", paths: [path.join(local, "big.bin")], directory: root });
  assert.equal((await finished(up.transfers[0])).status, "done");
  assert.ok(readFileSync(path.join(root, "big.bin")).equals(content));
  assert.equal(statSync(path.join(root, "big.bin")).mode & 0o777, 0o750);
  const destination = path.join(local, "out");
  mkdirSync(destination);
  const down = await manager.download({ id: "s1", paths: [path.join(root, "big.bin")], destination });
  const result = await finished(down.transfers[0]);
  assert.deepEqual([result.status, result.bytes], ["done", content.length]);
  assert.ok(readFileSync(path.join(destination, "big.bin")).equals(content));
  assert.deepEqual(readdirSync(destination), ["big.bin"]);
  assert.deepEqual(readdirSync(root), ["big.bin"]);
});

test("real sftp-server: folders transfer recursively and links are skipped", needsServer, async (t) => {
  const { manager, root, finished } = await liveRemote(t);
  const local = scratch(t);
  mkdirSync(path.join(local, "tree/sub"), { recursive: true });
  writeFileSync(path.join(local, "tree/a.txt"), "a");
  writeFileSync(path.join(local, "tree/sub/b.txt"), "b");
  symlinkSync("/etc/passwd", path.join(local, "tree/secret"));
  const up = await manager.upload({ id: "s1", paths: [path.join(local, "tree")], directory: root });
  const uploaded = await finished(up.transfers[0]);
  assert.deepEqual([uploaded.status, uploaded.skipped, uploaded.total], ["done", 1, 2]);
  assert.equal(readFileSync(path.join(root, "tree/sub/b.txt"), "utf8"), "b");
  assert.equal(existsSync(path.join(root, "tree/secret")), false);
  symlinkSync("/etc/passwd", path.join(root, "tree/remote-secret"));
  const destination = path.join(local, "out");
  mkdirSync(destination);
  const down = await manager.download({ id: "s1", paths: [path.join(root, "tree")], destination });
  const downloaded = await finished(down.transfers[0]);
  assert.deepEqual([downloaded.status, downloaded.skipped], ["done", 1]);
  assert.equal(readFileSync(path.join(destination, "tree/sub/b.txt"), "utf8"), "b");
  assert.equal(existsSync(path.join(destination, "tree/remote-secret")), false);
});

test("real sftp-server: existing files are only replaced after confirmation", needsServer, async (t) => {
  const { manager, root, finished } = await liveRemote(t);
  const local = scratch(t);
  writeFileSync(path.join(local, "same.txt"), "new");
  writeFileSync(path.join(root, "same.txt"), "old");
  assert.deepEqual(await manager.upload({ id: "s1", paths: [path.join(local, "same.txt")], directory: root }), { conflicts: ["same.txt"] });
  assert.equal(readFileSync(path.join(root, "same.txt"), "utf8"), "old");
  const replaced = await manager.upload({ id: "s1", paths: [path.join(local, "same.txt")], directory: root, overwrite: true });
  assert.equal((await finished(replaced.transfers[0])).status, "done");
  assert.equal(readFileSync(path.join(root, "same.txt"), "utf8"), "new");
  writeFileSync(path.join(root, "same.txt"), "remote");
  const result = await manager.download({ id: "s1", paths: [path.join(root, "same.txt")], destination: local });
  assert.deepEqual(result, { conflicts: ["same.txt"], destination: local });
  assert.equal(readFileSync(path.join(local, "same.txt"), "utf8"), "new");
});

test("real sftp-server: canceled transfers leave no partial files", needsServer, async (t) => {
  const { manager, root, finished } = await liveRemote(t);
  const local = scratch(t);
  writeFileSync(path.join(local, "big.bin"), randomBytes(64 * 1024 * 1024));
  const up = await manager.upload({ id: "s1", paths: [path.join(local, "big.bin")], directory: root });
  manager.cancel({ id: "s1", transferId: up.transfers[0] });
  assert.equal((await finished(up.transfers[0])).status, "canceled");
  await waitFor(() => readdirSync(root).length === 0);
});

test("real sftp-server: delete removes trees but never follows links", needsServer, async (t) => {
  const { manager, root } = await liveRemote(t);
  const outside = scratch(t);
  writeFileSync(path.join(outside, "keep.txt"), "keep");
  mkdirSync(path.join(root, "tree/sub"), { recursive: true });
  writeFileSync(path.join(root, "tree/sub/a.txt"), "a");
  symlinkSync(outside, path.join(root, "tree/link"));
  await manager.remove({ id: "s1", paths: [path.join(root, "tree")] });
  assert.deepEqual(readdirSync(root), []);
  assert.equal(readFileSync(path.join(outside, "keep.txt"), "utf8"), "keep");
  await assert.rejects(manager.remove({ id: "s1", paths: ["/"] }), /root directory/);
});

test("real sftp-server: dragged items are prepared in a private folder", needsServer, async (t) => {
  const { manager, root } = await liveRemote(t);
  writeFileSync(path.join(root, "doc.txt"), "drag me");
  const { token } = await manager.prepareDrag({ id: "s1", path: path.join(root, "doc.txt") });
  const file = manager.dragFile({ id: "s1", token });
  assert.equal(readFileSync(file, "utf8"), "drag me");
  assert.throws(() => manager.dragFile({ id: "s1", token: "forged" }), /no longer available/);
  manager.detach("s1");
  assert.equal(existsSync(file), false);
});

test("the metrics script is safe to wrap in single quotes", () => {
  assert.equal(METRICS_SCRIPT.includes("'"), false);
});

test("the metrics script produces a parsable sample from a real shell", { skip: process.platform === "linux" ? false : "reads /proc" }, () => {
  const script = METRICS_SCRIPT.replace(/\nsleep \d+\ndone/, "\nbreak\ndone");
  const sample = parseSample(takeSamples(execFileSync("sh", ["-c", script], { encoding: "utf8" })).samples[0]);
  assert.equal(sample.os, "Linux");
  assert.ok(sample.cpuTicks.total > 0 && sample.cores >= 1 && sample.mem.total > 0 && sample.disk.total > 0 && sample.load.length === 3 && sample.uptime > 0);
});

const LINUX = `@@begin Linux
@@cpu
cpu  100 0 100 800 0 0 0 0 0 0
@@cores
8
@@mem
MemTotal:       16384 kB
MemAvailable:    4096 kB
SwapTotal:       2048 kB
SwapFree:        1024 kB
@@load
0.50 0.25 0.10 1/100 1234
@@uptime
3600.5 100.0
@@net
    lo: 999 1 0 0 0 0 0 0 999 1 0 0 0 0 0 0
  eth0: 1000 1 0 0 0 0 0 0 2000 1 0 0 0 0 0 0
@@disk
/dev/sda1 1000 250 750 25% /
@@nvidia
NVIDIA GeForce RTX 4090, 37, 2048, 24564, 61
Bad Line, [N/A], [N/A], [N/A], [N/A]
@@amdgpu
12 1048576 8589934592
`;

test("metrics parse every section and compute rates from counters", () => {
  let time = 0;
  const tracker = new MetricsTracker(() => time);
  const first = tracker.next(parseSample(LINUX));
  assert.equal(first.cpu, undefined);
  assert.equal(first.net, undefined);
  assert.deepEqual(first.mem, { total: 16777216, used: 12582912 });
  assert.deepEqual(first.swap, { total: 2097152, used: 1048576 });
  assert.deepEqual(first.disk, { total: 1024000, used: 256000 });
  assert.deepEqual(first.load, [0.5, 0.25, 0.1]);
  assert.equal(first.gpus.length, 2);
  assert.deepEqual(first.gpus[0], { name: "NVIDIA GeForce RTX 4090", util: 37, memUsed: 2048 * 1048576, memTotal: 24564 * 1048576, temp: 61 });
  assert.deepEqual(first.gpus[1], { name: "AMD GPU 0", util: 12, memUsed: 1048576, memTotal: 8589934592, temp: null });
  time = 2000;
  const second = tracker.next(parseSample(LINUX.replace("cpu  100 0 100 800", "cpu  180 0 120 900").replace("eth0: 1000", "eth0: 3000").replace("0 2000 1", "0 2500 1")));
  assert.equal(second.cpu, 50);
  assert.deepEqual(second.net, { rx: 1000, tx: 250 });
});

test("metrics parsing tolerates hostile or incomplete output", () => {
  const sample = parseSample(`@@begin ${"x".repeat(100)}\n@@cpu\ncpu NaN -1\n@@mem\nMemTotal: nope\n@@disk\nshort\n@@nvidia\n${"n".repeat(500)}, 1, 2, 3, 4\n@@load\na b c\n`);
  assert.equal(sample.os.length, 40);
  assert.equal(sample.cpuTicks, undefined);
  assert.equal(sample.mem, undefined);
  assert.equal(sample.disk, undefined);
  assert.equal(sample.load, undefined);
  assert.equal(sample.gpus[0].name.length, 80);
  assert.deepEqual(parseSample("").gpus, []);
});

test("macOS samples report memory and CPU", () => {
  const sample = parseSample("@@begin Darwin\n@@cpupercent\nCPU usage: 3.0% user, 7.0% sys, 90.0% idle\n@@cores\n10\n@@memdarwin\n17179869184\n16384\nPages active: 1000.\nPages wired down: 500.\nPages occupied by compressor: 500.\n@@load\n 1.5 1.0 0.5 \n");
  assert.equal(new MetricsTracker().next(sample).cpu, 10);
  assert.deepEqual(sample.mem, { total: 17179869184, used: 2000 * 16384 });
  assert.deepEqual(sample.load, [1.5, 1, 0.5]);
});

test("streamed metrics are split on complete samples", () => {
  const { samples, rest } = takeSamples("@@begin A\n@@end\n@@begin B\n@@end\n@@begin C\n");
  assert.deepEqual(samples, ["@@begin A\n", "@@begin B\n"]);
  assert.equal(rest, "@@begin C\n");
});

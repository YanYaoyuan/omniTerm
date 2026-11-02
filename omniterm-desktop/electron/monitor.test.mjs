import test from "node:test";
import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import { appendFileSync, existsSync, mkdtempSync, renameSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { conditionHolds, parseProbe, probeScript, validateCondition, validateTarget } from "./monitor-probe.mjs";
import { MonitorService } from "./monitor-service.mjs";

const unix = process.platform !== "win32";
const sh = (script) => execFileSync("sh", ["-c", script], { encoding: "utf8" });

test("targets are validated before they can reach a script", () => {
  for (const target of [{ kind: "process", name: "a;rm -rf /" }, { kind: "process", name: "$(id)" }, { kind: "process", pid: "1 2" }, { kind: "log", path: "relative" }, { kind: "log", path: "/a b" }, { kind: "log", path: "/etc/../x" }, { kind: "log", path: "/a'b" }, { kind: "thread", pid: 1 }, { kind: "other" }, { kind: "sharedMemory", name: "x", access: "sysv", layout: [{ field: "a", offset: 0, type: "u8" }] }, { kind: "sharedMemory", name: "x/../y", access: "posix-shm", layout: [{ field: "a", offset: 0, type: "u8" }] }, { kind: "sharedMemory", name: "x", access: "posix-shm", layout: [] }, { kind: "sharedMemory", name: "x", access: "posix-shm", layout: [{ field: "a", offset: 9000, type: "u8" }] }]) assert.throws(() => validateTarget(target), undefined, JSON.stringify(target));
});

test("every probe avoids single quotes, so none can break out of sh -c", () => {
  const layout = [{ field: "a", offset: 0, type: "u32" }];
  for (const target of [{ kind: "process", pid: 1 }, { kind: "process", name: "sshd" }, { kind: "thread", pid: 1, tid: 2 }, { kind: "log", path: "/var/log/x" }, { kind: "sharedMemory", name: "r", access: "posix-shm", layout }]) assert.ok(!probeScript(target).includes("'"));
});

test("a real process reports CPU, memory and existence, and a gone one says so", { skip: !unix }, async () => {
  const child = spawn("sleep", ["30"]);
  const target = validateTarget({ kind: "process", pid: child.pid });
  const sample = parseProbe(target, sh(probeScript(target)));
  assert.equal(sample.exists, true);
  assert.equal(sample.pid, child.pid);
  assert.ok(sample.memoryKb > 0);
  assert.ok(String(sample.command).includes("sleep"));
  child.kill();
  await new Promise((resolve) => child.once("exit", resolve));
  assert.equal(parseProbe(target, sh(probeScript(target))).exists, false);
  const missing = validateTarget({ kind: "process", name: "omniterm-no-such-process" });
  assert.equal(parseProbe(missing, sh(probeScript(missing))).exists, false);
});

test("a real log is followed from its end, across appends and rotation", { skip: !unix }, () => {
  const directory = mkdtempSync(path.join(os.tmpdir(), "omniterm-log-"));
  try {
    const file = path.join(directory, "app.log");
    writeFileSync(file, "old line\n");
    const target = validateTarget({ kind: "log", path: file });
    const first = parseProbe(target, sh(probeScript(target, {})));
    assert.equal(first.text, "");
    appendFileSync(file, "new ERROR line\n");
    const second = parseProbe(target, sh(probeScript(target, { offset: first.nextOffset })));
    assert.equal(second.text, "new ERROR line\n");
    writeFileSync(file, "x\n");
    assert.equal(parseProbe(target, sh(probeScript(target, { offset: second.nextOffset }))).text, "x\n");
    assert.equal(parseProbe(target, sh(probeScript(validateTarget({ kind: "log", path: `${file}.missing` })))).exists, false);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});

// Each replacement is a new file at the same path, however it got there.
async function followsReplacedLogs(read, file) {
  const next = async (state) => { const sample = await read(state); return [sample, { offset: sample.nextOffset, mark: sample.mark }]; };
  writeFileSync(file, "old\n");
  let [sample, state] = await next({});
  assert.equal(sample.text, "");

  renameSync(file, `${file}.1`);
  writeFileSync(file, "ERROR new log\n");
  [sample, state] = await next(state);
  assert.equal(sample.text, "ERROR new log\n", "larger replacement");

  appendFileSync(file, "日志 again\n");
  [sample, state] = await next(state);
  assert.equal(sample.text, "日志 again\n", "an append stays an append");
  [sample, state] = await next(state);
  assert.equal(sample.text, "", "nothing new");

  const size = Buffer.byteLength("ERROR new log\n日志 again\n");
  renameSync(file, `${file}.2`);
  writeFileSync(file, "E".repeat(size - 1) + "\n");
  [sample, state] = await next(state);
  assert.equal(sample.text, "E".repeat(size - 1) + "\n", "equal-sized replacement");

  rmSync(file);
  writeFileSync(file, "ERROR after a delete, in a longer file than before it was\n".repeat(3));
  [sample, state] = await next(state);
  assert.match(sample.text, /^ERROR after/, "deleted and recreated, perhaps on the same inode");

  writeFileSync(file, "ERROR rewritten in place, keeping the inode\n".repeat(4));
  [sample, state] = await next(state);
  assert.match(sample.text, /^ERROR rewritten/, "rewritten in place and larger");

  writeFileSync(file, "x\n");
  [sample, state] = await next(state);
  assert.equal(sample.text, "x\n", "truncated");
  appendFileSync(file, "y\n");
  [sample] = await next(state);
  assert.equal(sample.text, "y\n");
}

test("a real log is read from its start whenever the file behind the path changes", { skip: !unix }, async () => {
  const directory = mkdtempSync(path.join(os.tmpdir(), "omniterm-log-"));
  try {
    const file = path.join(directory, "app.log");
    const target = validateTarget({ kind: "log", path: file });
    await followsReplacedLogs(async (state) => parseProbe(target, sh(probeScript(target, state))), file);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});

test("a long read keeps raw byte offsets, even when a character is split", { skip: !unix }, () => {
  const directory = mkdtempSync(path.join(os.tmpdir(), "omniterm-log-"));
  try {
    const file = path.join(directory, "app.log");
    const target = validateTarget({ kind: "log", path: file });
    writeFileSync(file, "");
    const start = parseProbe(target, sh(probeScript(target, {})));
    const body = "日".repeat(30_000);
    appendFileSync(file, body);
    const first = parseProbe(target, sh(probeScript(target, { offset: start.nextOffset, mark: start.mark })));
    assert.equal(first.nextOffset, 64 * 1024);
    const second = parseProbe(target, sh(probeScript(target, { offset: first.nextOffset, mark: first.mark })));
    assert.equal(second.nextOffset, Buffer.byteLength(body));
  } finally { rmSync(directory, { recursive: true, force: true }); }
});

test("a mark that is not one of ours never reaches a script", () => {
  const target = validateTarget({ kind: "log", path: "/tmp/app.log" });
  assert.doesNotMatch(probeScript(target, { offset: 4, mark: "x\"; rm -rf /; \"" }), /rm -rf/);
});

test("a real shared-memory region is read through its declared layout", { skip: !existsSync("/dev/shm") }, () => {
  const name = `omniterm-test-${process.pid}`;
  const bytes = Buffer.alloc(16);
  bytes.writeUInt32LE(4242, 0);
  bytes.writeInt16LE(-7, 4);
  bytes.writeDoubleLE(1.5, 8);
  writeFileSync(`/dev/shm/${name}`, bytes);
  try {
    const target = validateTarget({ kind: "sharedMemory", name, access: "posix-shm", layout: [{ field: "count", offset: 0, type: "u32" }, { field: "delta", offset: 4, type: "i16" }, { field: "ratio", offset: 8, type: "f64" }] });
    const sample = parseProbe(target, sh(probeScript(target)));
    assert.deepEqual([sample.exists, sample.count, sample.delta, sample.ratio], [true, 4242, -7, 1.5]);
  } finally { rmSync(`/dev/shm/${name}`, { force: true }); }
});

test("conditions cover numbers, text and disappearance", () => {
  assert.equal(conditionHolds(validateCondition({ field: "cpu", op: ">", value: 50 }), { cpu: 80 }), true);
  assert.equal(conditionHolds(validateCondition({ field: "cpu", op: ">", value: 50 }), { cpu: 10 }), false);
  assert.equal(conditionHolds(validateCondition({ field: "cpu", op: ">", value: 50 }), {}), false, "missing data never fires");
  assert.equal(conditionHolds(validateCondition({ field: "text", op: "contains", value: "ERROR" }), { text: "an ERROR" }), true);
  assert.equal(conditionHolds(validateCondition({ op: "gone" }), { exists: false }), true);
  assert.throws(() => validateCondition({ field: "cpu", op: "exec", value: 1 }));
  assert.throws(() => validateCondition({ field: "cpu", op: ">", value: "1" }));
});

function fixture(samples = []) {
  const calls = [];
  const events = [];
  const timers = [];
  let time = 1000;
  const host = { calls, request: async (method, params) => { calls.push([method, params]); return method === "operation.propose" ? { operationId: `op-${calls.length}`, status: host.proposalStatus } : true; }, proposalStatus: "approved" };
  const service = new MonitorService({ host, emit: (event) => events.push(event), now: () => time, run: async () => { const next = samples.shift(); if (next instanceof Error) throw next; return next ?? "exists=1\ncpu=1\n"; }, schedule: (callback) => { timers.push(callback); return timers.length; }, unschedule: () => {} });
  const tick = async () => { await timers.shift()(); };
  return { service, host, calls, events, tick, advance: (ms) => { time += ms; } };
}

const request = { host: "local", target: { kind: "process", pid: 1 }, conditions: [{ field: "cpu", op: ">", value: 50 }], intervalSeconds: 5, durationMinutes: 10 };
const action = { command: "restart-worker", terminal: { sessionId: "b", epoch: 1 }, maxRuns: 2 };
const settle = () => new Promise((resolve) => setImmediate(resolve));

test("nothing is collected until a person approves the task", async () => {
  const { service } = fixture();
  let ran = 0;
  service.run = async () => { ran++; return "exists=1\n"; };
  const monitor = service.propose({ ...request, proposedBy: "agent" });
  assert.equal(monitor.status, "pending");
  await settle();
  assert.equal(ran, 0);
  await service.approve(monitor.id);
  await settle();
  assert.equal(ran, 1);
});

test("a denied proposal leaves nothing behind", () => {
  const { service } = fixture();
  const { id } = service.propose(request);
  service.deny(id);
  assert.deepEqual(service.list(), []);
});

test("sampling repeats on its own, and notifies once per crossing", async () => {
  const { service, events, tick } = fixture(["exists=1\ncpu=90\n", "exists=1\ncpu=95\n", "exists=1\ncpu=5\n", "exists=1\ncpu=99\n"]);
  const { id } = service.propose(request);
  await service.approve(id);
  await settle();
  for (let index = 0; index < 3; index++) { await tick(); await settle(); }
  assert.equal(events.filter(({ type }) => type === "monitor-alert").length, 2);
  assert.equal(service.get(id).series.length, 4);
});

test("a condition can require several samples in a row", async () => {
  const { service, events, tick } = fixture(["exists=1\ncpu=90\n", "exists=1\ncpu=5\n", "exists=1\ncpu=90\n", "exists=1\ncpu=90\n"]);
  const { id } = service.propose({ ...request, conditions: [{ field: "cpu", op: ">", value: 50, forSamples: 2 }] });
  await service.approve(id);
  await settle();
  for (let index = 0; index < 3; index++) { await tick(); await settle(); }
  assert.equal(events.filter(({ type }) => type === "monitor-alert").length, 1);
});

test("approval grants exactly the action's terminal until the task expires", async () => {
  const { service, calls } = fixture();
  const { id } = service.propose({ ...request, action });
  await service.approve(id);
  assert.deepEqual(calls[0], ["task.begin", { taskId: id, context: [], targets: [{ sessionId: "b", epoch: 1 }] }]);
  assert.deepEqual(calls[1], ["task.grant", { taskId: id, target: { sessionId: "b", epoch: 1 }, expiresAt: 1000 + 600_000 }]);
});

test("a read-only monitor never touches the broker", async () => {
  const { service, calls, tick } = fixture(["exists=1\ncpu=90\n"]);
  const { id } = service.propose(request);
  await service.approve(id);
  await settle();
  await tick();
  assert.deepEqual(calls, []);
});

test("a trigger runs the fixed command through the broker, up to its limit", async () => {
  const hot = "exists=1\ncpu=90\n";
  const cold = "exists=1\ncpu=1\n";
  const { service, calls, tick } = fixture([hot, cold, hot, cold, hot]);
  const { id } = service.propose({ ...request, action });
  await service.approve(id);
  await settle();
  for (let index = 0; index < 4; index++) { await tick(); await settle(); }
  const proposals = calls.filter(([method]) => method === "operation.propose");
  assert.equal(proposals.length, 2);
  assert.ok(proposals.every(([, params]) => params.command === "restart-worker" && params.terminal === "b"));
  assert.equal(calls.filter(([method]) => method === "operation.execute").length, 2);
  assert.ok(service.get(id).history.some(({ message }) => message.includes("次数上限")));
});

test("an action the broker did not authorize is recorded and not executed", async () => {
  const { service, host, calls } = fixture(["exists=1\ncpu=90\n"]);
  host.proposalStatus = "pendingApproval";
  const { id } = service.propose({ ...request, action });
  await service.approve(id);
  await settle();
  assert.equal(calls.filter(([method]) => method === "operation.execute").length, 0);
  assert.ok(service.get(id).history.some(({ message }) => message.includes("未获授权")));
});

test("pausing stops sampling and actions, and resuming continues", async () => {
  const { service, tick } = fixture();
  const { id } = service.propose(request);
  await service.approve(id);
  await settle();
  service.pause(id);
  let ran = 0;
  service.run = async () => { ran++; return "exists=1\n"; };
  await tick();
  await settle();
  assert.equal(ran, 0);
  service.resume(id);
  await settle();
  assert.equal(ran, 1);
});

test("stopping or revoking ends the broker task", async () => {
  for (const end of ["stop", "revokeAction"]) {
    const { service, calls } = fixture();
    const { id } = service.propose({ ...request, action });
    await service.approve(id);
    await settle();
    await service[end](id);
    assert.ok(calls.some(([method]) => method === "task.end"), end);
    if (end === "revokeAction") assert.equal(service.get(id).action, null);
  }
});

test("a task ends itself at its duration", async () => {
  const { service, calls, tick, advance } = fixture();
  const { id } = service.propose({ ...request, action });
  await service.approve(id);
  await settle();
  advance(600_001);
  await tick();
  assert.equal(service.get(id).status, "expired");
  assert.ok(calls.some(([method]) => method === "task.end"));
});

test("repeated sampling failures stop the task and alert", async () => {
  const failure = () => new Error("connection lost");
  const { service, events, tick } = fixture([failure(), failure(), failure(), failure(), failure()]);
  const { id } = service.propose({ ...request, action });
  await service.approve(id);
  await settle();
  for (let index = 0; index < 4; index++) { await tick(); await settle(); }
  assert.equal(service.get(id).status, "failed");
  assert.ok(events.some(({ type, message }) => type === "monitor-alert" && message.includes("失败")));
});

// The host answers each named method only when the test releases it.
function heldHost(...held) {
  const calls = [];
  const release = {};
  const started = {};
  for (const method of held) started[method] = new Promise((resolve) => { release[`${method}:started`] = resolve; });
  const host = { request: async (method) => {
    calls.push(method);
    if (held.includes(method)) {
      release[`${method}:started`]();
      return new Promise((resolve) => { release[method] = resolve; });
    }
    return method === "operation.propose" ? { operationId: "op", status: "approved" } : true;
  } };
  return { host, calls, release, started };
}

function heldService(host, run = async () => "exists=1\ncpu=80\n") {
  const timers = [];
  const service = new MonitorService({ host, now: () => 1000, run, schedule: (callback) => { timers.push(callback); return timers.length; }, unschedule: () => {} });
  return { service, timers };
}

test("a pause while a proposal is in flight sends nothing", async () => {
  const { host, calls, release, started } = heldHost("operation.propose");
  const { service, timers } = heldService(host);
  const { id } = service.propose({ ...request, action });
  await service.approve(id);
  await started["operation.propose"];
  await service.pause(id);
  release["operation.propose"]({ operationId: "op", status: "approved" });
  await settle();
  assert.equal(calls.filter((method) => method === "operation.execute").length, 0);
  assert.equal(service.get(id).status, "paused");
  assert.equal(timers.length, 0);
  assert.match(service.get(id).history.at(-1).message, /未执行/);
});

test("a revocation while a proposal is in flight sends nothing, and sampling goes on", async () => {
  const { host, calls, release, started } = heldHost("operation.propose");
  const { service, timers } = heldService(host);
  const { id } = service.propose({ ...request, action });
  await service.approve(id);
  await started["operation.propose"];
  await service.revokeAction(id);
  release["operation.propose"]({ operationId: "op", status: "approved" });
  await settle();
  assert.equal(calls.filter((method) => method === "operation.execute").length, 0);
  assert.equal(service.get(id).status, "running");
  assert.equal(timers.length, 1);
});

test("a paused task holds no authority in the broker, and resuming restores exactly its own", async () => {
  const { service, calls, host } = fixture();
  const { id } = service.propose({ ...request, action });
  await service.approve(id);
  await settle();
  calls.length = 0;
  await service.pause(id);
  // task.end would interrupt a running command; removing the target voids only what is unwritten.
  assert.deepEqual(calls, [["task.update", { taskId: id, context: [], targets: [], paused: true }]]);
  calls.length = 0;
  host.proposalStatus = "pending";
  await service.resume(id);
  assert.deepEqual(calls.slice(0, 2), [
    ["task.update", { taskId: id, context: [], targets: [action.terminal], paused: false }],
    ["task.grant", { taskId: id, target: action.terminal, expiresAt: service.get(id).expiresAt }],
  ]);
});

test("a stop during approval wins, and the broker never sees a grant after it", async () => {
  const { host, calls, release, started } = heldHost("task.begin");
  const { service, timers } = heldService(host);
  const { id } = service.propose({ ...request, action });
  const approving = service.approve(id);
  await started["task.begin"];
  await service.stop(id);
  release["task.begin"](true);
  await approving;
  await settle();
  assert.deepEqual(calls, ["task.begin", "task.end"]);
  assert.equal(service.get(id).status, "stopped");
  assert.equal(timers.length, 0);
});

test("a denial during approval leaves no task and no grant", async () => {
  const { host, calls, release, started } = heldHost("task.begin");
  const { service } = heldService(host);
  const { id } = service.propose({ ...request, action });
  const approving = service.approve(id);
  await started["task.begin"];
  await assert.rejects(service.approve(id), /不在等待批准/);
  service.deny(id);
  release["task.begin"](true);
  await approving;
  assert.deepEqual(calls, ["task.begin", "task.end"]);
  assert.equal(service.list().length, 0);
});

test("a revocation during approval leaves a read-only monitor without a grant", async () => {
  const { host, calls, release, started } = heldHost("task.begin");
  const { service } = heldService(host);
  const { id } = service.propose({ ...request, action });
  const approving = service.approve(id);
  await started["task.begin"];
  await service.revokeAction(id);
  release["task.begin"](true);
  await approving;
  assert.deepEqual(calls.slice(0, 2), ["task.begin", "task.end"]);
  assert.ok(!calls.includes("task.grant"));
  assert.equal(service.get(id).status, "running");
});

test("a stop while resuming wins, and no grant or sample follows it", async () => {
  const { host, calls, release } = heldHost("task.update");
  let ran = 0;
  const { service } = heldService(host, async () => { ran++; return "exists=1\ncpu=1\n"; });
  const { id } = service.propose({ ...request, action });
  await service.approve(id);
  await settle();
  const pausing = service.pause(id);
  release["task.update"](true);
  await pausing;
  const before = ran;
  calls.length = 0;
  const resuming = service.resume(id);
  await settle();
  await service.stop(id);
  release["task.update"](true);
  await resuming;
  await settle();
  assert.deepEqual(calls, ["task.update", "task.end"]);
  assert.equal(service.get(id).status, "stopped");
  assert.equal(ran, before);
});

test("resuming with a sample still in flight leaves one sampling loop", async () => {
  const { host } = heldHost();
  const pending = [];
  const { service, timers } = heldService(host, () => new Promise((resolve) => pending.push(resolve)));
  const { id } = service.propose(request);
  await service.approve(id);
  await settle();
  await service.pause(id);
  await service.resume(id);
  await settle();
  assert.equal(pending.length, 2);
  pending[1]("exists=1\ncpu=2\n");
  await settle();
  pending[0]("exists=1\ncpu=99\n");
  await settle();
  assert.equal(timers.length, 1);
  assert.equal(service.get(id).lastSample.cpu, 2, "the older sample is dropped");
  assert.equal(service.get(id).series.length, 1);
});

test("a timer left over from before a pause cannot start a second loop", async () => {
  const { service, tick } = fixture();
  let ran = 0;
  const { id } = service.propose(request);
  await service.approve(id);
  await settle();
  await service.pause(id);
  service.run = async () => { ran++; return "exists=1\n"; };
  await service.resume(id);
  await settle();
  assert.equal(ran, 1);
  await tick();
  await settle();
  assert.equal(ran, 1);
});

test("a stopped task stays stopped, and a finished one keeps its outcome", async () => {
  const { service, calls } = fixture();
  const { id } = service.propose({ ...request, action });
  await service.approve(id);
  await settle();
  await service.stop(id);
  const ends = calls.filter(([method]) => method === "task.end").length;
  await service.stop(id);
  assert.equal(calls.filter(([method]) => method === "task.end").length, ends);
});

test("a log absent at the first sample is read from its start once it appears", async () => {
  const seen = [];
  const { service } = fixture();
  const outputs = ["exists=0\n", "exists=1\noffset=0\nnext=6\nmark=1-2\n@@text\nERROR\n"];
  service.run = async (host, script) => { seen.push(script); return outputs.shift() ?? "exists=0\n"; };
  const { id } = service.propose({ ...request, target: { kind: "log", path: "/tmp/absent.log" }, conditions: [{ field: "text", op: "contains", value: "ERROR" }] });
  await service.approve(id);
  await settle();
  assert.match(seen[0], /start=\$size/);
  const monitor = service.monitors.get(id);
  assert.deepEqual(monitor.probeState, { offset: 0 });
  await service.sample(monitor);
  assert.deepEqual(monitor.probeState, { offset: 6, mark: "1-2" });
  assert.equal(monitor.triggers, 1);
  assert.equal(service.get(id).lastSample.mark, undefined);
});

test("requests that cannot be honoured are refused up front", () => {
  const { service } = fixture();
  assert.throws(() => service.propose({ ...request, host: null }), /主机/);
  assert.throws(() => service.propose({ ...request, intervalSeconds: 0 }), /采样间隔/);
  assert.throws(() => service.propose({ ...request, durationMinutes: 99_999_999 }), /时长/);
  assert.throws(() => service.propose({ ...request, conditions: [], action }), /触发条件/);
  assert.throws(() => service.propose({ ...request, action: { ...action, command: "a\nb" } }), /一行/);
  assert.throws(() => service.propose({ ...request, action: { ...action, terminal: { sessionId: "b" } } }), /目标终端/);
});

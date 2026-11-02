// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import { randomUUID } from "node:crypto";
import { conditionHolds, parseProbe, probeScript, validateCondition, validateTarget, withCpuRate } from "./monitor-probe.mjs";

const MAX_MONITORS = 16;
const MAX_HISTORY = 200;
const MAX_SERIES = 120;
const MAX_FAILURES = 5;
const MIN_INTERVAL_S = 1;
const MAX_INTERVAL_S = 3600;
const MAX_DURATION_MIN = 7 * 24 * 60;
const MAX_ACTIONS = 100;
const REF = (value) => ({ sessionId: value?.sessionId, epoch: value?.epoch });

function validateRequest(input = {}, localPlatform) {
  const hostRef = input.host === "local" ? "local" : typeof input.host?.sessionId === "string" ? { sessionId: input.host.sessionId } : null;
  if (!hostRef) throw new Error("请选择监控所在的主机。");
  const intervalSeconds = input.intervalSeconds ?? 5;
  const durationMinutes = input.durationMinutes ?? 60;
  if (!Number.isFinite(intervalSeconds) || intervalSeconds < MIN_INTERVAL_S || intervalSeconds > MAX_INTERVAL_S) throw new Error(`采样间隔必须是 ${MIN_INTERVAL_S}–${MAX_INTERVAL_S} 秒。`);
  if (!Number.isFinite(durationMinutes) || durationMinutes <= 0 || durationMinutes > MAX_DURATION_MIN) throw new Error("监控时长必须大于 0，且不超过 7 天。");
  if (!Array.isArray(input.conditions) || input.conditions.length > 8) throw new Error("最多可以设置 8 个条件。");
  const action = input.action ? validateAction(input.action) : null;
  if (action && !input.conditions.length) throw new Error("自动操作需要至少一个触发条件。");
  return { host: hostRef, label: typeof input.label === "string" ? input.label.slice(0, 80) : "", target: validateTarget(input.target, { platform: hostRef === "local" ? localPlatform : "linux" }), conditions: input.conditions.map(validateCondition), intervalSeconds, durationMinutes, action, proposedBy: input.proposedBy === "agent" ? "agent" : "user" };
}

function validateAction({ command, terminal, maxRuns = 1 } = {}) {
  if (typeof command !== "string" || !command.trim() || command.length > 4096 || /[\u0000-\u0008\u000a-\u001f\u007f]/.test(command)) throw new Error("自动操作的命令必须是一行文本。");
  if (typeof terminal?.sessionId !== "string" || !Number.isSafeInteger(terminal.epoch)) throw new Error("请选择自动操作的目标终端。");
  if (!Number.isSafeInteger(maxRuns) || maxRuns < 1 || maxRuns > MAX_ACTIONS) throw new Error(`自动操作次数必须是 1–${MAX_ACTIONS}。`);
  return { command, terminal: REF(terminal), maxRuns };
}

// Sampling costs no model tokens and touches no terminal. Actions are fixed when the user
// approves the task: one exact command on one pinned terminal, run through the broker with
// its usual checks, for at most the task's duration and `maxRuns` times.
export class MonitorService {
  constructor({ run, host, emit = () => {}, now = Date.now, schedule = setTimeout, unschedule = clearTimeout, platform = process.platform }) {
    Object.assign(this, { run, host, emit, now, schedule, unschedule, platform });
    this.monitors = new Map();
    this.operations = new Map();
  }
  view({ timer, probeState, streaks, generation, approving, ...monitor }) { return monitor; }
  list() { return [...this.monitors.values()].map((monitor) => this.view(monitor)); }
  get(id) {
    const monitor = this.monitors.get(id);
    if (!monitor) throw new Error("这个监控任务不存在。");
    return monitor;
  }
  publish(monitor) { this.emit({ type: "monitor", monitor: this.view(monitor) }); }
  record(monitor, kind, message, detail = {}) {
    monitor.history.push({ at: this.now(), kind, message, ...detail });
    if (monitor.history.length > MAX_HISTORY) monitor.history.shift();
  }
  // A proposal only records intent. Nothing is collected until the user approves it.
  propose(input) {
    if (this.monitors.size >= MAX_MONITORS) throw new Error(`最多可以保留 ${MAX_MONITORS} 个监控任务，请先删除不需要的任务。`);
    const request = validateRequest(input, this.platform);
    const monitor = { id: randomUUID(), ...request, status: "pending", createdAt: this.now(), startedAt: null, expiresAt: null, lastSample: null, series: [], history: [], triggers: 0, actionRuns: 0, failures: 0, timer: null, probeState: {}, streaks: new Map(), generation: 0, approving: false };
    this.monitors.set(monitor.id, monitor);
    this.record(monitor, "state", request.proposedBy === "agent" ? "Agent 提议了这个任务" : "已创建");
    this.publish(monitor);
    return this.view(monitor);
  }
  // Every lifecycle change moves the generation on. Whatever waited on the host compares the
  // generation it started with, so older work can never outlive a pause, a stop or a removal.
  advance(monitor) {
    this.clearTimer(monitor);
    return ++monitor.generation;
  }
  current(monitor, generation) { return this.monitors.get(monitor.id) === monitor && monitor.generation === generation; }
  // Requests for one task reach the host in the order they were sent, so a stop sent after
  // `open` also lands after it. Each later step only has to check it is still wanted.
  async authorize(monitor, generation, open) {
    const { action } = monitor;
    if (!action) return;
    await this.host.request(...open(action.terminal));
    if (!this.current(monitor, generation) || monitor.action !== action) return;
    await this.host.request("task.grant", { taskId: monitor.id, target: action.terminal, expiresAt: monitor.expiresAt });
  }
  endTask(monitor) {
    return this.host.request("task.end", { taskId: monitor.id }).catch((error) => this.record(monitor, "error", `结束自动操作授权失败：${error.message}`));
  }
  async approve(id) {
    const monitor = this.get(id);
    if (monitor.status !== "pending" || monitor.approving) throw new Error("这个任务不在等待批准。");
    const generation = this.advance(monitor);
    monitor.startedAt = this.now();
    monitor.expiresAt = monitor.startedAt + monitor.durationMinutes * 60_000;
    monitor.approving = true;
    try {
      await this.authorize(monitor, generation, (target) => ["task.begin", { taskId: monitor.id, context: [], targets: [target] }]);
    } finally { monitor.approving = false; }
    if (!this.current(monitor, generation)) return this.view(monitor);
    monitor.status = "running";
    this.record(monitor, "state", "已批准并开始监控");
    this.publish(monitor);
    this.sample(monitor);
    return this.view(monitor);
  }
  deny(id) {
    const monitor = this.get(id);
    if (monitor.status !== "pending") throw new Error("这个任务不在等待批准。");
    this.advance(monitor);
    if (monitor.approving && monitor.action) this.endTask(monitor);
    this.monitors.delete(id);
    this.emit({ type: "monitor-removed", id });
    return true;
  }
  // A paused task holds no authority. Its target is taken away, which drops the grant and voids
  // anything not yet written. Unlike ending the task, this leaves a running command uninterrupted.
  async pause(id) {
    const monitor = this.get(id);
    if (monitor.status !== "running") return this.view(monitor);
    this.advance(monitor);
    monitor.status = "paused";
    this.record(monitor, "state", "已暂停：不再采样，也不会触发操作");
    this.publish(monitor);
    if (monitor.action) await this.host.request("task.update", { taskId: monitor.id, context: [], targets: [], paused: true }).catch((error) => this.record(monitor, "error", `暂停自动操作授权失败：${error.message}`));
    return this.view(monitor);
  }
  async resume(id) {
    const monitor = this.get(id);
    if (monitor.status !== "paused") return this.view(monitor);
    const generation = this.advance(monitor);
    monitor.status = "running";
    monitor.failures = 0;
    this.record(monitor, "state", "已恢复");
    this.publish(monitor);
    try {
      await this.authorize(monitor, generation, (target) => ["task.update", { taskId: monitor.id, context: [], targets: [target], paused: false }]);
    } catch (error) {
      if (!this.current(monitor, generation)) return this.view(monitor);
      this.advance(monitor);
      monitor.status = "paused";
      this.record(monitor, "error", `无法恢复自动操作授权，任务保持暂停：${error.message}`);
      this.publish(monitor);
      throw error;
    }
    if (this.current(monitor, generation)) this.sample(monitor);
    return this.view(monitor);
  }
  clearTimer(monitor) {
    if (monitor.timer !== null) this.unschedule(monitor.timer);
    monitor.timer = null;
  }
  // Ending the broker task revokes the grant and cancels anything it had waiting.
  async revokeAction(id) {
    const monitor = this.get(id);
    if (!monitor.action) return this.view(monitor);
    monitor.action = null;
    await this.endTask(monitor);
    this.record(monitor, "state", "已撤销自动操作，继续只读监控");
    this.publish(monitor);
    return this.view(monitor);
  }
  async finish(monitor, status, message) {
    this.advance(monitor);
    monitor.status = status;
    if (monitor.action) await this.endTask(monitor);
    this.record(monitor, "state", message);
    this.publish(monitor);
  }
  async stop(id) {
    const monitor = this.get(id);
    if (!["pending", "running", "paused"].includes(monitor.status)) return this.view(monitor);
    await this.finish(monitor, "stopped", "已停止");
    return this.view(monitor);
  }
  remove(id) {
    const monitor = this.get(id);
    if (monitor.approving || ["running", "paused"].includes(monitor.status)) throw new Error("请先停止这个任务。");
    this.monitors.delete(id);
    this.emit({ type: "monitor-removed", id });
    return true;
  }
  async sample(monitor) {
    const { generation } = monitor;
    const live = () => this.current(monitor, generation);
    this.clearTimer(monitor);
    if (monitor.status !== "running") return;
    if (this.now() >= monitor.expiresAt) { await this.finish(monitor, "expired", "已到达设定时长"); return; }
    try {
      const platform = monitor.host === "local" ? this.platform : "linux";
      const output = await this.run(monitor.host, probeScript(monitor.target, monitor.probeState, platform));
      if (!live()) return;
      const { mark, ...sample } = withCpuRate({ ...parseProbe(monitor.target, output), at: this.now() }, monitor.lastSample);
      // A log that is absent now is new when it appears, so all of it counts.
      if (monitor.target.kind === "log") monitor.probeState = sample.nextOffset === undefined ? { offset: 0 } : { offset: sample.nextOffset, mark };
      monitor.failures = 0;
      monitor.lastSample = sample.text === undefined ? sample : { ...sample, text: sample.text.slice(-2000) };
      const { text, ...numbers } = monitor.lastSample;
      monitor.series.push(numbers);
      if (monitor.series.length > MAX_SERIES) monitor.series.shift();
      await this.evaluate(monitor, sample, generation);
      if (!live()) return;
    } catch (error) {
      if (!live()) return;
      monitor.failures++;
      this.record(monitor, "error", error.message);
      if (monitor.failures >= MAX_FAILURES) {
        await this.finish(monitor, "failed", `连续 ${MAX_FAILURES} 次采样失败，已停止`);
        this.emit({ type: "monitor-alert", id: monitor.id, label: monitor.label, message: "监控采样连续失败，任务已停止。" });
        return;
      }
    }
    this.publish(monitor);
    monitor.timer = this.schedule(() => { if (live()) this.sample(monitor); }, monitor.intervalSeconds * 1000);
  }
  // Fires on the transition into a condition, not on every sample while it holds.
  async evaluate(monitor, sample, generation) {
    for (const [index, condition] of monitor.conditions.entries()) {
      if (!this.current(monitor, generation)) return;
      const streak = conditionHolds(condition, sample) ? (monitor.streaks.get(index) ?? 0) + 1 : 0;
      monitor.streaks.set(index, streak);
      if (streak !== condition.forSamples) continue;
      monitor.triggers++;
      const message = `条件满足：${condition.field} ${condition.op} ${condition.value ?? ""}`.trim();
      this.record(monitor, "trigger", message);
      this.emit({ type: "monitor-alert", id: monitor.id, label: monitor.label, message });
      await this.act(monitor, generation);
    }
  }
  // The run stays counted if the task is paused before it is sent: `maxRuns` is a ceiling, and
  // the broker treats a repeated call ID as the same operation.
  async act(monitor, generation) {
    const { action } = monitor;
    if (!action) return;
    if (monitor.actionRuns >= action.maxRuns) { this.record(monitor, "action", "已达到自动操作次数上限，未执行"); return; }
    monitor.actionRuns++;
    try {
      const operation = await this.host.request("operation.propose", { taskId: monitor.id, callId: `monitor-${monitor.actionRuns}`, terminal: action.terminal.sessionId, command: action.command });
      this.operations.set(operation.operationId, monitor.id);
      if (operation.status !== "approved") { this.record(monitor, "action", `自动操作未获授权（${operation.status}），未执行`); return; }
      // The last check before anything is sent. The pause or stop already voided this in the broker.
      if (!this.current(monitor, generation) || monitor.action !== action) { this.record(monitor, "action", "任务已暂停、停止或撤销，自动操作未执行", { operationId: operation.operationId }); return; }
      await this.host.request("operation.execute", { operationId: operation.operationId, taskId: monitor.id });
      this.record(monitor, "action", "已发出自动操作", { operationId: operation.operationId, command: action.command });
    } catch (error) { this.record(monitor, "action", `自动操作未执行：${error.message}`); }
  }
  operationChanged(operation) {
    const monitor = this.monitors.get(this.operations.get(operation.operationId));
    if (!monitor) return;
    if (["completed", "blocked", "invalidated", "outcomeUnknown", "cancelled", "takenOver", "staged", "interrupted"].includes(operation.status)) {
      this.record(monitor, "action", `自动操作结果：${operation.status}${operation.exitCode !== null && operation.exitCode !== undefined ? `，退出码 ${operation.exitCode}` : ""}`, { operationId: operation.operationId, reason: operation.reason });
      this.publish(monitor);
    }
  }
  dispose() {
    for (const monitor of this.monitors.values()) {
      this.advance(monitor);
      if (monitor.status === "running" || monitor.status === "paused") monitor.status = "stopped";
    }
  }
}

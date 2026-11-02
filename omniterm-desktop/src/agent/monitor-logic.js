// Pure helpers for the monitoring view, kept free of JSX so tests can import them.

export const MONITOR_STATUS = { pending: "等待批准", running: "监控中", paused: "已暂停", stopped: "已停止", expired: "已到期", failed: "已失败" };

// Mirrors what a host can read, so a choice that can't work is explained before it is made.
export function kindNote(kind, host, platform) {
  if (host !== "local") return kind === "thread" || kind === "sharedMemory" ? "需要 Linux 远程主机。" : "";
  if (platform === "win32" && kind === "sharedMemory") return "Windows 本机暂不支持。";
  if (platform === "darwin" && (kind === "thread" || kind === "sharedMemory")) return "macOS 暂不支持。";
  return "";
}

export const emptyMonitorForm = Object.freeze({
  host: "local", kind: "process", process: "", pid: "", tid: "", path: "", region: "", layout: "", field: "", value: "",
  intervalSeconds: "5", durationMinutes: "60", withAction: false, command: "", terminal: "", maxRuns: "1",
});

const number = (value, label) => {
  const result = Number(value);
  if (value === "" || !Number.isFinite(result)) throw new Error(`请填写${label}。`);
  return result;
};

function targetOf(form) {
  switch (form.kind) {
    case "process": return /^\d+$/.test(form.process) ? { kind: "process", pid: Number(form.process) } : { kind: "process", name: form.process.trim() };
    case "thread": return { kind: "thread", pid: number(form.pid, "进程 ID"), tid: number(form.tid, "线程 ID") };
    case "log": return { kind: "log", path: form.path.trim() };
    default: return {
      kind: "sharedMemory", name: form.region.trim(), access: "posix-shm",
      layout: form.layout.split("\n").map((line) => line.trim()).filter(Boolean).map((line) => {
        const [field, offset, type] = line.split(":").map((part) => part.trim());
        return { field, offset: Number(offset), type };
      }),
    };
  }
}

function conditionOf({ field, value }) {
  if (!field) return [];
  if (field === "exists") return [{ op: "gone" }];
  if (field === "text") return [{ field, op: "contains", value }];
  return [{ field, op: ">", value: number(value, "条件数值") }];
}

export function buildMonitorRequest(form, terminals) {
  const conditions = conditionOf(form);
  const request = {
    host: form.host === "local" ? "local" : { sessionId: form.host },
    target: targetOf(form), conditions,
    intervalSeconds: number(form.intervalSeconds, "采样间隔"),
    durationMinutes: number(form.durationMinutes, "时长"),
  };
  if (!form.withAction) return request;
  const terminal = terminals.find(({ ref }) => ref.sessionId === form.terminal);
  if (!terminal) throw new Error("请选择自动操作的目标终端。");
  return { ...request, action: { command: form.command, terminal: terminal.ref, maxRuns: number(form.maxRuns, "最多执行次数") } };
}

export function monitorReducer(monitors, event) {
  if (event.type === "monitor-removed") return monitors.filter(({ id }) => id !== event.id);
  if (event.type !== "monitor") return monitors;
  return monitors.some(({ id }) => id === event.monitor.id) ? monitors.map((monitor) => monitor.id === event.monitor.id ? event.monitor : monitor) : [...monitors, event.monitor];
}

export function targetSummary(target) {
  switch (target.kind) {
    case "process": return `进程 ${target.pid ?? target.name}`;
    case "thread": return `线程 ${target.pid}/${target.tid}`;
    case "log": return target.path;
    default: return `共享内存 ${target.name}`;
  }
}

const HIDDEN = new Set(["at", "exists", "text", "size", "offset", "nextOffset", "pid", "command"]);

export function monitorMetrics(sample) {
  if (!sample) return [];
  if (sample.exists === false) return [["目标", "不存在"]];
  return Object.entries(sample).filter(([key, value]) => !HIDDEN.has(key) && typeof value === "number");
}

// Points for an SVG polyline, scaled to its own range.
export function sparkline(series, key) {
  const values = (series ?? []).map((sample) => sample[key]).filter((value) => typeof value === "number");
  if (!key || values.length < 2) return "";
  const [low, high] = [Math.min(...values), Math.max(...values)];
  const span = high - low || 1;
  return values.map((value, index) => `${(index / (values.length - 1) * 100).toFixed(1)},${(25 - (value - low) / span * 24).toFixed(1)}`).join(" ");
}

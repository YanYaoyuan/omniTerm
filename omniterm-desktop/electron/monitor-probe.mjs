// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
// What a monitor reads, from the host itself and never from a terminal screen. Each probe is one
// read-only POSIX sh script, so nothing is installed remotely. Scripts stay free of single quotes
// because they travel as `sh -c '<script>'`, and every value in them is validated here first.

const NAME = /^[\w.+@-]{1,128}$/;
const PATH = /^\/[\w./+@-]{0,4095}$/;
const MAX_LOG_BYTES = 64 * 1024;
const MAX_REGION_BYTES = 4096;
export const TARGET_KINDS = Object.freeze(["process", "thread", "log", "sharedMemory"]);
const FIELD_TYPES = { u8: 1, i8: 1, u16: 2, i16: 2, u32: 4, i32: 4, f32: 4, u64: 8, i64: 8, f64: 8 };
const OPERATORS = { ">": (a, b) => a > b, ">=": (a, b) => a >= b, "<": (a, b) => a < b, "<=": (a, b) => a <= b, "==": (a, b) => a === b, "!=": (a, b) => a !== b };

const integer = (value, label, maximum = Number.MAX_SAFE_INTEGER) => {
  if (!Number.isSafeInteger(value) || value < 0 || value > maximum) throw new Error(`${label}格式不正确。`);
  return value;
};

export function validateTarget(input = {}) {
  switch (input.kind) {
    case "process": {
      if (input.pid !== undefined) return { kind: "process", pid: integer(input.pid, "进程 ID") };
      if (!NAME.test(input.name ?? "")) throw new Error("请填写进程 ID 或进程名。");
      return { kind: "process", name: input.name };
    }
    case "thread": return { kind: "thread", pid: integer(input.pid, "进程 ID"), tid: integer(input.tid, "线程 ID") };
    case "log":
      if (!PATH.test(input.path ?? "") || input.path.includes("..")) throw new Error("日志路径必须是绝对路径，且只能包含常见文件名字符。");
      return { kind: "log", path: input.path };
    case "sharedMemory": {
      if (input.access !== "posix-shm") throw new Error("共享内存目前只支持只读的 POSIX 共享内存（/dev/shm）。");
      if (!NAME.test(input.name ?? "")) throw new Error("请填写共享内存区域名称。");
      if (!Array.isArray(input.layout) || !input.layout.length || input.layout.length > 32) throw new Error("请填写共享内存布局。");
      const layout = input.layout.map(({ field, offset, type }) => {
        if (!/^\w{1,64}$/.test(field ?? "") || !Object.hasOwn(FIELD_TYPES, type ?? "")) throw new Error("共享内存布局的字段或类型不正确。");
        if (integer(offset, "偏移量", MAX_REGION_BYTES) + FIELD_TYPES[type] > MAX_REGION_BYTES) throw new Error(`共享内存布局不能超过 ${MAX_REGION_BYTES} 字节。`);
        return { field, offset, type };
      });
      return { kind: "sharedMemory", name: input.name, access: "posix-shm", layout };
    }
    default: throw new Error("未知的监控目标。");
  }
}

function processScript(target) {
  const find = target.pid !== undefined ? `pid=${target.pid}` : `pid=$(pgrep -x ${target.name} 2>/dev/null | head -n 1)`;
  return `export LC_ALL=C
${find}
if [ -z "$pid" ] || ! kill -0 "$pid" 2>/dev/null && ! ps -p "$pid" >/dev/null 2>&1; then echo exists=0; exit 0; fi
echo exists=1
echo pid=$pid
ps -p "$pid" -o pcpu=,rss=,comm= | head -n 1 | { read -r cpu rss comm; echo cpu=$cpu; echo memoryKb=$rss; echo command=$comm; }
if [ -r "/proc/$pid/status" ]; then echo threads=$(grep "^Threads:" "/proc/$pid/status" | tr -cd 0-9); fi
`;
}

function threadScript({ pid, tid }) {
  return `export LC_ALL=C
if [ ! -d "/proc/${pid}/task/${tid}" ]; then echo exists=0; exit 0; fi
echo exists=1
ps -L -p ${pid} -o tid=,pcpu= | while read -r id cpu; do if [ "$id" = "${tid}" ]; then echo cpu=$cpu; fi; done
`;
}

function logScript({ path }, { offset }) {
  return `export LC_ALL=C
if [ ! -r "${path}" ]; then echo exists=0; exit 0; fi
echo exists=1
size=$(wc -c < "${path}" | tr -d " ")
echo size=$size
start=${offset ?? "$size"}
if [ "$start" -gt "$size" ]; then start=0; fi
echo offset=$start
echo @@text
tail -c +$((start + 1)) "${path}" | head -c ${MAX_LOG_BYTES}
`;
}

function regionScript({ name, layout }) {
  const length = Math.max(...layout.map(({ offset, type }) => offset + FIELD_TYPES[type]));
  return `export LC_ALL=C
if [ ! -r "/dev/shm/${name}" ]; then echo exists=0; exit 0; fi
echo exists=1
echo @@bytes
od -An -v -tx1 -N ${length} "/dev/shm/${name}" | tr -d " \\n"
`;
}

export function probeScript(target, state = {}) {
  const script = { process: processScript, thread: threadScript, log: logScript, sharedMemory: regionScript }[target.kind](target, state);
  if (script.includes("'")) throw new Error("Probe scripts must not contain single quotes.");
  return script;
}

function readField(buffer, { offset, type }) {
  if (offset + FIELD_TYPES[type] > buffer.length) return null;
  const readers = { u8: "readUInt8", i8: "readInt8", u16: "readUInt16LE", i16: "readInt16LE", u32: "readUInt32LE", i32: "readInt32LE", f32: "readFloatLE", f64: "readDoubleLE" };
  if (type === "u64" || type === "i64") return Number(type === "u64" ? buffer.readBigUInt64LE(offset) : buffer.readBigInt64LE(offset));
  return buffer[readers[type]](offset);
}

export function parseProbe(target, output) {
  const separator = output.search(/^@@(text|bytes)$/m);
  const head = separator < 0 ? output : output.slice(0, separator);
  const sample = {};
  for (const line of head.split("\n")) {
    const at = line.indexOf("=");
    if (at < 1) continue;
    const [key, value] = [line.slice(0, at), line.slice(at + 1).trim()];
    const number = Number(value);
    sample[key] = value !== "" && Number.isFinite(number) ? number : value;
  }
  sample.exists = sample.exists === 1;
  if (separator >= 0) {
    const body = output.slice(output.indexOf("\n", separator) + 1);
    if (target.kind === "log") {
      sample.text = body;
      sample.nextOffset = (sample.offset ?? 0) + Buffer.byteLength(body);
    } else {
      const bytes = Buffer.from(body.replace(/[^0-9a-f]/g, ""), "hex");
      for (const field of target.layout) sample[field.field] = readField(bytes, field);
    }
  }
  return sample;
}

export function validateCondition(input = {}) {
  const forSamples = input.forSamples === undefined ? 1 : integer(input.forSamples, "连续次数", 1000);
  if (input.op === "gone") return { field: "exists", op: "gone", forSamples };
  if (!/^\w{1,64}$/.test(input.field ?? "")) throw new Error("请选择要判断的字段。");
  if (input.op === "contains") {
    if (typeof input.value !== "string" || !input.value || input.value.length > 500) throw new Error("请填写要匹配的文本。");
    return { field: input.field, op: "contains", value: input.value, forSamples };
  }
  if (!Object.hasOwn(OPERATORS, input.op ?? "") || !Number.isFinite(input.value)) throw new Error("条件必须是一个比较和一个数值。");
  return { field: input.field, op: input.op, value: input.value, forSamples };
}

export function conditionHolds(condition, sample) {
  if (condition.op === "gone") return sample.exists === false;
  const value = sample[condition.field];
  if (condition.op === "contains") return typeof value === "string" && value.includes(condition.value);
  return typeof value === "number" && OPERATORS[condition.op](value, condition.value);
}

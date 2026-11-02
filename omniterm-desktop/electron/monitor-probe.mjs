// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
// What a monitor reads, from the host itself and never from a terminal screen. Each probe is one
// read-only POSIX sh script, so nothing is installed remotely. Scripts stay free of single quotes
// because they travel as `sh -c '<script>'`, and every value in them is validated here first.

const NAME = /^[\w.+@-]{1,128}$/;
// Letters and digits of any language, but no quotes of any kind: PowerShell also treats curly
// quotes as quotes, and these paths sit inside quoted strings.
const PATH = /^\/[\p{L}\p{N}_ ./+@~#,-]{0,4095}$/u;
const WINDOWS_PATH = /^[A-Za-z]:\\[\p{L}\p{N}_ .\\+@()~#,-]{0,4000}$/u;
const MAX_LOG_BYTES = 64 * 1024;
const LOG_MARK_BYTES = 256;
const MAX_REGION_BYTES = 4096;
export const TARGET_KINDS = Object.freeze(["process", "thread", "log", "sharedMemory"]);
const FIELD_TYPES = { u8: 1, i8: 1, u16: 2, i16: 2, u32: 4, i32: 4, f32: 4, u64: 8, i64: 8, f64: 8 };
const OPERATORS = { ">": (a, b) => a > b, ">=": (a, b) => a >= b, "<": (a, b) => a < b, "<=": (a, b) => a <= b, "==": (a, b) => a === b, "!=": (a, b) => a !== b };

const integer = (value, label, maximum = Number.MAX_SAFE_INTEGER) => {
  if (!Number.isSafeInteger(value) || value < 0 || value > maximum) throw new Error(`${label}格式不正确。`);
  return value;
};

// What each kind of host can read. Remote hosts are reached through a POSIX shell.
export function targetSupport(kind, platform) {
  if (platform === "win32") return kind === "sharedMemory" ? "Windows 本机暂不支持共享内存监控。" : null;
  if (platform === "darwin") return kind === "thread" || kind === "sharedMemory" ? "macOS 暂不支持这类监控。" : null;
  return null;
}

export function validateTarget(input = {}, { platform = "linux" } = {}) {
  const unsupported = targetSupport(input.kind, platform);
  if (unsupported) throw new Error(unsupported);
  switch (input.kind) {
    case "process": {
      if (input.pid !== undefined) return { kind: "process", pid: integer(input.pid, "进程 ID") };
      if (!NAME.test(input.name ?? "")) throw new Error("请填写进程 ID 或进程名。");
      return { kind: "process", name: input.name };
    }
    case "thread": return { kind: "thread", pid: integer(input.pid, "进程 ID"), tid: integer(input.tid, "线程 ID") };
    case "log":
      if (!(platform === "win32" ? WINDOWS_PATH : PATH).test(input.path ?? "") || input.path.includes("..")) throw new Error("日志路径必须是绝对路径，且只能包含常见文件名字符。");
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

// A saved mark comes back from a remote host and goes into a script, so it is checked first.
const savedMark = (mark) => (typeof mark === "string" && /^[\w-]{1,128}$/.test(mark) ? mark : null);

// An offset only means something in the file it was taken from. The mark names that file: its
// inode, which a rename changes, and a checksum of the bytes before the offset, which also catches
// a reused inode and a file rewritten in place. A mismatch means a new file, read from its start.
// The inode is read first, so a rotation during this sample shows up in the next one: text may
// then be read twice, but none is skipped.
function logScript({ path }, { offset, mark }) {
  const saved = savedMark(mark);
  return `export LC_ALL=C
file="${path}"
if [ ! -r "$file" ]; then echo exists=0; exit 0; fi
echo exists=1
inode=$(ls -idL "$file" | { read -r id rest; echo "$id"; })
mark() {
  length=$1
  if [ "$length" -gt ${LOG_MARK_BYTES} ]; then length=${LOG_MARK_BYTES}; fi
  if [ "$length" -gt 0 ]; then tail -c +$(($1 - length + 1)) "$file" | head -c "$length"; fi | cksum | { read -r crc rest; echo "$inode-$crc"; }
}
size=$(wc -c < "$file" | tr -d " ")
echo size=$size
start=${offset ?? "$size"}
if [ "$start" -gt "$size" ]${saved ? ` || [ "$(mark "$start")" != "${saved}" ]` : ""}; then start=0; fi
count=$((size - start))
if [ "$count" -gt ${MAX_LOG_BYTES} ]; then count=${MAX_LOG_BYTES}; fi
next=$((start + count))
echo offset=$start
echo next=$next
echo mark=$(mark "$next")
echo @@text
if [ "$count" -gt 0 ]; then tail -c +$((start + 1)) "$file" | head -c "$count"; fi
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

const UTF8_OUTPUT = "$ErrorActionPreference = 'SilentlyContinue'\n[Console]::OutputEncoding = [Text.Encoding]::UTF8\n";

function windowsProcessScript(target) {
  const find = target.pid !== undefined ? `Get-Process -Id ${target.pid}` : `Get-Process -Name '${target.name.replace(/\.exe$/i, "")}' | Select-Object -First 1`;
  return `${UTF8_OUTPUT}$p = ${find}
if (-not $p) { 'exists=0'; exit 0 }
'exists=1'
"pid=$($p.Id)"
"cpuSeconds=$([math]::Round($p.CPU, 3))"
"memoryKb=$([int64]($p.WorkingSet64 / 1024))"
"command=$($p.ProcessName)"
"threads=$($p.Threads.Count)"
`;
}

function windowsThreadScript({ pid, tid }) {
  return `${UTF8_OUTPUT}$t = (Get-Process -Id ${pid}).Threads | Where-Object { $_.Id -eq ${tid} }
if (-not $t) { 'exists=0'; exit 0 }
'exists=1'
"cpuSeconds=$([math]::Round($t.TotalProcessorTime.TotalSeconds, 3))"
`;
}

// Windows has no inode to read cheaply, and a file created where another was just renamed from
// inherits its creation time. So the mark is the hash of the bytes before the offset: a replacement
// only passes for the same file if those bytes match, and then reading on loses nothing unseen.
function windowsLogScript({ path }, { offset, mark }) {
  const saved = savedMark(mark);
  return `${UTF8_OUTPUT}$path = '${path}'
if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { 'exists=0'; exit 0 }
'exists=1'
$file = [IO.File]::Open($path, 'Open', 'Read', 'ReadWrite,Delete')
function Get-Mark($end) {
  $length = [Math]::Min(${LOG_MARK_BYTES}, $end)
  $window = New-Object byte[] $length
  [void]$file.Seek($end - $length, 'Begin')
  $got = $file.Read($window, 0, $length)
  [BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash($window, 0, $got))
}
$size = $file.Length
"size=$size"
$start = ${offset ?? "$size"}
if ($start -gt $size${saved ? ` -or (Get-Mark $start) -ne '${saved}'` : ""}) { $start = 0 }
[void]$file.Seek($start, 'Begin')
$bytes = New-Object byte[] ([Math]::Min(${MAX_LOG_BYTES}, $size - $start))
$read = $file.Read($bytes, 0, $bytes.Length)
$next = $start + $read
$mark = Get-Mark $next
$file.Close()
"offset=$start"
"next=$next"
"mark=$mark"
'@@text'
[Console]::Out.Write([Text.Encoding]::UTF8.GetString($bytes, 0, $read))
`;
}

export function probeScript(target, state = {}, platform = "linux") {
  if (platform === "win32") return { process: windowsProcessScript, thread: windowsThreadScript, log: windowsLogScript }[target.kind](target, state);
  const script = { process: processScript, thread: threadScript, log: logScript, sharedMemory: regionScript }[target.kind](target, state);
  if (script.includes("'")) throw new Error("Probe scripts must not contain single quotes.");
  return script;
}

// Windows reports CPU as cumulative seconds, so usage is the rate between two samples, in the
// same units as ps: 100 means one full core.
export function withCpuRate(sample, previous) {
  if (sample.cpuSeconds === undefined) return sample;
  const elapsed = previous ? (sample.at - previous.at) / 1000 : 0;
  if (!(elapsed > 0) || previous.cpuSeconds === undefined || previous.pid !== sample.pid) return sample;
  return { ...sample, cpu: Math.max(0, (sample.cpuSeconds - previous.cpuSeconds) / elapsed * 100) };
}

function readField(buffer, { offset, type }) {
  if (offset + FIELD_TYPES[type] > buffer.length) return null;
  const readers = { u8: "readUInt8", i8: "readInt8", u16: "readUInt16LE", i16: "readInt16LE", u32: "readUInt32LE", i32: "readInt32LE", f32: "readFloatLE", f64: "readDoubleLE" };
  if (type === "u64" || type === "i64") return Number(type === "u64" ? buffer.readBigUInt64LE(offset) : buffer.readBigInt64LE(offset));
  return buffer[readers[type]](offset);
}

export function parseProbe(target, output) {
  const separator = output.search(/^@@(text|bytes)\r?$/m);
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
      // Counted by the probe in raw bytes: the text here may have been decoded lossily.
      sample.nextOffset = sample.next ?? (sample.offset ?? 0) + Buffer.byteLength(body);
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

const MAX_PROCESSES = 5000;

export function processListScript(platform = "linux") {
  if (platform !== "win32") return "export LC_ALL=C\nps -A -o pid=,ppid=,pcpu=,rss=,comm=\n";
  return `${UTF8_OUTPUT}Get-CimInstance Win32_Process | ForEach-Object { "{0}\`t{1}\`t{2}\`t{3}\`t{4}" -f $_.ProcessId, $_.ParentProcessId, ([int64]$_.KernelModeTime + [int64]$_.UserModeTime), ([int64]($_.WorkingSetSize / 1024)), $_.Name }
`;
}

export function parseProcessList(output, platform = "linux") {
  const processes = [];
  for (const line of output.split("\n")) {
    if (platform === "win32") {
      const [pid, ppid, ticks, memoryKb, ...name] = line.trimEnd().split("\t");
      if (!name.length || !/^\d+$/.test(pid)) continue;
      processes.push({ pid: Number(pid), ppid: Number(ppid), cpuTicks: Number(ticks), cpu: 0, memoryKb: Number(memoryKb), command: name.join(" ").slice(0, 200) });
    } else {
      const match = /^\s*(\d+)\s+(\d+)\s+([\d.]+)\s+(\d+)\s+(.+?)\s*$/.exec(line);
      if (!match) continue;
      processes.push({ pid: Number(match[1]), ppid: Number(match[2]), cpu: Number(match[3]), memoryKb: Number(match[4]), command: match[5].slice(0, 200) });
    }
    if (processes.length >= MAX_PROCESSES) break;
  }
  return processes;
}

const TICKS_PER_SECOND = 10_000_000;

// `previous` is the last list and when it was taken. The first list has no rate to show.
export function withProcessRates(processes, previous, at) {
  const ticks = new Map((previous?.processes ?? []).map(({ pid, cpuTicks }) => [pid, cpuTicks]));
  const elapsed = previous ? (at - previous.at) / 1000 : 0;
  return processes.map((process) => {
    const before = ticks.get(process.pid);
    if (process.cpuTicks === undefined || before === undefined || !(elapsed > 0)) return process;
    return { ...process, cpu: Math.max(0, (process.cpuTicks - before) / TICKS_PER_SECOND / elapsed * 100) };
  });
}

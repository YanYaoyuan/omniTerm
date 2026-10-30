// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
// Host statistics come from one read-only POSIX sh loop on the remote host, so
// nothing is installed there. The script stays free of single quotes because it
// is passed as `sh -c '<script>'` to whatever login shell the account uses.
export const SAMPLE_SECONDS = 3;

export const METRICS_SCRIPT = `export LC_ALL=C
os=$(uname -s)
while :; do
echo "@@begin $os"
case $os in
Linux)
echo @@cpu; head -n 1 /proc/stat
echo @@cores; grep -c "^cpu[0-9]" /proc/stat
echo @@mem; grep -E "^(MemTotal|MemAvailable|SwapTotal|SwapFree):" /proc/meminfo
echo @@load; cat /proc/loadavg
echo @@uptime; cat /proc/uptime
echo @@net; tail -n +3 /proc/net/dev
;;
Darwin)
echo @@cpupercent; top -l 1 -n 0 -s 0 | grep "CPU usage"
echo @@cores; sysctl -n hw.ncpu
echo @@memdarwin; sysctl -n hw.memsize hw.pagesize; vm_stat
echo @@load; sysctl -n vm.loadavg | tr -d "{}"
;;
esac
echo @@disk; df -Pk / 2>/dev/null | tail -n 1
echo @@nvidia
if command -v nvidia-smi >/dev/null 2>&1; then nvidia-smi --query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu --format=csv,noheader,nounits 2>/dev/null; fi
echo @@amdgpu
for card in /sys/class/drm/card[0-9]*/device; do
if [ -r "$card/gpu_busy_percent" ]; then
echo "$(cat "$card/gpu_busy_percent" 2>/dev/null) $(cat "$card/mem_info_vram_used" 2>/dev/null) $(cat "$card/mem_info_vram_total" 2>/dev/null)"
fi
done
echo @@end
sleep ${SAMPLE_SECONDS}
done
`;

const number = (value) => {
  const result = Number(value);
  return Number.isFinite(result) && result >= 0 ? result : null;
};
const positive = (value) => { const result = number(value); return result === null ? null : result; };

function parseMeminfo(lines) {
  const values = {};
  for (const line of lines) {
    const match = /^(\w+):\s+(\d+)/.exec(line);
    if (match) values[match[1]] = Number(match[2]) * 1024;
  }
  const result = {};
  if (values.MemTotal && values.MemAvailable !== undefined) result.mem = { total: values.MemTotal, used: Math.max(0, values.MemTotal - values.MemAvailable) };
  if (values.SwapTotal) result.swap = { total: values.SwapTotal, used: Math.max(0, values.SwapTotal - (values.SwapFree ?? 0)) };
  return result;
}

function parseDarwinMemory(lines) {
  const [total, pageSize] = [number(lines[0]), number(lines[1])];
  if (!total || !pageSize) return {};
  const pages = (label) => Number(new RegExp(`^${label}:\\s+(\\d+)`, "m").exec(lines.join("\n"))?.[1] ?? 0);
  return { mem: { total, used: Math.min(total, (pages("Pages active") + pages("Pages wired down") + pages("Pages occupied by compressor")) * pageSize) } };
}

function parseNet(lines) {
  let rx = 0, tx = 0, seen = false;
  for (const line of lines) {
    const separator = line.indexOf(":");
    if (separator < 0 || line.slice(0, separator).trim() === "lo") continue;
    const fields = line.slice(separator + 1).trim().split(/\s+/);
    const [received, sent] = [number(fields[0]), number(fields[8])];
    if (received === null || sent === null) continue;
    rx += received; tx += sent; seen = true;
  }
  return seen ? { rx, tx } : null;
}

function parseGpus(nvidia, amd) {
  const gpus = [];
  for (const line of nvidia) {
    const [name, util, memUsed, memTotal, temp] = line.split(",").map((field) => field.trim());
    if (!name || number(util) === null) continue;
    gpus.push({ name: name.slice(0, 80), util: number(util), memUsed: positive(memUsed) === null ? null : positive(memUsed) * 1048576, memTotal: positive(memTotal) === null ? null : positive(memTotal) * 1048576, temp: number(temp) });
  }
  amd.forEach((line, index) => {
    const [util, memUsed, memTotal] = line.trim().split(/\s+/).map(number);
    if (util !== null && util !== undefined) gpus.push({ name: `AMD GPU ${index}`, util, memUsed: memUsed ?? null, memTotal: memTotal ?? null, temp: null });
  });
  return gpus.slice(0, 8);
}

// Parses the text of one @@begin…@@end block. Remote output is untrusted: every
// value is checked, and anything unrecognized is simply left out of the result.
export function parseSample(text) {
  const sections = new Map();
  let current = null;
  let os = "";
  for (const line of text.split("\n")) {
    const marker = /^@@(\w+)(?: (.*))?$/.exec(line);
    if (marker) {
      if (marker[1] === "begin") os = (marker[2] || "").slice(0, 40);
      else { current = []; sections.set(marker[1], current); }
    } else if (current && line) current.push(line);
  }
  const sample = { os, gpus: parseGpus(sections.get("nvidia") ?? [], sections.get("amdgpu") ?? []) };
  const cpu = sections.get("cpu")?.[0]?.trim().split(/\s+/).slice(1).map(number);
  if (cpu?.length >= 4 && cpu.every((value) => value !== null)) sample.cpuTicks = { total: cpu.slice(0, 8).reduce((sum, value) => sum + value, 0), idle: cpu[3] + (cpu[4] ?? 0) };
  const idle = /([\d.]+)% idle/.exec(sections.get("cpupercent")?.[0] ?? "");
  if (idle) sample.cpu = Math.min(100, Math.max(0, 100 - Number(idle[1])));
  const cores = number(sections.get("cores")?.[0]);
  if (cores) sample.cores = cores;
  Object.assign(sample, parseMeminfo(sections.get("mem") ?? []), parseDarwinMemory(sections.get("memdarwin") ?? []));
  const load = sections.get("load")?.[0]?.trim().split(/\s+/).slice(0, 3).map(number);
  if (load?.length === 3 && load.every((value) => value !== null)) sample.load = load;
  const uptime = number(sections.get("uptime")?.[0]?.split(/\s+/)[0]);
  if (uptime !== null) sample.uptime = uptime;
  const net = parseNet(sections.get("net") ?? []);
  if (net) sample.net = net;
  const disk = sections.get("disk")?.[0]?.trim().split(/\s+/);
  if (disk && disk.length >= 6) {
    const [total, used] = [number(disk[1]), number(disk[2])];
    if (total && used !== null) sample.disk = { total: total * 1024, used: used * 1024 };
  }
  return sample;
}

// CPU and network figures are counters, so each report compares two samples.
export class MetricsTracker {
  constructor(now = Date.now) { this.now = now; this.previous = null; }
  next(sample) {
    const time = this.now();
    const { cpuTicks, net, ...report } = sample;
    const previous = this.previous;
    if (cpuTicks && previous?.cpuTicks) {
      const total = cpuTicks.total - previous.cpuTicks.total;
      const idle = cpuTicks.idle - previous.cpuTicks.idle;
      if (total > 0) report.cpu = Math.min(100, Math.max(0, (1 - idle / total) * 100));
    }
    if (net && previous?.net) {
      const seconds = (time - previous.time) / 1000;
      if (seconds > 0) report.net = { rx: Math.max(0, net.rx - previous.net.rx) / seconds, tx: Math.max(0, net.tx - previous.net.tx) / seconds };
    }
    this.previous = { cpuTicks, net, time };
    return report;
  }
}

// Splits streamed stdout into complete samples and keeps the unfinished tail.
export function takeSamples(buffer) {
  const samples = [];
  let rest = buffer;
  for (;;) {
    const end = rest.indexOf("@@end\n");
    if (end < 0) return { samples, rest };
    samples.push(rest.slice(0, end));
    rest = rest.slice(end + 6);
  }
}

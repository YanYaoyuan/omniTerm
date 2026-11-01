// Pure helpers for the process window, kept free of JSX so tests can import them.

const matches = (process, query) => process.command.toLowerCase().includes(query) || String(process.pid).includes(query);

function treeRows(processes, keep) {
  const byPid = new Map(processes.map((process) => [process.pid, process]));
  const children = new Map();
  const roots = [];
  for (const process of processes) {
    if (process.ppid !== process.pid && byPid.has(process.ppid)) children.set(process.ppid, [...(children.get(process.ppid) ?? []), process]);
    else roots.push(process);
  }
  const rows = [];
  const seen = new Set();
  const visit = (process, depth) => {
    if (seen.has(process.pid)) return;
    seen.add(process.pid);
    if (keep(process)) rows.push({ ...process, depth });
    const kids = (children.get(process.pid) ?? []).toSorted((a, b) => a.command.localeCompare(b.command));
    for (const child of kids) visit(child, depth + 1);
  };
  for (const root of roots.toSorted((a, b) => a.pid - b.pid)) visit(root, 0);
  // Parents reported in a loop have no root; they still have to appear.
  for (const process of processes) visit(process, 0);
  return rows;
}

// A tree by default. Sorting by a column flattens it, since a tree has no meaningful order then.
export function processRows(processes, { query = "", sort = null } = {}) {
  const needle = query.trim().toLowerCase();
  if (sort) {
    const list = needle ? processes.filter((process) => matches(process, needle)) : processes;
    return list.map((process) => ({ ...process, depth: 0 })).toSorted((a, b) => b[sort] - a[sort]);
  }
  if (!needle) return treeRows(processes, () => true);
  const byPid = new Map(processes.map((process) => [process.pid, process]));
  const shown = new Set();
  for (const process of processes) {
    if (!matches(process, needle)) continue;
    for (let current = process; current && !shown.has(current.pid); current = byPid.get(current.ppid)) shown.add(current.pid);
  }
  return treeRows(processes, (process) => shown.has(process.pid));
}

export function pushTrends(trends, processes, limit = 60) {
  const next = new Map();
  for (const { pid, cpu, memoryKb } of processes) next.set(pid, [...(trends.get(pid) ?? []), { cpu, memoryKb }].slice(-limit));
  return next;
}

export const summarize = (processes) => ({
  count: processes.length,
  cpu: Math.round(processes.reduce((total, { cpu }) => total + cpu, 0)),
  memoryKb: processes.reduce((total, { memoryKb }) => total + memoryKb, 0),
});

export function monitorRequest(process, host) {
  return { host: host === "local" ? "local" : { sessionId: host }, label: `${process.command} (${process.pid})`, target: { kind: "process", pid: process.pid }, conditions: [{ op: "gone" }], intervalSeconds: 5, durationMinutes: 60 };
}

export const formatMemory = (kilobytes) => kilobytes >= 1048576 ? `${(kilobytes / 1048576).toFixed(1)} GB` : kilobytes >= 1024 ? `${(kilobytes / 1024).toFixed(0)} MB` : `${kilobytes} KB`;

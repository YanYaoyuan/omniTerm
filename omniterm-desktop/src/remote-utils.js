// Pure helpers for the SFTP panel and host metrics, kept free of JSX for tests.

const UNITS = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];

export function formatBytes(value) {
  if (!Number.isFinite(value) || value < 0) return "—";
  let size = value;
  let unit = 0;
  while (size >= 1024 && unit < UNITS.length - 1) { size /= 1024; unit++; }
  return `${unit === 0 || size >= 100 ? Math.round(size) : size.toFixed(1)} ${UNITS[unit]}`;
}

export function formatUptime(seconds) {
  if (!Number.isFinite(seconds)) return "—";
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor(seconds / 3600) % 24;
  const minutes = Math.floor(seconds / 60) % 60;
  return days ? `${days}天 ${hours}时` : hours ? `${hours}时 ${minutes}分` : `${minutes}分`;
}

export function formatTime(seconds) {
  if (!seconds) return "";
  const date = new Date(seconds * 1000);
  const pad = (value) => String(value).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

export const percent = (used, total) => total > 0 ? Math.min(100, Math.max(0, used / total * 100)) : null;

export function joinRemote(directory, name) {
  return directory.endsWith("/") ? `${directory}${name}` : `${directory}/${name}`;
}

export function parentRemote(directory) {
  const trimmed = directory.replace(/\/+$/, "");
  const index = trimmed.lastIndexOf("/");
  return index <= 0 ? "/" : trimmed.slice(0, index);
}

// Folders first, then names in a natural order so file10 follows file9.
export function sortEntries(entries, showHidden) {
  return entries
    .filter((entry) => showHidden || !entry.name.startsWith("."))
    .sort((a, b) => Number(b.directory) - Number(a.directory) || a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" }));
}

// A rename target is one path segment; `a/b` would silently move the item.
export const validEntryName = (name) => Boolean(name) && name !== "." && name !== ".." && !/[/\0]/.test(name);

// Applies a click to the selection: plain replaces, Ctrl/Cmd toggles, Shift extends.
export function nextSelection(selection, names, name, { toggle = false, range = false } = {}, anchor = null) {
  if (range && anchor !== null && names.includes(anchor)) {
    const [from, to] = [names.indexOf(anchor), names.indexOf(name)].sort((a, b) => a - b);
    return new Set(names.slice(from, to + 1));
  }
  if (!toggle) return new Set([name]);
  const next = new Set(selection);
  if (!next.delete(name)) next.add(name);
  return next;
}

const FINISHED = ["done", "failed", "canceled"];
export const transferFinished = (transfer) => FINISHED.includes(transfer.status);
const MAX_FINISHED_TRANSFERS = 20;

// Connection, transfer and metric events arrive far more often than the rest of
// the app changes. They live here so only the panels showing them re-render.
export function createRemoteStore(api) {
  const states = new Map();
  const transfers = new Map();
  const metrics = new Map();
  const listeners = new Set();
  const emit = () => { for (const listener of listeners) listener(); };
  const none = [];
  api.onRemoteState((state) => {
    if (state.state === "closed") { states.delete(state.id); transfers.delete(state.id); metrics.delete(state.id); }
    else states.set(state.id, state);
    emit();
  });
  api.onRemoteTransfer((transfer) => {
    const list = transfers.get(transfer.id) ?? none;
    const index = list.findIndex((item) => item.transferId === transfer.transferId);
    const next = index < 0 ? [...list, transfer] : list.map((item, position) => position === index ? transfer : item);
    const finished = next.filter(transferFinished);
    const stale = new Set(finished.slice(0, Math.max(0, finished.length - MAX_FINISHED_TRANSFERS)));
    transfers.set(transfer.id, next.filter((item) => !stale.has(item)));
    emit();
  });
  api.onRemoteMetrics(({ id, available, sample }) => {
    metrics.set(id, available ? { available, sample } : { available, sample: null });
    emit();
  });
  return {
    subscribe: (listener) => { listeners.add(listener); return () => listeners.delete(listener); },
    state: (id) => states.get(id) ?? null,
    transfers: (id) => transfers.get(id) ?? none,
    metrics: (id) => metrics.get(id) ?? null,
    clearFinished: (id) => { transfers.set(id, (transfers.get(id) ?? none).filter((item) => !transferFinished(item))); emit(); },
  };
}

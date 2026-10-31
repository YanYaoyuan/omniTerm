// Workspace snapshots let the next launch reopen the same local tabs and split
// layout. Only the interface is restored: each pane becomes a fresh shell in its
// last known directory. Commands are never re-executed, and remote, serial and
// tunnel panes are dropped rather than reconnected without the user asking.

import { MIN_RATIO } from "./layout.js";

export const WORKSPACE_KEY = "omniterm-workspace";
export const WORKSPACE_VERSION = 1;
// The terminal host allows 20 sessions; leave room for new tabs.
export const MAX_RESTORED_PANES = 16;
// A binary tree of n panes is at most n - 1 deep, so this never truncates a
// layout the pane budget would otherwise keep.
const MAX_DEPTH = MAX_RESTORED_PANES;
const MAX_TITLE = 60;
const MAX_PATH = 4096;

function snapshotNode(node, cwdOf) {
  if (!node) return null;
  if (node.type === "pane") {
    const cwd = cwdOf(node.sessionId);
    return cwd === null ? null : { type: "pane", cwd: cwd || "" };
  }
  const [first, second] = node.children.map((child) => snapshotNode(child, cwdOf));
  if (first && second) return { type: "split", direction: node.direction, ratio: node.ratio, children: [first, second] };
  return first || second || null;
}

// tabs: App tab state; sessions: session descriptors with kind/cwd/liveCwd.
export function serializeWorkspace(tabs, sessions, activeTabId) {
  const byId = new Map(sessions.map((session) => [session.id, session]));
  const cwdOf = (id) => {
    const session = byId.get(id);
    return session?.kind === "shell" ? session.liveCwd || session.cwd || "" : null;
  };
  const saved = [];
  let active = 0;
  for (const tab of tabs) {
    const root = snapshotNode(tab.root, cwdOf);
    if (!root) continue;
    if (tab.id === activeTabId) active = saved.length;
    saved.push({ title: tab.title || "", root });
  }
  return { version: WORKSPACE_VERSION, active, tabs: saved };
}

// Validates untrusted stored data; returns null when nothing usable remains.
export function parseWorkspace(raw) {
  let data;
  try { data = typeof raw === "string" ? JSON.parse(raw) : raw; } catch { return null; }
  if (!data || data.version !== WORKSPACE_VERSION || !Array.isArray(data.tabs)) return null;
  let budget = MAX_RESTORED_PANES;
  const node = (value, depth) => {
    if (!value || typeof value !== "object" || depth > MAX_DEPTH) return null;
    if (value.type === "pane") {
      if (budget <= 0) return null;
      budget--;
      return { type: "pane", cwd: typeof value.cwd === "string" && value.cwd.length <= MAX_PATH ? value.cwd : "" };
    }
    if (value.type !== "split" || !["row", "column"].includes(value.direction) || !Array.isArray(value.children) || value.children.length !== 2) return null;
    const [first, second] = value.children.map((child) => node(child, depth + 1));
    if (!first || !second) return first || second || null;
    const ratio = Number.isFinite(value.ratio) ? Math.min(1 - MIN_RATIO, Math.max(MIN_RATIO, value.ratio)) : 0.5;
    return { type: "split", direction: value.direction, ratio, children: [first, second] };
  };
  const tabs = [];
  for (const tab of data.tabs) {
    const root = node(tab?.root, 0);
    if (!root) continue;
    tabs.push({ title: typeof tab.title === "string" ? tab.title.slice(0, MAX_TITLE) : "", root });
  }
  if (!tabs.length) return null;
  const active = Number.isInteger(data.active) && data.active >= 0 && data.active < tabs.length ? data.active : 0;
  return { version: WORKSPACE_VERSION, active, tabs };
}

// Replaces each saved pane with a live session, left to right. openPane(cwd)
// resolves to a session id or null; panes that fail to open collapse away.
export async function restoreTree(node, openPane) {
  if (node.type === "pane") {
    const id = await openPane(node.cwd);
    return id ? { type: "pane", sessionId: id } : null;
  }
  const first = await restoreTree(node.children[0], openPane);
  const second = await restoreTree(node.children[1], openPane);
  if (first && second) return { type: "split", direction: node.direction, ratio: node.ratio, children: [first, second] };
  return first || second || null;
}

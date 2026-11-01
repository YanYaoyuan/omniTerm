// Pane layout trees for split terminals. A node is either
//   { type: "pane", sessionId }
//   { type: "split", direction: "row" | "column", ratio, children: [a, b] }
// "row" places children side by side; "column" stacks them vertically.
// All helpers are pure so tab state can be updated immutably.

export const MIN_RATIO = 0.12;

export const pane = (sessionId) => ({ type: "pane", sessionId });

export function paneIds(node) {
  if (!node) return [];
  return node.type === "pane" ? [node.sessionId] : node.children.flatMap(paneIds);
}

export function splitPane(node, target, sessionId, direction) {
  if (node.type === "pane") {
    return node.sessionId === target ? { type: "split", direction, ratio: 0.5, children: [node, pane(sessionId)] } : node;
  }
  return { ...node, children: node.children.map((child) => splitPane(child, target, sessionId, direction)) };
}

const SIDES = {
  left: { direction: "row", before: true },
  right: { direction: "row", before: false },
  top: { direction: "column", before: true },
  bottom: { direction: "column", before: false },
};

// Places `sessionId` on one side of `target`, splitting the target in half.
export function insertPane(node, target, sessionId, side) {
  if (node.type === "pane") {
    if (node.sessionId !== target) return node;
    const { direction, before } = SIDES[side];
    return { type: "split", direction, ratio: 0.5, children: before ? [pane(sessionId), node] : [node, pane(sessionId)] };
  }
  return { ...node, children: node.children.map((child) => insertPane(child, target, sessionId, side)) };
}

// The edge of a pane nearest to a pointer, as fractions of that pane.
export function dropSide(x, y) {
  const distances = { left: x, right: 1 - x, top: y, bottom: 1 - y };
  return Object.keys(distances).reduce((best, side) => distances[side] < distances[best] ? side : best);
}

// Moves a pane within or between tabs. Only layout changes: sessions and their terminal views
// are keyed by session ID, so a moved pane keeps its live session. A source tab left with no
// panes goes away. `tabId` omitted means a new tab, from `createTab`.
export function movePaneInTabs(tabs, sessionId, { tabId, target = null, side = "right" } = {}, createTab = null) {
  const source = tabs.find((tab) => paneIds(tab.root).includes(sessionId));
  const destination = tabId ? tabs.find((tab) => tab.id === tabId) : createTab?.(sessionId);
  if (!source || !destination || target === sessionId) return tabs;
  const sameTab = destination.id === source.id;
  const remaining = removePane(source.root, sessionId);
  const base = sameTab ? remaining : destination.root;
  if (!base && sameTab) return tabs;
  const anchor = target ?? (base ? (destination.focus !== sessionId && paneIds(base).includes(destination.focus) ? destination.focus : paneIds(base).at(-1)) : null);
  if (base && !paneIds(base).includes(anchor)) return tabs;
  if (!tabId && !remaining) return tabs;
  const root = base ? insertPane(base, anchor, sessionId, side) : pane(sessionId);
  const moved = { ...destination, root, focus: sessionId, zoomed: null };
  const next = tabs.flatMap((tab) => {
    if (tab.id === moved.id) return [moved];
    if (tab.id !== source.id) return [tab];
    if (!remaining) return [];
    const ids = paneIds(remaining);
    return [{ ...tab, root: remaining, focus: ids.includes(tab.focus) ? tab.focus : ids[0], zoomed: tab.zoomed === sessionId ? null : tab.zoomed }];
  });
  return tabId ? next : [...next, moved];
}

// Panels share the pane tree, with the panel ID in place of a session ID. New panels join here.
export const DOCK_PANELS = Object.freeze(["sidebar", "terminals", "agent"]);
export const DOCK_KEY = "omniterm-dock-layout";

export const defaultDock = () => ({
  type: "split", direction: "row", ratio: 0.18,
  children: [pane("sidebar"), { type: "split", direction: "row", ratio: 0.72, children: [pane("terminals"), pane("agent")] }],
});

export function moveDockPanel(root, id, target, side) {
  if (id === target) return root;
  const remaining = removePane(root, id);
  return remaining && paneIds(remaining).includes(target) ? insertPane(remaining, target, id, side) : root;
}

// The layout to draw with hidden panels left out. Splits remember their place in the full tree,
// so resizing a divider changes the saved layout even while a panel is hidden.
export function visibleDock(node, hidden = [], path = []) {
  if (node.type === "pane") return hidden.includes(node.sessionId) ? null : node;
  const [first, second] = node.children.map((child, index) => visibleDock(child, hidden, [...path, index]));
  return first && second ? { ...node, origin: path, children: [first, second] } : first || second;
}

// A stored layout is used only if it holds every panel exactly once, so a layout saved by a
// build with other panels falls back to the default instead of losing one.
export function parseDock(raw, panels = DOCK_PANELS) {
  let data;
  try { data = typeof raw === "string" ? JSON.parse(raw) : raw; } catch { return null; }
  const clean = (value, depth) => {
    if (!value || typeof value !== "object" || depth > 8) return null;
    if (value.type === "pane") return panels.includes(value.sessionId) ? pane(value.sessionId) : null;
    if (value.type !== "split" || !["row", "column"].includes(value.direction) || !Array.isArray(value.children) || value.children.length !== 2) return null;
    const [first, second] = value.children.map((child) => clean(child, depth + 1));
    if (!first || !second) return null;
    const ratio = Number.isFinite(value.ratio) ? Math.min(1 - MIN_RATIO, Math.max(MIN_RATIO, value.ratio)) : 0.5;
    return { type: "split", direction: value.direction, ratio, children: [first, second] };
  };
  const root = clean(data, 0);
  if (!root) return null;
  const ids = paneIds(root);
  return ids.length === panels.length && panels.every((panel) => ids.includes(panel)) ? root : null;
}

export function removePane(node, target) {
  if (!node) return null;
  if (node.type === "pane") return node.sessionId === target ? null : node;
  const [first, second] = node.children.map((child) => removePane(child, target));
  if (first && second) return { ...node, children: [first, second] };
  return first || second || null;
}

export function setRatio(node, path, ratio) {
  if (!path.length) return { ...node, ratio: Math.min(1 - MIN_RATIO, Math.max(MIN_RATIO, ratio)) };
  const [index, ...rest] = path;
  return { ...node, children: node.children.map((child, position) => position === index ? setRatio(child, rest, ratio) : child) };
}

export function equalize(node) {
  if (node.type === "pane") return node;
  return { ...node, ratio: 0.5, children: node.children.map(equalize) };
}

// Returns fractional rectangles (0–1) for every pane and every divider.
export function computeLayout(node, rect = { x: 0, y: 0, w: 1, h: 1 }, path = [], out = { panes: [], dividers: [] }) {
  if (!node) return out;
  if (node.type === "pane") { out.panes.push({ sessionId: node.sessionId, rect }); return out; }
  const row = node.direction === "row";
  const size = (row ? rect.w : rect.h) * node.ratio;
  const first = row ? { ...rect, w: size } : { ...rect, h: size };
  const second = row ? { x: rect.x + size, y: rect.y, w: rect.w - size, h: rect.h } : { x: rect.x, y: rect.y + size, w: rect.w, h: rect.h - size };
  out.dividers.push({ path: node.origin ?? path, direction: node.direction, parent: rect, position: row ? rect.x + size : rect.y + size });
  computeLayout(node.children[0], first, [...path, 0], out);
  computeLayout(node.children[1], second, [...path, 1], out);
  return out;
}

// Finds the closest pane in a direction ("left", "right", "up", "down").
export function neighborPane(node, fromId, direction) {
  const { panes } = computeLayout(node);
  const from = panes.find((item) => item.sessionId === fromId)?.rect;
  if (!from) return null;
  const epsilon = 1e-6;
  const center = (rect) => ({ x: rect.x + rect.w / 2, y: rect.y + rect.h / 2 });
  const origin = center(from);
  let best = null;
  for (const item of panes) {
    if (item.sessionId === fromId) continue;
    const { rect } = item;
    const ahead = {
      left: rect.x + rect.w <= from.x + epsilon,
      right: rect.x >= from.x + from.w - epsilon,
      up: rect.y + rect.h <= from.y + epsilon,
      down: rect.y >= from.y + from.h - epsilon,
    }[direction];
    if (!ahead) continue;
    const horizontal = direction === "left" || direction === "right";
    const overlaps = horizontal ? rect.y < from.y + from.h - epsilon && rect.y + rect.h > from.y + epsilon : rect.x < from.x + from.w - epsilon && rect.x + rect.w > from.x + epsilon;
    const target = center(rect);
    const distance = Math.hypot(target.x - origin.x, target.y - origin.y) + (overlaps ? 0 : 10);
    if (!best || distance < best.distance) best = { sessionId: item.sessionId, distance };
  }
  return best?.sessionId ?? null;
}

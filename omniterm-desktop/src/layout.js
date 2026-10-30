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
  out.dividers.push({ path, direction: node.direction, parent: rect, position: row ? rect.x + size : rect.y + size });
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

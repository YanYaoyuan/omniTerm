// Pure state for the Agent panel, kept free of JSX so tests can import it.

export const OPERATION_LABELS = {
  pendingApproval: "等待批准", approved: "已批准", queued: "排队中", running: "执行中", staged: "已输入，等待手动提交",
  completed: "已完成", denied: "已拒绝", cancelled: "已取消", invalidated: "已失效", blocked: "未发送", interrupted: "已中断",
  takenOver: "已由用户接管", outcomeUnknown: "结果未知",
};
export const INTERACTION_LABELS = {
  ready: "就绪", busy: "忙碌", fullscreen: "全屏程序", password: "密码提示", unknown: "状态未知", exited: "已退出", unsupported: "不支持命令",
};
export const WIRE_LABELS = { responses: "Responses", chat: "Chat Completions", anthropic_messages: "Anthropic Messages" };
export const OPEN_OPERATIONS = new Set(["pendingApproval", "approved", "queued", "running"]);

export const emptyConversation = Object.freeze({ entries: [], running: false, retrying: false });

const append = (state, entry) => ({ ...state, entries: [...state.entries, entry] });
const replaceWhere = (state, match, update) => ({ ...state, entries: state.entries.map((entry) => match(entry) ? update(entry) : entry) });
const hasEntry = (state, match) => state.entries.some(match);

export function conversationReducer(state, event) {
  switch (event.type) {
    case "user": return { ...append(state, { kind: "user", id: event.id, text: event.text }), running: true, retrying: false };
    case "turn-started": return { ...state, running: true };
    case "text-delta": {
      const match = (entry) => entry.kind === "assistant" && entry.id === event.itemId;
      return hasEntry(state, match) ? replaceWhere(state, match, (entry) => ({ ...entry, text: entry.text + event.delta })) : append(state, { kind: "assistant", id: event.itemId, text: event.delta });
    }
    case "text-done": {
      const match = (entry) => entry.kind === "assistant" && entry.id === event.itemId;
      return hasEntry(state, match) ? replaceWhere(state, match, (entry) => ({ ...entry, text: event.text })) : append(state, { kind: "assistant", id: event.itemId, text: event.text });
    }
    case "reasoning-delta": {
      const match = (entry) => entry.kind === "reasoning" && entry.id === event.itemId;
      return hasEntry(state, match) ? replaceWhere(state, match, (entry) => ({ ...entry, text: entry.text + event.delta })) : append(state, { kind: "reasoning", id: event.itemId, text: event.delta });
    }
    case "tool": {
      const match = (entry) => entry.kind === "tool" && entry.id === event.itemId;
      const entry = { kind: "tool", id: event.itemId, tool: event.tool, status: event.status, success: event.success };
      return hasEntry(state, match) ? replaceWhere(state, match, () => entry) : append(state, entry);
    }
    case "context-shared": return append(state, { kind: "context", id: `context-${state.entries.length}`, terminals: event.terminals, unavailable: event.unavailable ?? [], provider: event.provider, model: event.model });
    case "operation": {
      const match = (entry) => entry.kind === "operation" && entry.id === event.operation.operationId;
      const entry = { kind: "operation", id: event.operation.operationId, operation: event.operation };
      return hasEntry(state, match) ? replaceWhere(state, match, () => entry) : append(state, entry);
    }
    case "retrying": return { ...state, retrying: true };
    case "error": return { ...append(state, { kind: "error", id: `error-${state.entries.length}`, text: event.message }), running: false, retrying: false };
    case "turn-completed": {
      const next = { ...state, running: false, retrying: false };
      return event.error ? append(next, { kind: "error", id: `error-${state.entries.length}`, text: event.error }) : next;
    }
    case "notice": return append(state, { kind: "notice", id: `notice-${state.entries.length}`, text: event.text });
    case "reset": return { ...state, running: false, retrying: false };
    case "clear": return emptyConversation;
    default: return state;
  }
}

export const terminalName = (terminal) => `${terminal.label} · ${terminal.shortId}`;

export function scopeSummary(terminals, ids) {
  const selected = terminals.filter((terminal) => ids.has(terminal.ref.sessionId));
  if (!selected.length) return "无";
  return selected.length <= 2 ? selected.map((terminal) => terminal.label).join("、") : `${selected.length} 个终端`;
}

// Refs come from the host, so each carries the connection epoch it was selected for.
export const refsFor = (terminals, ids) => terminals.filter((terminal) => ids.has(terminal.ref.sessionId)).map((terminal) => terminal.ref);

// An `@` being typed at the end of the text selects a terminal; returns its partial name.
export function mentionQuery(text) {
  const match = /(?:^|\s)@([^\s@]*)$/.exec(text);
  return match ? match[1] : null;
}

export function mentionMatches(terminals, query) {
  const needle = query.toLowerCase();
  return terminals.filter((terminal) => terminal.label.toLowerCase().includes(needle) || terminal.shortId.startsWith(needle)).slice(0, 8);
}

// Splits model text around known terminal IDs so citations can link to their terminal.
export function citationParts(text, terminals) {
  const ids = new Map(terminals.map((terminal) => [terminal.shortId, terminal]));
  const parts = [];
  let last = 0;
  for (const match of text.matchAll(/\b[0-9a-f]{8}\b/g)) {
    const terminal = ids.get(match[0]);
    if (!terminal) continue;
    if (match.index > last) parts.push(text.slice(last, match.index));
    parts.push({ terminal, text: match[0] });
    last = match.index + match[0].length;
  }
  if (last < text.length) parts.push(text.slice(last));
  return parts;
}

// Terminals that closed since selection must not linger in a scope shown as active.
export function pruneSelection(ids, terminals) {
  const live = new Set(terminals.map((terminal) => terminal.ref.sessionId));
  const kept = [...ids].filter((id) => live.has(id));
  return kept.length === ids.size ? ids : new Set(kept);
}

export const executable = (terminal) => ["shell", "ssh"].includes(terminal.kind) && terminal.status === "running";

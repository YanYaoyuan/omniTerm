// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import test from "node:test";
import { monitorRequest, processRows, pushTrends, summarize } from "./process/process-logic.js";
import { buildMonitorRequest, emptyMonitorForm, kindNote, monitorMetrics, monitorReducer, sparkline, targetSummary } from "./agent/monitor-logic.js";
import { citationParts, conversationReducer, emptyConversation, executable, mentionMatches, mentionQuery, operationReason, pruneSelection, refsFor, scopeSummary } from "./agent/agent-logic.js";
import assert from "node:assert/strict";
import { DOCK_PANELS, computeLayout, defaultDock, dockedTerminal, dockedTerminals, dropSide, equalize, insertPane, MIN_RATIO, moveDockPanel, movePaneInTabs, neighborPane, pane, paneIds, parseDock, pruneDock, removePane, setRatio, splitPane, storableDock, takePaneFromTabs, visibleDock } from "./layout.js";
import { isAppShortcut, matchShortcut, shortcutLabel, tabCycle, tabNumber } from "./shortcuts.js";
import { createRemoteStore, formatBytes, formatUptime, joinRemote, nextSelection, parentRemote, percent, sortEntries, validEntryName } from "./remote-utils.js";
import { cwdFromOsc7, fuzzyScore, isTerminalReply, quotePath } from "./terminal-utils.js";
import { THEMES, backdrop, contrast, terminalTheme, themeFor, themeTokens } from "./themes.js";
import { MAX_RESTORED_PANES, WORKSPACE_VERSION, parseWorkspace, restoreTree, serializeWorkspace } from "./workspace.js";

const key = (code, modifiers = {}) => ({ type: "keydown", code, key: "", metaKey: false, ctrlKey: false, shiftKey: false, altKey: false, isComposing: false, ...modifiers });

test("splitting and removing panes keeps a valid tree", () => {
  let root = pane("a");
  root = splitPane(root, "a", "b", "row");
  root = splitPane(root, "b", "c", "column");
  assert.deepEqual(paneIds(root), ["a", "b", "c"]);
  assert.deepEqual(paneIds(removePane(root, "b")), ["a", "c"]);
  assert.equal(removePane(pane("a"), "a"), null);
  assert.deepEqual(removePane(root, "missing"), root);
});

test("layout rectangles tile the area and neighbors follow geometry", () => {
  // a | (b / c)
  const root = splitPane(splitPane(pane("a"), "a", "b", "row"), "b", "c", "column");
  const { panes, dividers } = computeLayout(root);
  const area = panes.reduce((sum, item) => sum + item.rect.w * item.rect.h, 0);
  assert.ok(Math.abs(area - 1) < 1e-9);
  assert.equal(dividers.length, 2);
  assert.equal(neighborPane(root, "a", "right"), "b");
  assert.equal(neighborPane(root, "c", "left"), "a");
  assert.equal(neighborPane(root, "b", "down"), "c");
  assert.equal(neighborPane(root, "c", "up"), "b");
  assert.equal(neighborPane(root, "a", "left"), null);
});

test("ratios are clamped and can be equalized", () => {
  const root = splitPane(pane("a"), "a", "b", "row");
  assert.equal(setRatio(root, [], 0).ratio, MIN_RATIO);
  assert.equal(setRatio(root, [], 2).ratio, 1 - MIN_RATIO);
  assert.equal(equalize(setRatio(root, [], 0.3)).ratio, 0.5);
});

test("non-mac shortcuts require Ctrl+Shift so shell control keys pass through", () => {
  for (const code of ["KeyW", "KeyD", "KeyF", "KeyK", "KeyT", "KeyA", "KeyP"]) {
    assert.equal(matchShortcut(key(code, { ctrlKey: true }), "linux"), null, code);
    assert.equal(isAppShortcut(key(code, { ctrlKey: true }), "linux"), false, code);
  }
  assert.equal(matchShortcut(key("KeyT", { ctrlKey: true, shiftKey: true }), "linux"), "newTab");
  assert.equal(matchShortcut(key("KeyE", { ctrlKey: true, shiftKey: true }), "win32"), "splitRight");
  assert.equal(matchShortcut(key("Equal", { ctrlKey: true }), "linux"), "fontBigger");
  assert.equal(matchShortcut(key("Equal", { ctrlKey: true, shiftKey: true }), "linux"), "fontBigger");
  assert.equal(matchShortcut(key("ArrowLeft", { ctrlKey: true, altKey: true }), "linux"), "paneLeft");
  assert.equal(matchShortcut(key("KeyC", { ctrlKey: true, shiftKey: true }), "linux"), null);
});

test("mac shortcuts use Command and distinguish Shift", () => {
  assert.equal(matchShortcut(key("KeyD", { metaKey: true }), "darwin"), "splitRight");
  assert.equal(matchShortcut(key("KeyD", { metaKey: true, shiftKey: true }), "darwin"), "splitDown");
  assert.equal(matchShortcut(key("KeyI", { metaKey: true, altKey: true }), "darwin"), "broadcast");
  assert.equal(matchShortcut(key("KeyD", { ctrlKey: true }), "darwin"), null);
  assert.equal(matchShortcut(key("KeyT", { metaKey: true, isComposing: true }), "darwin"), null);
  assert.equal(shortcutLabel("splitDown", "darwin"), "⇧⌘D");
  assert.equal(shortcutLabel("newTab", "linux"), "Ctrl+Shift+T");
  assert.equal(shortcutLabel("fontBigger", "linux"), "Ctrl++");
  assert.equal(shortcutLabel("paneLeft", "win32"), "Ctrl+Alt+←");
});

test("tab cycling and numbering", () => {
  assert.equal(tabCycle(key("Tab", { ctrlKey: true })), 1);
  assert.equal(tabCycle(key("Tab", { ctrlKey: true, shiftKey: true })), -1);
  assert.equal(tabCycle(key("Tab")), 0);
  assert.equal(tabNumber(key("Digit3", { metaKey: true }), "darwin"), 3);
  assert.equal(tabNumber(key("Digit3", { altKey: true }), "linux"), 3);
  assert.equal(tabNumber(key("Digit3", { ctrlKey: true }), "linux"), 0);
});

test("OSC 7 working directories and dropped paths", () => {
  assert.equal(cwdFromOsc7("file://host/Users/me/my%20dir", "darwin"), "/Users/me/my dir");
  assert.equal(cwdFromOsc7("file:///C:/Users/me", "win32"), "C:\\Users\\me");
  assert.equal(cwdFromOsc7("https://example.com/", "linux"), null);
  assert.equal(cwdFromOsc7("garbage", "linux"), null);
  assert.equal(quotePath("/tmp/plain-file.txt", "linux"), "/tmp/plain-file.txt");
  assert.equal(quotePath("/tmp/it's here", "linux"), "'/tmp/it'\\''s here'");
  assert.equal(quotePath("C:\\My Files\\a.txt", "win32"), '"C:\\My Files\\a.txt"');
});

test("fuzzy scoring prefers word starts and rejects non-matches", () => {
  assert.equal(fuzzyScore("xyz", "split right"), 0);
  assert.ok(fuzzyScore("sr", "split right") > 0);
  assert.ok(fuzzyScore("spl", "split right") > fuzzyScore("spl", "display all"));
});

test("workspace snapshots keep local shells, layout and live directories", () => {
  // a | (b / ssh)  plus a tab holding only a serial session
  const root = splitPane(splitPane(pane("a"), "a", "b", "row"), "b", "ssh", "column");
  const tabs = [
    { id: "t1", root: setRatio(root, [], 0.3), focus: "b", title: "build" },
    { id: "t2", root: pane("serial"), focus: "serial", title: "" },
    { id: "t3", root: pane("c"), focus: "c", title: "" },
  ];
  const sessions = [
    { id: "a", kind: "shell", cwd: "/home/u", liveCwd: "/home/u/project" },
    { id: "b", kind: "shell", cwd: "/tmp" },
    { id: "c", kind: "shell", cwd: "/srv" },
    { id: "ssh", kind: "ssh", cwd: "/home/u" },
    { id: "serial", kind: "serial", cwd: "/home/u" },
  ];
  const snapshot = serializeWorkspace(tabs, sessions, "t3");
  assert.equal(snapshot.active, 1);
  assert.equal(snapshot.tabs.length, 2);
  assert.equal(snapshot.tabs[0].title, "build");
  assert.deepEqual(snapshot.tabs[0].root, { type: "split", direction: "row", ratio: 0.3, children: [{ type: "pane", cwd: "/home/u/project" }, { type: "pane", cwd: "/tmp" }] });
  assert.deepEqual(parseWorkspace(JSON.stringify(snapshot)), snapshot);
});

test("workspace parsing rejects malformed data and caps restored panes", () => {
  assert.equal(parseWorkspace("not json"), null);
  assert.equal(parseWorkspace(JSON.stringify({ version: 99, tabs: [] })), null);
  assert.equal(parseWorkspace({ version: WORKSPACE_VERSION, tabs: [{ root: { type: "pane" } }] }).tabs[0].root.cwd, "");
  const bad = parseWorkspace({ version: WORKSPACE_VERSION, active: 7, tabs: [
    { title: 5, root: { type: "split", direction: "diagonal", ratio: 0.5, children: [] } },
    { title: "x".repeat(200), root: { type: "split", direction: "row", ratio: 5, children: [{ type: "pane", cwd: "/a" }, { type: "bogus" }] } },
    { root: { type: "split", direction: "column", ratio: -1, children: [{ type: "pane", cwd: "/b" }, { type: "pane", cwd: 3 }] } },
  ] });
  assert.equal(bad.active, 0);
  assert.equal(bad.tabs.length, 2);
  assert.equal(bad.tabs[0].title.length, 60);
  assert.deepEqual(bad.tabs[0].root, { type: "pane", cwd: "/a" });
  assert.equal(bad.tabs[1].root.ratio, MIN_RATIO);
  assert.equal(bad.tabs[1].root.children[1].cwd, "");
  const many = { version: WORKSPACE_VERSION, tabs: Array.from({ length: 30 }, () => ({ root: { type: "pane", cwd: "/" } })) };
  assert.equal(parseWorkspace(many).tabs.length, MAX_RESTORED_PANES);
});

test("restoring a tree opens panes in order and collapses failures", async () => {
  const saved = { type: "split", direction: "row", ratio: 0.4, children: [{ type: "pane", cwd: "/a" }, { type: "split", direction: "column", ratio: 0.5, children: [{ type: "pane", cwd: "/gone" }, { type: "pane", cwd: "/c" }] }] };
  const opened = [];
  const root = await restoreTree(saved, async (cwd) => { opened.push(cwd); return cwd === "/gone" ? null : `id${cwd}`; });
  assert.deepEqual(opened, ["/a", "/gone", "/c"]);
  assert.deepEqual(root, { type: "split", direction: "row", ratio: 0.4, children: [{ type: "pane", sessionId: "id/a" }, { type: "pane", sessionId: "id/c" }] });
  assert.equal(await restoreTree({ type: "pane", cwd: "/x" }, async () => null), null);
});

test("a 16 pane chain survives a save and restore", () => {
  let root = pane("p0");
  const sessions = [{ id: "p0", kind: "shell", cwd: "/0" }];
  for (let index = 1; index < MAX_RESTORED_PANES; index++) {
    root = splitPane(root, `p${index - 1}`, `p${index}`, "row");
    sessions.push({ id: `p${index}`, kind: "shell", cwd: `/${index}` });
  }
  const parsed = parseWorkspace(JSON.stringify(serializeWorkspace([{ id: "t", root, title: "" }], sessions, "t")));
  const count = (node) => node.type === "pane" ? 1 : count(node.children[0]) + count(node.children[1]);
  assert.equal(count(parsed.tabs[0].root), MAX_RESTORED_PANES);
});

test("OSC 7 reports from another host are not local directories", () => {
  assert.equal(cwdFromOsc7("file://remote.example/remote/path", "linux", "laptop"), null);
  assert.equal(cwdFromOsc7("file://laptop.local/home/me", "linux", "laptop"), "/home/me");
  assert.equal(cwdFromOsc7("file://LAPTOP/home/me", "linux", "laptop.lan"), "/home/me");
  assert.equal(cwdFromOsc7("file://localhost/home/me", "linux", "laptop"), "/home/me");
  assert.equal(cwdFromOsc7("file:///home/me", "linux", "laptop"), "/home/me");
});

test("PowerShell paths stay literal", () => {
  assert.equal(quotePath("C:\\My Files\\$(1+1).txt", "win32", true), "'C:\\My Files\\$(1+1).txt'");
  assert.equal(quotePath("C:\\it's.txt", "win32", true), "'C:\\it''s.txt'");
  assert.equal(quotePath("C:\\plain\\a.txt", "win32", true), "C:\\plain\\a.txt");
  assert.equal(quotePath("C:\\My Files\\a.txt", "win32"), '"C:\\My Files\\a.txt"');
});

test("terminal generated replies are told apart from typed input", () => {
  for (const reply of ["\x1b[24;80R", "\x1b[?1;2c", "\x1b[>0;276;0c", "\x1b[0n", "\x1b[I", "\x1b[O", "\x1b[<0;10;5M", "\x1b]11;rgb:1c1c/2222/2b2b\x07", "\x1b]10;rgb:0/0/0\x1b\\", "\x1bP1$r0m\x1b\\", "\x1b[?2004;1$y"]) assert.equal(isTerminalReply(reply), true, JSON.stringify(reply));
  for (const typed of ["a", "ls -la\r", "\x1b", "\x1b[A", "\x1b[D", "\x1b[3~", "\x03", "\x1bOP", "粘贴"]) assert.equal(isTerminalReply(typed), false, JSON.stringify(typed));
});

test("byte, uptime and percent formatting", () => {
  assert.deepEqual([formatBytes(0), formatBytes(1023), formatBytes(1536), formatBytes(1048576 * 250), formatBytes(NaN)], ["0 B", "1023 B", "1.5 KiB", "250 MiB", "—"]);
  assert.deepEqual([formatUptime(59), formatUptime(3700), formatUptime(90000)], ["0分", "1时 1分", "1天 1时"]);
  assert.deepEqual([percent(1, 4), percent(9, 4), percent(1, 0)], [25, 100, null]);
});

test("remote paths join and climb without doubling separators", () => {
  assert.deepEqual([joinRemote("/", "a"), joinRemote("/a", "b"), joinRemote("/a/", "b")], ["/a", "/a/b", "/a/b"]);
  assert.deepEqual([parentRemote("/a/b"), parentRemote("/a"), parentRemote("/"), parentRemote("/a/b/")], ["/a", "/", "/", "/a"]);
  assert.deepEqual(["a", "a b", ".x"].map(validEntryName), [true, true, true]);
  assert.deepEqual(["", ".", "..", "a/b", "a\0b"].map(validEntryName), [false, false, false, false, false]);
});

test("entries list folders first in natural order and can hide dotfiles", () => {
  const entries = [{ name: "file10", directory: false }, { name: ".env", directory: false }, { name: "file9", directory: false }, { name: "zdir", directory: true }];
  assert.deepEqual(sortEntries(entries, false).map((entry) => entry.name), ["zdir", "file9", "file10"]);
  assert.equal(sortEntries(entries, true).length, 4);
});

test("row clicks replace, toggle and extend the selection", () => {
  const names = ["a", "b", "c", "d"];
  assert.deepEqual([...nextSelection(new Set(["a"]), names, "b")], ["b"]);
  assert.deepEqual([...nextSelection(new Set(["a"]), names, "b", { toggle: true })], ["a", "b"]);
  assert.deepEqual([...nextSelection(new Set(["a", "b"]), names, "b", { toggle: true })], ["a"]);
  assert.deepEqual([...nextSelection(new Set(["d"]), names, "b", { range: true }, "d")], ["b", "c", "d"]);
});

test("the remote store keeps snapshots stable and forgets closed sessions", () => {
  const handlers = {};
  const store = createRemoteStore({ onRemoteState: (f) => { handlers.state = f; }, onRemoteTransfer: (f) => { handlers.transfer = f; }, onRemoteMetrics: (f) => { handlers.metrics = f; } });
  let changes = 0;
  store.subscribe(() => changes++);
  assert.equal(store.transfers("s"), store.transfers("other"));
  handlers.state({ id: "s", state: "ready" });
  handlers.transfer({ id: "s", transferId: "t", status: "running", bytes: 1 });
  const snapshot = store.transfers("s");
  assert.equal(store.transfers("s"), snapshot);
  handlers.transfer({ id: "s", transferId: "t", status: "done", bytes: 2 });
  assert.deepEqual(store.transfers("s").map((item) => item.bytes), [2]);
  handlers.metrics({ id: "s", available: true, sample: { cpu: 1 } });
  assert.equal(store.metrics("s").sample.cpu, 1);
  handlers.metrics({ id: "s", available: false, reason: "channel-refused", detail: "Session open refused by peer" });
  assert.equal(store.metrics("s").reason, "channel-refused");
  assert.equal(store.metrics("s").detail, "Session open refused by peer");
  assert.equal(store.metrics("s").sample, null);
  handlers.metrics({ id: "s", available: true, sample: { cpu: 2 } });
  assert.equal(store.metrics("s").reason, undefined, "recovery clears the old diagnostic");
  store.clearFinished("s");
  assert.equal(store.transfers("s").length, 0);
  for (let index = 0; index < 30; index++) handlers.transfer({ id: "s", transferId: `t${index}`, status: "done", bytes: 0 });
  assert.equal(store.transfers("s").length, 20);
  handlers.state({ id: "s", state: "closed" });
  assert.deepEqual([store.state("s"), store.metrics("s"), store.transfers("s").length], [null, null, 0]);
  assert.ok(changes > 5);
});

test("every theme is complete and its interface text stays readable", () => {
  const colors = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white", "brightBlack", "brightRed", "brightGreen", "brightYellow", "brightBlue", "brightMagenta", "brightCyan", "brightWhite", "background", "foreground", "cursor", "cursorAccent", "selectionBackground"];
  assert.ok(Object.keys(THEMES).length >= 10);
  assert.ok(Object.values(THEMES).some((theme) => theme.light));
  for (const [id, theme] of Object.entries(THEMES)) {
    for (const color of colors) assert.match(theme.terminal[color], /^#[0-9a-f]{6}([0-9a-f]{2})?$/, `${id} ${color}`);
    const tokens = themeTokens(theme);
    for (const surface of ["--bg", "--surface", "--surface-hover"]) {
      assert.ok(contrast(tokens["--text"], tokens[surface]) >= 4.5, `${id} text on ${surface}`);
      assert.ok(contrast(tokens["--muted"], tokens[surface]) >= 3, `${id} muted on ${surface}`);
    }
    assert.ok(contrast(tokens["--on-accent"], tokens["--accent"]) >= 4.5, `${id} accent button`);
  }
  assert.equal(themeFor("missing"), THEMES.midnight);
});

test("the backdrop stays opaque unless something can show through it", () => {
  const settings = { backgroundMode: "opaque", backgroundOpacity: 0.8, backgroundImage: null, backgroundImageOpacity: 0.3 };
  assert.deepEqual(backdrop(settings, false), { base: 1, image: "", imageOpacity: 0.3, clear: false, panelAlpha: 1 });
  // A translucent choice waits for the transparent window that needs a restart.
  assert.equal(backdrop({ ...settings, backgroundMode: "translucent" }, false).clear, false);
  assert.equal(backdrop({ ...settings, backgroundMode: "translucent" }, true).base, 0.8);
  assert.equal(backdrop({ ...settings, backgroundMode: "translucent", backgroundOpacity: 5 }, true).base, 0.95);
  const transparent = backdrop({ ...settings, backgroundMode: "transparent" }, true);
  assert.deepEqual([transparent.base, transparent.clear, transparent.panelAlpha], [0, true, 0.6]);
  const image = backdrop({ ...settings, backgroundImage: { url: "file:///a.png", name: "a.png" } }, false);
  assert.deepEqual([image.base, image.image, image.clear, image.panelAlpha], [1, "file:///a.png", true, 0.85]);
});

test("a clear terminal keeps its background color at zero alpha", () => {
  assert.equal(terminalTheme(THEMES.nord, true).background, "#2e344000");
  assert.equal(terminalTheme(THEMES.nord, false), THEMES.nord.terminal);
});

const terminals = [
  { ref: { sessionId: "aaaaaaaa-1", epoch: 1 }, shortId: "aaaaaaaa", label: "A", kind: "shell", status: "running" },
  { ref: { sessionId: "bbbbbbbb-1", epoch: 2 }, shortId: "bbbbbbbb", label: "B", kind: "ssh", status: "running" },
  { ref: { sessionId: "cccccccc-1", epoch: 1 }, shortId: "cccccccc", label: "C", kind: "sftp", status: "running" },
];

test("the conversation assembles streamed text and keeps one card per operation", () => {
  let state = conversationReducer(emptyConversation, { type: "user", id: "u", text: "hi" });
  assert.equal(state.running, true);
  for (const delta of ["Hel", "lo"]) state = conversationReducer(state, { type: "text-delta", itemId: "m", delta });
  state = conversationReducer(state, { type: "text-done", itemId: "m", text: "Hello!" });
  assert.equal(state.entries.at(-1).text, "Hello!");
  for (const status of ["pendingApproval", "running"]) state = conversationReducer(state, { type: "operation", operation: { operationId: "o", status } });
  assert.equal(state.entries.filter((entry) => entry.kind === "operation").length, 1);
  assert.equal(state.entries.at(-1).operation.status, "running");
  state = conversationReducer(state, { type: "turn-completed", error: "failed" });
  assert.equal(state.running, false);
  assert.equal(state.entries.at(-1).kind, "error");
});

test("scope helpers keep epochs and drop closed terminals", () => {
  const ids = new Set(["bbbbbbbb-1", "closed"]);
  assert.deepEqual(refsFor(terminals, ids), [{ sessionId: "bbbbbbbb-1", epoch: 2 }]);
  assert.deepEqual([...pruneSelection(ids, terminals)], ["bbbbbbbb-1"]);
  const unchanged = new Set(["aaaaaaaa-1"]);
  assert.equal(pruneSelection(unchanged, terminals), unchanged);
  assert.equal(scopeSummary(terminals, new Set()), "无");
  assert.equal(scopeSummary(terminals, new Set(["aaaaaaaa-1", "bbbbbbbb-1"])), "A、B");
  assert.equal(scopeSummary(terminals, new Set(terminals.map((terminal) => terminal.ref.sessionId))), "3 个终端");
});

test("only running shells and SSH terminals can be execution targets", () => {
  assert.deepEqual(terminals.map(executable), [true, true, false]);
  assert.equal(executable({ ...terminals[0], status: "exited" }), false);
});

test("mentions resolve while typing and citations link only known terminals", () => {
  assert.equal(mentionQuery("check @bb"), "bb");
  assert.equal(mentionQuery("mail me@host"), null);
  assert.equal(mentionQuery("done @b "), null);
  assert.deepEqual(mentionMatches(terminals, "b").map((terminal) => terminal.label), ["B"]);
  const parts = citationParts("see bbbbbbbb and deadbeef here", terminals);
  assert.equal(parts[1].terminal.label, "B");
  assert.equal(parts.at(-1), " and deadbeef here");
});

test("errors keep technical details apart from the message", () => {
  const failed = conversationReducer(emptyConversation, { type: "error", message: "friendly", detail: "raw" });
  assert.deepEqual([failed.entries[0].text, failed.entries[0].detail], ["friendly", "raw"]);
  const turn = conversationReducer(emptyConversation, { type: "turn-completed", error: "friendly", errorDetail: "raw" });
  assert.equal(turn.entries[0].detail, "raw");
});

test("a blocked command explains the limit in the user's terms", () => {
  assert.match(operationReason("A full-screen program is active, so the command was not sent. Only shell commands at a prompt are supported."), /发送给运行中的程序/);
  assert.match(operationReason("The terminal stayed busy, so the command was not sent."), /按一次回车/);
  assert.equal(operationReason("Cancelled."), "Cancelled.");
  assert.equal(operationReason(null), null);
});

const form = (patch) => ({ ...emptyMonitorForm, ...patch });

test("monitor forms become precise requests", () => {
  assert.deepEqual(buildMonitorRequest(form({ process: "42", field: "cpu", value: "80" }), []).target, { kind: "process", pid: 42 });
  assert.deepEqual(buildMonitorRequest(form({ process: "nginx" }), []).target, { kind: "process", name: "nginx" });
  const log = buildMonitorRequest(form({ host: "s1", kind: "log", path: "/var/log/x", field: "text", value: "ERROR" }), []);
  assert.deepEqual([log.host, log.conditions], [{ sessionId: "s1" }, [{ field: "text", op: "contains", value: "ERROR" }]]);
  const shm = buildMonitorRequest(form({ kind: "sharedMemory", region: "r", layout: "count:0:u32\nflag : 4 : u8", field: "count", value: "9" }), []);
  assert.deepEqual(shm.target.layout, [{ field: "count", offset: 0, type: "u32" }, { field: "flag", offset: 4, type: "u8" }]);
  assert.deepEqual(buildMonitorRequest(form({ process: "1", field: "exists" }), []).conditions, [{ op: "gone" }]);
});

test("an action carries the terminal's pinned identity", () => {
  const terminals = [{ ref: { sessionId: "s1", epoch: 3 } }];
  const { action } = buildMonitorRequest(form({ process: "1", field: "cpu", value: "1", withAction: true, command: "restart", terminal: "s1", maxRuns: "2" }), terminals);
  assert.deepEqual(action, { command: "restart", terminal: { sessionId: "s1", epoch: 3 }, maxRuns: 2 });
  assert.throws(() => buildMonitorRequest(form({ process: "1", withAction: true, terminal: "gone" }), terminals), /目标终端/);
});

test("monitor events update the list in place", () => {
  const first = monitorReducer([], { type: "monitor", monitor: { id: "a", status: "pending" } });
  const second = monitorReducer(first, { type: "monitor", monitor: { id: "a", status: "running" } });
  assert.deepEqual(second, [{ id: "a", status: "running" }]);
  assert.deepEqual(monitorReducer(second, { type: "monitor-removed", id: "a" }), []);
});

test("monitor values and trends read cleanly", () => {
  assert.deepEqual(monitorMetrics({ at: 1, exists: true, pid: 3, cpu: 4, memoryKb: 5, command: "x" }), [["cpu", 4], ["memoryKb", 5]]);
  assert.deepEqual(monitorMetrics({ exists: false }), [["目标", "不存在"]]);
  assert.equal(sparkline([{ cpu: 1 }], "cpu"), "");
  assert.equal(sparkline([{ cpu: 0 }, { cpu: 10 }], "cpu"), "0.0,25.0 100.0,1.0");
  assert.equal(targetSummary({ kind: "thread", pid: 1, tid: 2 }), "线程 1/2");
});

const processList = [
  { pid: 1, ppid: 0, cpu: 0, memoryKb: 100, command: "init" },
  { pid: 10, ppid: 1, cpu: 5, memoryKb: 200, command: "server" },
  { pid: 11, ppid: 10, cpu: 50, memoryKb: 300, command: "worker" },
  { pid: 12, ppid: 10, cpu: 1, memoryKb: 400, command: "logger" },
  { pid: 99, ppid: 77, cpu: 2, memoryKb: 500, command: "orphan" },
];

test("processes form a tree, with orphans at the top level", () => {
  const rows = processRows(processList);
  assert.deepEqual(rows.map(({ pid, depth }) => [pid, depth]), [[1, 0], [10, 1], [12, 2], [11, 2], [99, 0]]);
});

test("a cycle in reported parents cannot hang the window", () => {
  const rows = processRows([{ pid: 1, ppid: 2, cpu: 0, memoryKb: 0, command: "a" }, { pid: 2, ppid: 1, cpu: 0, memoryKb: 0, command: "b" }]);
  assert.deepEqual(rows.map(({ pid }) => pid).sort(), [1, 2]);
});

test("searching keeps a match's ancestors for context, or lists flat when sorted", () => {
  assert.deepEqual(processRows(processList, { query: "worker" }).map(({ pid }) => pid), [1, 10, 11]);
  assert.deepEqual(processRows(processList, { query: "12" }).map(({ pid }) => pid), [1, 10, 12]);
  assert.deepEqual(processRows(processList, { sort: "cpu" }).map(({ pid }) => pid).slice(0, 2), [11, 10]);
  assert.deepEqual(processRows(processList, { sort: "memoryKb", query: "o" }).map(({ pid }) => pid), [99, 12, 11]);
});

test("trends follow each process, and end with it", () => {
  let trends = pushTrends(new Map(), processList.slice(0, 2), 3);
  trends = pushTrends(trends, [{ ...processList[1], cpu: 9 }], 3);
  assert.deepEqual(trends.get(10).map(({ cpu }) => cpu), [5, 9]);
  assert.equal(trends.has(1), false);
  for (let index = 0; index < 5; index++) trends = pushTrends(trends, [processList[1]], 3);
  assert.equal(trends.get(10).length, 3);
});

test("attaching a monitor pins the selected process and host", () => {
  assert.deepEqual(monitorRequest(processList[2], "local"), { host: "local", label: "worker (11)", target: { kind: "process", pid: 11 }, conditions: [{ op: "gone" }], intervalSeconds: 5, durationMinutes: 60 });
  assert.deepEqual(monitorRequest(processList[2], "s1").host, { sessionId: "s1" });
  assert.deepEqual(summarize(processList), { count: 5, cpu: 58, memoryKb: 1500 });
});

test("a target a host can't read is explained up front", () => {
  assert.match(kindNote("sharedMemory", "local", "win32"), /Windows/);
  assert.equal(kindNote("process", "local", "win32"), "");
  assert.equal(kindNote("thread", "local", "win32"), "");
  assert.match(kindNote("thread", "local", "darwin"), /macOS/);
  assert.match(kindNote("sharedMemory", "s1", "win32"), /Linux/);
  assert.equal(kindNote("log", "s1", "win32"), "");
});

const tab = (id, root, focus = paneIds(root)[0]) => ({ id, root, focus, title: id, broadcast: false, zoomed: null });
const newTab = () => ({ id: "new", root: null, focus: null, title: "", broadcast: false, zoomed: null });

test("a pane can be inserted on any side of another", () => {
  const rect = (side) => {
    const { panes } = computeLayout(insertPane(pane("a"), "a", "b", side));
    return panes.find(({ sessionId }) => sessionId === "b").rect;
  };
  assert.deepEqual(rect("left"), { x: 0, y: 0, w: 0.5, h: 1 });
  assert.deepEqual(rect("right"), { x: 0.5, y: 0, w: 0.5, h: 1 });
  assert.deepEqual(rect("top"), { x: 0, y: 0, w: 1, h: 0.5 });
  assert.deepEqual(rect("bottom"), { x: 0, y: 0.5, w: 1, h: 0.5 });
});

test("the nearest edge decides where a pane lands", () => {
  assert.equal(dropSide(0.1, 0.5), "left");
  assert.equal(dropSide(0.9, 0.5), "right");
  assert.equal(dropSide(0.5, 0.1), "top");
  assert.equal(dropSide(0.5, 0.9), "bottom");
});

test("moving a pane inside a tab rearranges it and keeps every session", () => {
  const root = splitPane(splitPane(pane("a"), "a", "b", "row"), "b", "c", "column");
  const [moved] = movePaneInTabs([tab("t", root)], "c", { tabId: "t", target: "a", side: "top" });
  assert.deepEqual(paneIds(moved.root).sort(), ["a", "b", "c"]);
  const { panes } = computeLayout(moved.root);
  assert.deepEqual(panes.find(({ sessionId }) => sessionId === "c").rect, { x: 0, y: 0, w: 0.5, h: 0.5 });
  assert.equal(moved.focus, "c");
});

test("moving a pane to another tab keeps the source tab's other panes", () => {
  const tabs = [tab("one", splitPane(pane("a"), "a", "b", "row")), tab("two", pane("c"))];
  const next = movePaneInTabs(tabs, "b", { tabId: "two", target: "c", side: "bottom" });
  assert.deepEqual(next.map(({ root }) => paneIds(root)), [["a"], ["c", "b"]]);
  assert.equal(next[0].focus, "a");
  assert.equal(next[1].focus, "b");
});

test("dropping on a tab without a target lands next to its focused pane", () => {
  const tabs = [tab("one", pane("a")), tab("two", splitPane(pane("c"), "c", "d", "row"), "c")];
  const next = movePaneInTabs(tabs, "a", { tabId: "two" });
  assert.equal(next.length, 1, "the emptied source tab goes away");
  assert.deepEqual(paneIds(next[0].root), ["c", "a", "d"]);
});

test("a pane can become its own tab, unless it already is one", () => {
  const split = [tab("one", splitPane(pane("a"), "a", "b", "row"))];
  const next = movePaneInTabs(split, "b", {}, newTab);
  assert.deepEqual(next.map(({ root }) => paneIds(root)), [["a"], ["b"]]);
  const alone = [tab("one", pane("a"))];
  assert.equal(movePaneInTabs(alone, "a", {}, newTab), alone);
});

test("impossible moves leave the layout untouched", () => {
  const tabs = [tab("one", splitPane(pane("a"), "a", "b", "row")), tab("two", pane("c"))];
  assert.equal(movePaneInTabs(tabs, "a", { tabId: "one", target: "a", side: "left" }), tabs);
  assert.equal(movePaneInTabs(tabs, "missing", { tabId: "one" }), tabs);
  assert.equal(movePaneInTabs(tabs, "a", { tabId: "gone" }), tabs);
  assert.equal(movePaneInTabs(tabs, "a", { tabId: "two", target: "missing" }), tabs);
  assert.equal(movePaneInTabs([tab("one", pane("a"))], "a", { tabId: "one" }).length, 1);
});

test("moving clears zoom, and a moved layout survives a save and restore", async () => {
  const tabs = [{ ...tab("one", splitPane(pane("a"), "a", "b", "row")), zoomed: "b" }, tab("two", pane("c"))];
  const next = movePaneInTabs(tabs, "b", { tabId: "two", target: "c", side: "left" });
  assert.equal(next[0].zoomed, null);
  const sessions = ["a", "b", "c"].map((id) => ({ id, kind: "shell", cwd: `/${id}` }));
  const saved = parseWorkspace(JSON.stringify(serializeWorkspace(next, sessions, "two")));
  assert.deepEqual(saved.tabs[1].root.children.map(({ cwd }) => cwd), ["/b", "/c"]);
  assert.equal(saved.tabs[1].root.direction, "row");
});

test("dock panels move to any edge of each other and none is lost", () => {
  for (const side of ["left", "right", "top", "bottom"]) {
    const moved = moveDockPanel(defaultDock(), "agent", "sidebar", side);
    assert.deepEqual(paneIds(moved).sort(), [...DOCK_PANELS].sort());
    const { panes } = computeLayout(moved);
    const agent = panes.find(({ sessionId }) => sessionId === "agent").rect;
    const sidebar = panes.find(({ sessionId }) => sessionId === "sidebar").rect;
    assert.ok({ left: agent.x < sidebar.x, right: agent.x > sidebar.x, top: agent.y < sidebar.y, bottom: agent.y > sidebar.y }[side], side);
  }
});

test("a panel dropped on itself or on nothing stays put", () => {
  const dock = defaultDock();
  assert.equal(moveDockPanel(dock, "agent", "agent", "left"), dock);
  assert.equal(moveDockPanel(dock, "agent", "missing", "left"), dock);
});

test("a hidden panel frees its space, and resizing still reaches the saved layout", () => {
  const dock = defaultDock();
  const shown = computeLayout(visibleDock(dock, ["agent", "monitor"]));
  assert.deepEqual(shown.panes.map(({ sessionId }) => sessionId), ["sidebar", "terminals"]);
  assert.equal(shown.dividers.length, 1);
  assert.deepEqual(shown.dividers[0].path, []);
  const swapped = moveDockPanel(dock, "sidebar", "agent", "right");
  const [divider] = computeLayout(visibleDock(swapped, ["agent"])).dividers.length ? computeLayout(visibleDock(swapped, ["agent"])).dividers : [{ path: [] }];
  assert.deepEqual(paneIds(setRatio(swapped, divider.path, 0.4)).sort(), [...DOCK_PANELS].sort());
  assert.deepEqual(computeLayout(visibleDock(dock, [])).dividers.map(({ path }) => path), [[], [1], [1, 1]]);
});

test("a dock layout survives a save, and a damaged one is refused", () => {
  const moved = setRatio(moveDockPanel(defaultDock(), "agent", "sidebar", "left"), [], 0.3);
  assert.deepEqual(parseDock(JSON.stringify(moved)), moved);
  for (const raw of ["not json", null, "{}", JSON.stringify(pane("agent")), JSON.stringify({ type: "split", direction: "row", children: [pane("agent"), pane("agent")] }), JSON.stringify({ type: "split", direction: "up", children: [pane("agent"), pane("sidebar")] })]) assert.equal(parseDock(raw), null, String(raw));
  assert.equal(parseDock(JSON.stringify({ ...defaultDock(), ratio: 5 })).ratio, 1 - MIN_RATIO);
});

test("a terminal pane can dock beside any panel and return to a tab", () => {
  const leaf = dockedTerminal("b");
  const tabs = [tab("one", splitPane(pane("a"), "a", "b", "row"))];
  const remaining = takePaneFromTabs(tabs, "b");
  assert.deepEqual(paneIds(remaining[0].root), ["a"]);
  assert.equal(remaining[0].focus, "a");
  const dock = insertPane(defaultDock(), "agent", leaf, "left");
  assert.deepEqual(dockedTerminals(dock), ["b"]);
  const { panes } = computeLayout(dock);
  assert.ok(panes.find(({ sessionId }) => sessionId === leaf).rect.x < panes.find(({ sessionId }) => sessionId === "agent").rect.x);
  const back = movePaneInTabs(remaining, "b", { tabId: "one", target: "a", side: "bottom", detached: true });
  assert.deepEqual(paneIds(back[0].root), ["a", "b"]);
  assert.equal(back[0].focus, "b");
});

test("a lone pane can leave its tab, and a docked pane can found a new one", () => {
  assert.deepEqual(takePaneFromTabs([tab("one", pane("a"))], "a"), []);
  const next = movePaneInTabs([tab("one", pane("a"))], "b", { detached: true }, newTab);
  assert.deepEqual(next.map(({ root }) => paneIds(root)), [["a"], ["b"]]);
  assert.deepEqual(paneIds(movePaneInTabs([tab("one", pane("a"))], "b", { tabId: "one", detached: true })[0].root), ["a", "b"]);
});

test("docked terminals are never saved, and leave when their session does", () => {
  const dock = insertPane(defaultDock(), "agent", dockedTerminal("b"), "left");
  assert.deepEqual(dockedTerminals(storableDock(dock)), []);
  assert.deepEqual(paneIds(parseDock(JSON.stringify(storableDock(dock)))).sort(), [...DOCK_PANELS].sort());
  assert.deepEqual(dockedTerminals(pruneDock(dock, ["a"])), []);
  assert.equal(pruneDock(dock, ["b"]), dock);
});

test("a layout saved before the monitoring panel keeps its arrangement and gains the panel", () => {
  const old = { type: "split", direction: "row", ratio: 0.3, children: [pane("agent"), { type: "split", direction: "column", ratio: 0.4, children: [pane("sidebar"), pane("terminals")] }] };
  const restored = parseDock(JSON.stringify(old));
  assert.deepEqual(paneIds(restored).sort(), [...DOCK_PANELS].sort());
  assert.equal(restored.ratio, 0.3);
  assert.equal(restored.children[0].direction, "column");
  const withoutTerminals = { type: "split", direction: "row", ratio: 0.5, children: [pane("agent"), pane("sidebar")] };
  assert.equal(parseDock(JSON.stringify(withoutTerminals)), null);
});

test("hiding the monitoring panel returns its space to the Agent panel", () => {
  const { panes } = computeLayout(visibleDock(defaultDock(), ["monitor"]));
  assert.deepEqual(panes.map(({ sessionId }) => sessionId), ["sidebar", "terminals", "agent"]);
  assert.equal(panes.find(({ sessionId }) => sessionId === "agent").rect.h, 1);
});

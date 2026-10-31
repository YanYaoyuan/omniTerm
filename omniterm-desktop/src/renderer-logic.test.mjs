// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import test from "node:test";
import { citationParts, conversationReducer, emptyConversation, executable, mentionMatches, mentionQuery, pruneSelection, refsFor, scopeSummary } from "./agent/agent-logic.js";
import assert from "node:assert/strict";
import { computeLayout, equalize, MIN_RATIO, neighborPane, pane, paneIds, removePane, setRatio, splitPane } from "./layout.js";
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

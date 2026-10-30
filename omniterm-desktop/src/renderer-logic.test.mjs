// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import test from "node:test";
import assert from "node:assert/strict";
import { computeLayout, equalize, MIN_RATIO, neighborPane, pane, paneIds, removePane, setRatio, splitPane } from "./layout.js";
import { isAppShortcut, matchShortcut, shortcutLabel, tabCycle, tabNumber } from "./shortcuts.js";
import { cwdFromOsc7, fuzzyScore, quotePath } from "./terminal-utils.js";
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

// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import test from "node:test";
import assert from "node:assert/strict";
import { computeLayout, equalize, MIN_RATIO, neighborPane, pane, paneIds, removePane, setRatio, splitPane } from "./layout.js";
import { isAppShortcut, matchShortcut, shortcutLabel, tabCycle, tabNumber } from "./shortcuts.js";
import { cwdFromOsc7, fuzzyScore, quotePath } from "./terminal-utils.js";

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

import test from "node:test";
import assert from "node:assert/strict";
import { contrast, THEMES } from "../src/themes.js";
import { windowChrome, windowTheme } from "./window-chrome.mjs";

test("native caption buttons remain readable in every selectable theme", () => {
  for (const id of Object.keys(THEMES)) {
    const theme = windowTheme(id);
    assert.ok(contrast(theme.color, theme.symbolColor) >= 4.5, id);
    for (const platform of ["win32", "linux"]) {
      const options = windowChrome(platform, theme);
      assert.equal(options.titleBarStyle, "hidden");
      assert.equal(options.autoHideMenuBar, true);
      assert.equal(options.titleBarOverlay.height, 43);
    }
  }
});

test("renderer can select only existing themes, with macOS traffic lights preserved", () => {
  for (const id of [null, {}, "__proto__", "unknown"]) assert.throws(() => windowTheme(id));
  assert.equal(windowTheme("midnight").dark, true);
  assert.equal(windowTheme("github").dark, false);
  const options = windowChrome("darwin", windowTheme("midnight"));
  assert.equal(options.titleBarStyle, "hiddenInset");
  assert.equal(options.titleBarOverlay, undefined);
});

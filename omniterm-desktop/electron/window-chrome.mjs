import { THEMES, themeFor, themeTokens } from "../src/themes.js";

export const TITLEBAR_HEIGHT = 43;
export function windowTheme(id) {
  if (typeof id !== "string" || !Object.hasOwn(THEMES, id)) throw new Error("Unknown window theme.");
  const theme = themeFor(id);
  const tokens = themeTokens(theme);
  return { id, dark: !theme.light, background: theme.chrome, color: tokens["--surface"], symbolColor: tokens["--text"] };
}

export function windowChrome(platform, theme) {
  if (platform === "darwin") return { titleBarStyle: "hiddenInset", trafficLightPosition: { x: 20, y: 20 } };
  return { titleBarStyle: "hidden", autoHideMenuBar: true, titleBarOverlay: { color: theme.color, symbolColor: theme.symbolColor, height: TITLEBAR_HEIGHT } };
}

export function applyWindowTheme(window, platform, theme) {
  if (!window || window.isDestroyed()) return;
  if (platform !== "darwin") window.setTitleBarOverlay(windowChrome(platform, theme).titleBarOverlay);
}

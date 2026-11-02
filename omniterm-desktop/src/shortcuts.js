// Application shortcuts. macOS uses Command; other platforms use Ctrl+Shift so
// shell keys such as Ctrl+W (delete word), Ctrl+D (EOF), Ctrl+F and Ctrl+K keep
// reaching the terminal. Keys are matched by KeyboardEvent.code so Option/Shift
// layouts do not change the binding.

const isMac = (platform) => platform === "darwin";

// mac / other: { code, shift, alt } where shift/alt default to false.
export const SHORTCUTS = {
  newTab: { label: "新建标签", mac: { code: "KeyT" }, other: { code: "KeyT", shift: true } },
  closePane: { label: "关闭当前窗格", mac: { code: "KeyW" }, other: { code: "KeyW", shift: true } },
  splitRight: { label: "向右分屏", mac: { code: "KeyD" }, other: { code: "KeyE", shift: true } },
  splitDown: { label: "向下分屏", mac: { code: "KeyD", shift: true }, other: { code: "KeyO", shift: true } },
  nextPane: { label: "下一个窗格", mac: { code: "BracketRight" }, other: { code: "BracketRight", shift: true } },
  previousPane: { label: "上一个窗格", mac: { code: "BracketLeft" }, other: { code: "BracketLeft", shift: true } },
  paneLeft: { label: "切换到左侧窗格", mac: { code: "ArrowLeft", alt: true }, other: { code: "ArrowLeft", alt: true, ctrlOnly: true } },
  paneRight: { label: "切换到右侧窗格", mac: { code: "ArrowRight", alt: true }, other: { code: "ArrowRight", alt: true, ctrlOnly: true } },
  paneUp: { label: "切换到上方窗格", mac: { code: "ArrowUp", alt: true }, other: { code: "ArrowUp", alt: true, ctrlOnly: true } },
  paneDown: { label: "切换到下方窗格", mac: { code: "ArrowDown", alt: true }, other: { code: "ArrowDown", alt: true, ctrlOnly: true } },
  maximizePane: { label: "最大化 / 还原窗格", mac: { code: "Enter", shift: true }, other: { code: "Enter", shift: true } },
  find: { label: "搜索终端输出", mac: { code: "KeyF" }, other: { code: "KeyF", shift: true } },
  clear: { label: "清空终端", mac: { code: "KeyK" }, other: { code: "KeyK", shift: true } },
  palette: { label: "命令面板", mac: { code: "KeyP", shift: true }, other: { code: "KeyP", shift: true } },
  broadcast: { label: "广播输入到当前标签所有窗格", mac: { code: "KeyI", alt: true }, other: { code: "KeyI", shift: true } },
  sendToAgent: { label: "附加选中内容到 omniTerm Agent", mac: { code: "KeyA", shift: true }, other: { code: "KeyA", shift: true } },
  toggleAgent: { label: "显示 / 隐藏 omniTerm Agent 面板", mac: { code: "KeyJ" }, other: { code: "KeyJ", shift: true } },
  fontBigger: { label: "放大字体", mac: { code: "Equal" }, other: { code: "Equal", ctrlOnly: true } },
  fontSmaller: { label: "缩小字体", mac: { code: "Minus" }, other: { code: "Minus", ctrlOnly: true } },
  fontReset: { label: "重置字体大小", mac: { code: "Digit0" }, other: { code: "Digit0", ctrlOnly: true } },
  settings: { label: "打开设置", mac: { code: "Comma" }, other: { code: "Comma", ctrlOnly: true } },
};

// Ctrl+Tab cycles tabs everywhere; it has no meaning to shells.
export function tabCycle(event) {
  if (event.type !== "keydown" || event.code !== "Tab" || !event.ctrlKey || event.metaKey || event.altKey) return 0;
  return event.shiftKey ? -1 : 1;
}

// Cmd+1…9 on macOS, Alt+1…9 elsewhere (as in most Linux terminals).
export function tabNumber(event, platform) {
  if (event.type !== "keydown" || !/^Digit[1-9]$/.test(event.code) || event.shiftKey) return 0;
  const mac = isMac(platform);
  const modifier = mac ? event.metaKey && !event.ctrlKey && !event.altKey : event.altKey && !event.ctrlKey && !event.metaKey;
  return modifier ? Number(event.code.slice(5)) : 0;
}

function matches(binding, event, platform) {
  if (event.code !== binding.code) return false;
  if (isMac(platform)) {
    return event.metaKey && !event.ctrlKey && event.shiftKey === Boolean(binding.shift) && event.altKey === Boolean(binding.alt);
  }
  if (event.metaKey || !event.ctrlKey) return false;
  if (binding.ctrlOnly) return event.altKey === Boolean(binding.alt) && (event.shiftKey === Boolean(binding.shift) || binding.code === "Equal");
  return event.shiftKey && event.altKey === Boolean(binding.alt);
}

export function matchShortcut(event, platform) {
  if (event.type !== "keydown" || event.isComposing) return null;
  for (const [name, definition] of Object.entries(SHORTCUTS)) {
    if (matches(isMac(platform) ? definition.mac : definition.other, event, platform)) return name;
  }
  return null;
}

// Whether the terminal should leave this key to the application.
export function isAppShortcut(event, platform) {
  return Boolean(matchShortcut(event, platform) || tabCycle(event) || tabNumber(event, platform));
}

const codeLabels = { BracketLeft: "[", BracketRight: "]", ArrowLeft: "←", ArrowRight: "→", ArrowUp: "↑", ArrowDown: "↓", Equal: "+", Minus: "−", Comma: ",", Enter: "↩" };

export function shortcutLabel(name, platform) {
  const definition = SHORTCUTS[name];
  if (!definition) return "";
  const binding = isMac(platform) ? definition.mac : definition.other;
  const key = codeLabels[binding.code] || binding.code.replace(/^(Key|Digit)/, "");
  if (isMac(platform)) return `${binding.alt ? "⌥" : ""}${binding.shift ? "⇧" : ""}⌘${key}`;
  const parts = ["Ctrl"];
  if (binding.alt) parts.push("Alt");
  if (binding.shift || !binding.ctrlOnly) parts.push("Shift");
  return [...new Set(parts), key].join("+");
}

// Built-in color schemes. `chrome` sets the terminal surface around xterm's canvas
// so padding and split gutters match the scheme, and themeTokens derives the rest
// of the interface from the same colors. `accent` and `text` override those
// derived colors where the terminal palette alone would not read well.

const ANSI = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white", "brightBlack", "brightRed", "brightGreen", "brightYellow", "brightBlue", "brightMagenta", "brightCyan", "brightWhite"];

function scheme(name, background, foreground, cursor, selectionBackground, colors, extra = {}) {
  const terminal = { background, foreground, cursor, cursorAccent: background, selectionBackground };
  ANSI.forEach((color, index) => { terminal[color] = colors[index]; });
  return { name, chrome: background, ...extra, terminal };
}

export const THEMES = {
  midnight: {
    name: "Midnight",
    chrome: "#14181f",
    terminal: {
      background: "#14181f", foreground: "#e0e6ee", cursor: "#8db4ff", cursorAccent: "#14181f",
      selectionBackground: "#8db4ff36",
      black: "#39414e", red: "#e58a91", green: "#a5c9a1", yellow: "#dec38d",
      blue: "#8db4ff", magenta: "#bdacd8", cyan: "#92c4cc", white: "#e0e6ee",
      brightBlack: "#7d899a", brightRed: "#f4a2a8", brightGreen: "#b9dab5", brightYellow: "#efd5a5",
      brightBlue: "#accaff", brightMagenta: "#d4c3ed", brightCyan: "#aedce3", brightWhite: "#f5f7fa",
    },
  },
  graphite: {
    name: "Graphite",
    chrome: "#1a1a1a",
    terminal: {
      background: "#1a1a1a", foreground: "#d6d6d6", cursor: "#f0f0f0", cursorAccent: "#1a1a1a",
      selectionBackground: "#ffffff30",
      black: "#3a3a3a", red: "#e06c75", green: "#98c379", yellow: "#e5c07b",
      blue: "#61afef", magenta: "#c678dd", cyan: "#56b6c2", white: "#d6d6d6",
      brightBlack: "#7a7a7a", brightRed: "#ef8a92", brightGreen: "#b0d898", brightYellow: "#f0d39b",
      brightBlue: "#86c3f7", brightMagenta: "#d899ea", brightCyan: "#7accd6", brightWhite: "#ffffff",
    },
  },
  aurora: {
    name: "Aurora",
    chrome: "#0f1a1c",
    terminal: {
      background: "#0f1a1c", foreground: "#d3e6e2", cursor: "#6fe0c2", cursorAccent: "#0f1a1c",
      selectionBackground: "#6fe0c232",
      black: "#26393c", red: "#f28b82", green: "#6fe0c2", yellow: "#f4d58d",
      blue: "#7cb7ff", magenta: "#d4a5ff", cyan: "#7fdbe6", white: "#d3e6e2",
      brightBlack: "#5f7b7f", brightRed: "#ffa59e", brightGreen: "#93f0d6", brightYellow: "#ffe4a8",
      brightBlue: "#a0cbff", brightMagenta: "#e3c2ff", brightCyan: "#a3ebf2", brightWhite: "#f2fbf9",
    },
  },
  ember: {
    name: "Ember",
    chrome: "#1d1714",
    terminal: {
      background: "#1d1714", foreground: "#ecdccf", cursor: "#ff9e64", cursorAccent: "#1d1714",
      selectionBackground: "#ff9e6430",
      black: "#3d322c", red: "#f47c6b", green: "#b5c97a", yellow: "#f2c66d",
      blue: "#88aee0", magenta: "#d79ac4", cyan: "#8ec9b9", white: "#ecdccf",
      brightBlack: "#857065", brightRed: "#ff9a8b", brightGreen: "#cadc95", brightYellow: "#ffd98c",
      brightBlue: "#a7c5ee", brightMagenta: "#e8b5d7", brightCyan: "#aadccf", brightWhite: "#fff6ee",
    },
  },
  daylight: {
    name: "Daylight",
    chrome: "#fbfbfa",
    light: true,
    terminal: {
      background: "#fbfbfa", foreground: "#2b3038", cursor: "#2f6fdb", cursorAccent: "#fbfbfa",
      selectionBackground: "#2f6fdb2e",
      black: "#2b3038", red: "#c4373f", green: "#2f7d3a", yellow: "#94660b",
      blue: "#2f6fdb", magenta: "#8a45b5", cyan: "#167a86", white: "#c9ced6",
      brightBlack: "#6b7380", brightRed: "#dc4a52", brightGreen: "#3a9447", brightYellow: "#b07c12",
      brightBlue: "#4a86ec", brightMagenta: "#a35acf", brightCyan: "#1f93a1", brightWhite: "#ffffff",
    },
  },
  tokyoNight: scheme("Tokyo Night", "#1a1b26", "#c0caf5", "#c0caf5", "#7aa2f740", ["#15161e", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#a9b1d6", "#414868", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#c0caf5"], { accent: "#7aa2f7" }),
  dracula: scheme("Dracula", "#282a36", "#f8f8f2", "#f8f8f2", "#44475a", ["#21222c", "#ff5555", "#50fa7b", "#f1fa8c", "#bd93f9", "#ff79c6", "#8be9fd", "#f8f8f2", "#6272a4", "#ff6e6e", "#69ff94", "#ffffa5", "#d6acff", "#ff92df", "#a4ffff", "#ffffff"], { accent: "#bd93f9" }),
  nord: scheme("Nord", "#2e3440", "#d8dee9", "#d8dee9", "#88c0d040", ["#3b4252", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#88c0d0", "#e5e9f0", "#4c566a", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#8fbcbb", "#eceff4"], { accent: "#88c0d0" }),
  catppuccin: scheme("Catppuccin Mocha", "#1e1e2e", "#cdd6f4", "#f5e0dc", "#585b7080", ["#45475a", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7", "#94e2d5", "#bac2de", "#585b70", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7", "#94e2d5", "#a6adc8"], { accent: "#cba6f7" }),
  gruvbox: scheme("Gruvbox Dark", "#282828", "#ebdbb2", "#ebdbb2", "#ebdbb230", ["#282828", "#cc241d", "#98971a", "#d79921", "#458588", "#b16286", "#689d6a", "#a89984", "#928374", "#fb4934", "#b8bb26", "#fabd2f", "#83a598", "#d3869b", "#8ec07c", "#ebdbb2"], { accent: "#fabd2f" }),
  solarizedDark: scheme("Solarized Dark", "#002b36", "#839496", "#93a1a1", "#93a1a130", ["#073642", "#dc322f", "#859900", "#b58900", "#268bd2", "#d33682", "#2aa198", "#eee8d5", "#586e75", "#cb4b16", "#859900", "#b58900", "#268bd2", "#6c71c4", "#2aa198", "#fdf6e3"], { accent: "#2aa198", text: "#c5ccc9" }),
  solarizedLight: scheme("Solarized Light", "#fdf6e3", "#586e75", "#268bd2", "#268bd22e", ["#073642", "#dc322f", "#6c7a00", "#946f00", "#268bd2", "#d33682", "#1f807a", "#93a1a1", "#586e75", "#cb4b16", "#6c7a00", "#946f00", "#268bd2", "#6c71c4", "#1f807a", "#073642"], { accent: "#1c6da8", light: true, text: "#2f4a53" }),
  github: scheme("GitHub Light", "#ffffff", "#1f2328", "#0969da", "#0969da2e", ["#24292f", "#cf222e", "#116329", "#7d4e00", "#0969da", "#8250df", "#1b7c83", "#6e7781", "#57606a", "#a40e26", "#1a7f37", "#633c01", "#218bff", "#a475f9", "#3192aa", "#8c959f"], { light: true }),
};

export const themeFor = (id) => THEMES[id] || THEMES.midnight;

const channels = (hex) => [1, 3, 5].map((start) => parseInt(hex.slice(start, start + 2), 16));
const mix = (from, to, amount) => `#${channels(from).map((value, index) => Math.round(value + (channels(to)[index] - value) * amount).toString(16).padStart(2, "0")).join("")}`;

export function contrast(first, second) {
  const luminance = (hex) => {
    const [red, green, blue] = channels(hex).map((value) => { const unit = value / 255; return unit <= 0.04045 ? unit / 12.92 : ((unit + 0.055) / 1.055) ** 2.4; });
    return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
  };
  const [light, dark] = [luminance(first), luminance(second)].sort((a, b) => b - a);
  return (light + 0.05) / (dark + 0.05);
}

// CSS custom properties for the interface around the terminals.
export function themeTokens(theme) {
  const { chrome: background, light = false, terminal } = theme;
  const text = theme.text || terminal.foreground;
  const accent = theme.accent || terminal.cursor;
  const danger = terminal.red;
  const warning = terminal.yellow;
  const rgb = (hex) => channels(hex).join(" ");
  return {
    "--bg": background,
    "--surface": mix(background, text, light ? 0.035 : 0.055),
    "--surface-soft": mix(background, text, light ? 0.02 : 0.03),
    "--surface-hover": mix(background, text, light ? 0.08 : 0.11),
    "--well": mix(background, light ? "#ffffff" : "#000000", light ? 0.6 : 0.16),
    "--text": text,
    "--text-soft": mix(text, background, 0.14),
    "--muted": mix(text, background, light ? 0.24 : 0.34),
    "--faint": mix(text, background, light ? 0.38 : 0.46),
    "--subtle": mix(text, background, 0.6),
    "--ink": light ? "0 0 0" : "255 255 255",
    "--accent": accent,
    "--accent-rgb": rgb(accent),
    "--on-accent": contrast(accent, "#ffffff") >= contrast(accent, "#10151c") ? "#ffffff" : "#10151c",
    "--danger": danger,
    "--danger-rgb": rgb(danger),
    "--warning": warning,
    "--warning-rgb": rgb(warning),
    "color-scheme": light ? "light" : "dark",
  };
}

// Keeps the color's RGB at zero alpha: xterm draws inverse text with it.
export const terminalTheme = (theme, clear) => clear ? { ...theme.terminal, background: `${theme.terminal.background}00` } : theme.terminal;

const clamp = (value, minimum, maximum) => Math.min(maximum, Math.max(minimum, Number(value) || minimum));

// The stack behind the interface, bottom to top: the desktop (only in a transparent
// window), the theme color at `base`, then the optional image. `clear` means
// something is worth seeing through the terminals; opaque terminals are kept
// otherwise because transparent ones lose subpixel text rendering.
export function backdrop(settings, windowTransparent) {
  const base = !windowTransparent || settings.backgroundMode === "opaque" ? 1 : settings.backgroundMode === "transparent" ? 0 : clamp(settings.backgroundOpacity, 0.1, 0.95);
  const image = settings.backgroundImage?.url || "";
  const clear = base < 1 || Boolean(image);
  return { base, image, imageOpacity: clamp(settings.backgroundImageOpacity, 0.05, 1), clear, panelAlpha: clear ? clamp(base, 0.6, 0.85) : 1 };
}

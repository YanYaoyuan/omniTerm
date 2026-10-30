// Built-in terminal color schemes. `chrome` sets the terminal surface around
// xterm's canvas so padding and split gutters match the scheme.

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
};

export const themeFor = (id) => THEMES[id] || THEMES.midnight;

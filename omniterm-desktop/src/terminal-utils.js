// Pure helpers for terminal views, kept free of JSX so tests can import them.

// OSC 7 reports the shell's working directory as file://host/path.
export function cwdFromOsc7(value, platform) {
  try {
    const url = new URL(value);
    if (url.protocol !== "file:") return null;
    let pathname = decodeURIComponent(url.pathname);
    if (platform === "win32" && /^\/[A-Za-z]:/.test(pathname)) pathname = pathname.slice(1).replaceAll("/", "\\");
    return pathname || null;
  } catch { return null; }
}

export function quotePath(filename, platform) {
  if (platform === "win32") return /[\s&()^;,]/.test(filename) ? `"${filename}"` : filename;
  return /^[\w@%+=:,./-]+$/.test(filename) ? filename : `'${filename.replaceAll("'", "'\\''")}'`;
}

// Subsequence match with bonuses for word starts and consecutive characters.
export function fuzzyScore(query, text) {
  if (!query) return 1;
  const needle = query.toLowerCase();
  const haystack = text.toLowerCase();
  let score = 0;
  let position = -1;
  let streak = 0;
  for (const character of needle) {
    const next = haystack.indexOf(character, position + 1);
    if (next < 0) return 0;
    streak = next === position + 1 ? streak + 1 : 0;
    const wordStart = next === 0 || /[\s·/_-]/.test(haystack[next - 1]);
    score += 1 + streak * 2 + (wordStart ? 3 : 0) - Math.min(next - position - 1, 5) * 0.1;
    position = next;
  }
  return score;
}

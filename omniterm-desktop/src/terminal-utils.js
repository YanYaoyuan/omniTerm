// Pure helpers for terminal views, kept free of JSX so tests can import them.

const shortHost = (value) => value.toLowerCase().split(".")[0];

// OSC 7 reports the shell's working directory as file://host/path. A report from
// another host (an ssh command inside a local shell) is not a local directory.
export function cwdFromOsc7(value, platform, localHost) {
  try {
    const url = new URL(value);
    if (url.protocol !== "file:") return null;
    if (url.hostname && url.hostname !== "localhost" && localHost && shortHost(url.hostname) !== shortHost(localHost)) return null;
    let pathname = decodeURIComponent(url.pathname);
    if (platform === "win32" && /^\/[A-Za-z]:/.test(pathname)) pathname = pathname.slice(1).replaceAll("/", "\\");
    return pathname || null;
  } catch { return null; }
}

export function quotePath(filename, platform, powershell = false) {
  // PowerShell expands $() and backticks inside double quotes; single quotes are literal.
  if (platform === "win32" && powershell) return /^[\w:\\.\/-]+$/.test(filename) ? filename : `'${filename.replace(/['\u2018\u2019\u201a\u201b]/g, "$&$&")}'`;
  if (platform === "win32") return /[\s&()^;,]/.test(filename) ? `"${filename}"` : filename;
  return /^[\w@%+=:,./-]+$/.test(filename) ? filename : `'${filename.replaceAll("'", "'\\''")}'`;
}

// xterm's onData mixes keystrokes with replies the terminal generates itself.
const TERMINAL_REPLY = /^(?:\x1b\[(?:\??\d*(?:;\d+)+R|\??\d*n|[?>][\d;]*c|\??[\d;]*\$y|\d+;[\d;]*t|[IO]|<\d+;\d+;\d+[Mm]|M[\s\S]{3})|\x1b\][\s\S]*?(?:\x07|\x1b\\)|\x1b[P_^][\s\S]*?\x1b\\)+$/;
export const isTerminalReply = (data) => TERMINAL_REPLY.test(data);

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

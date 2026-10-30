import { stripVTControlCharacters } from "node:util";

// Readline and ConPTY can insert terminal controls between command echo and
// output. A complete line keeps the echo from satisfying a command assertion.
export function hasOutputLine(output, line) {
  return stripVTControlCharacters(output).replaceAll("\r", "").split("\n").includes(line);
}

// Construct markers in the shell so their full value is absent from its echo,
// including when the echoed command wraps across terminal lines.
export function printParts(parts, windows) {
  return windows
    ? `Write-Output (${parts.map((part) => `'${part}'`).join(" + ")})`
    : `printf '${"%s".repeat(parts.length)}\\n' ${parts.map((part) => `'${part}'`).join(" ")}`;
}

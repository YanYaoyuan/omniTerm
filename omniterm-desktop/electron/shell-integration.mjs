// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0

// Prompt and command marks (OSC 133) for bash and zsh, typed into a shell the user chose and
// submitted by them. The nonce separates these marks from any that a program merely prints.
// Bash before 4.4 has no PS0, so it could mark a prompt but never a running command, and a
// builtin like `read` would look idle. Such a shell gets no marks and stays manual.
export function shellIntegrationScript(nonce) {
  if (!/^[0-9a-f]{32}$/.test(nonce)) throw new Error("Invalid shell integration nonce.");
  const prompt = `__omniterm_prompt() { local s=$?; printf '\\033]133;D;%s;k=${nonce}\\007\\033]133;A;k=${nonce}\\007' "$s"; return $s; }`;
  const start = `__omniterm_start() { printf '\\033]133;C;k=${nonce}\\007'; }`;
  const zsh = "precmd_functions+=(__omniterm_prompt); preexec_functions+=(__omniterm_start)";
  const bash = `PROMPT_COMMAND="__omniterm_prompt\${PROMPT_COMMAND:+;$PROMPT_COMMAND}"; PS0="$(__omniterm_start)$PS0"`;
  const refused = "echo 'omniTerm: readiness marks need bash 4.4+ or zsh; commands stay manual here.'";
  return ` ${prompt}; ${start}; if [ -n "$ZSH_VERSION" ]; then ${zsh}; else case "$BASH_VERSION" in [5-9].*|[1-9][0-9].*|4.[4-9]*) ${bash};; *) ${refused};; esac; fi`;
}

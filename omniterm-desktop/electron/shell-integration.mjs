// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0

// Prompt and command marks (OSC 133) for bash and zsh, typed into a shell the user chose and
// submitted by them. The nonce separates these marks from any that a program merely prints.
// Bash before 4.4 has no PS0, so it could mark a prompt but never a running command, and a
// builtin like `read` would look idle. Such a shell gets no marks and stays manual.
//
// The prompt mark reports whether input is queued on the terminal (`q=0` means none). Only the
// shell can see that, and it decides whether the next line is a command or an unfinished one.
// Each probe counts anything but a clean "nothing to read" as queued, so a failure stays manual.
//
// Dispatch handshake: 16 key sequences reach a line-editor widget that reports whether the line
// is empty. They are separate keys because a widget cannot reliably read an ID after a key, and
// the key itself is what tells one probe's reply from another's. Only emacs keymaps are bound,
// and `h=1` says so; zsh's `emacs` option doesn't track the keymap, so it asks bindkey instead.
// The PS2 mark makes continuation prompts visible for the same purpose.
const PROBE_KEYS = Array.from({ length: 16 }, (_, index) => String(index).padStart(2, "0")).join(" ");

export function shellIntegrationScript(nonce) {
  if (!/^[0-9a-f]{32}$/.test(nonce)) throw new Error("Invalid shell integration nonce.");
  const prompt = `__omniterm_prompt() { local s=$? q=0 h=0; __omniterm_queued; [ $? -eq 1 ] || q=1; __omniterm_editable && h=1; printf '\\033]133;D;%s;k=${nonce}\\007\\033]133;A;k=${nonce};q=%s;h=%s\\007' "$s" "$q" "$h"; return $s; }`;
  const start = `__omniterm_start() { printf '\\033]133;C;k=${nonce}\\007'; }`;
  const continuation = `\\033]133;P;k=${nonce}\\007`.replaceAll("\\033", "\\e").replaceAll("\\007", "\\a");
  const zshProbe = `__omniterm_probe() { local e=0 p=0 i=\${KEYS#*579}; [ -z "$BUFFER$PREBUFFER" ] && e=1; [ "$CONTEXT" = start ] && p=1; printf '\\033]133;R;k=${nonce};i=%s;e=%s;p=%s\\007' "\${i%\\~}" "$e" "$p" >/dev/tty; }`;
  const zsh = `zmodload zsh/zselect 2>/dev/null; __omniterm_queued() { zselect -t 0 -r 0 >/dev/null 2>&1; }; __omniterm_editable() { [[ "$(bindkey -lL main)" == *" emacs "* ]]; }; ${zshProbe}; zle -N __omniterm_probe; for i in ${PROBE_KEYS}; do bindkey "\\e[579$i~" __omniterm_probe; done; PS2=$'%{${continuation}%}'"$PS2"; precmd_functions+=(__omniterm_prompt); preexec_functions+=(__omniterm_start)`;
  // Bash has no way to ask if this is a secondary prompt, so `p=1` is constant here. The PS2
  // mark always precedes a reply in the stream and ends readiness first.
  const bashProbe = `__omniterm_probe() { local e=0; [ -z "$READLINE_LINE" ] && e=1; printf '\\033]133;R;k=${nonce};i=%s;e=%s;p=1\\007' "$1" "$e"; }`;
  const bash = `__omniterm_queued() { read -t 0; }; __omniterm_editable() { [[ -o emacs ]]; }; ${bashProbe}; for i in ${PROBE_KEYS}; do bind -x "\\"\\\\e[579$i~\\": __omniterm_probe $i"; done; PS2=$'\\\\[${continuation}\\\\]'"$PS2"; PROMPT_COMMAND="__omniterm_prompt\${PROMPT_COMMAND:+;$PROMPT_COMMAND}"; PS0="$(__omniterm_start)$PS0"`;
  const refused = "echo 'omniTerm: readiness marks need bash 4.4+ or zsh; commands stay manual here.'";
  return ` ${prompt}; ${start}; if [ -n "$ZSH_VERSION" ]; then ${zsh}; else case "$BASH_VERSION" in [5-9].*|[1-9][0-9].*|4.[4-9]*) ${bash};; *) ${refused};; esac; fi`;
}

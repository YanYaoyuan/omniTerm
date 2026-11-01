# omniTerm Agent follow-ups

Scoped follow-ups to [#4](https://github.com/YanYaoyuan/omniTerm/issues/4), from manual GUI testing of PR #5. **None of this is implemented or part of PR #5.** Each section can become its own issue.

## 1. Proposal: reliable approved execution over SSH

Status: **implemented behind a per-terminal opt-in, off by default** (branch `feat/ssh-approved-execution`). Without the opt-in, SSH commands stay staged for a manual Enter.

Decisions from review: no probe into a full-screen program or password prompt; unsupported shells or keymaps, nested sessions, failed or stale handshakes, disconnects and uncertain state stage or block, and never submit. Only emacs keymaps are bound in this version, and vi falls back to staging.

The one-time ID in step 1 is the key sequence itself: the snippet binds 16 sequences and the host cycles through them, because a widget can't reliably read bytes after its key once the line editor has buffered them.

| Choice | Behaviour |
| --- | --- |
| Approve and execute | After reliable confirmation that the shell is ready, type the approved command and submit it |
| Type only | Stage the command without Enter |
| Busy, full-screen, unfinished input or uncertain state | Block auto-submit and explain why |

### The problem

Signed prompt marks and the `q=0` input check say what the shell saw when it drew the prompt. Input that reaches the remote shell after that check is invisible, and on a slow link it can leave the shell at a continuation prompt, which fires no hooks. Timing windows can't close this, because latency is unbounded. The guard must ask the shell at the moment of dispatch.

### Proposed mechanism

**Report continuation prompts.** Add a signed mark to `PS2` as well (`%{...%}` in zsh, `\[...\]` in bash). Continuation prompts currently emit nothing, so this makes a silent state visible, even when input arrives late.

**Dispatch handshake, per command.** Immediately before submitting:

1. The host writes a private key sequence containing a one-time ID, bound to a line-editor widget by the integration snippet (`zle -N` in zsh, `bind -x` in bash).
2. The widget replies with a signed mark carrying the same ID, whether the edit buffer is empty, and whether this is the primary prompt (`$PREBUFFER`/`$CONTEXT` in zsh; the `PS2` mark in bash).
3. The host writes the command only after a matching reply, with no user input and no new output marks since sending the probe.
4. No reply within the timeout means block and report. It never falls back to submitting.

The widget runs inside the line editor, so a reply proves that the editor on that host was idle at that moment, regardless of latency. Nested `ssh`, `su` or tmux have no widget, so they produce no reply, which also answers which shell is really at the other end.

**Keep the existing guards.** Target and epoch pinning, interaction revision, approval scope and `q=0` all still apply. Without a successful handshake the command stays staged, so shells without the integration keep today's behaviour.

### Open questions

- The probe bytes reach whatever is reading the terminal. They are only sent once marks already say the prompt is ready, but if that were wrong a program would receive stray bytes. Is that acceptable, or should the probe need a very recent mark?
- Bash's `bind -x` depends on the active keymap, so vi mode needs handling or an explicit "unsupported, stays staged".
- Should this be a per-terminal opt-in, or is a working handshake enough?

### Regression tests

`npm run test:ssh` runs these against real `ssh` through an injected-latency proxy, at 0 and 250 ms, on bash and zsh. Checked items are covered. Removing the handshake fails the suite.

- [x] Each case from PR #5 over SSH: unclosed quote, line continuation, heredoc, queued lines, slow `preexec`, `read`.
- [x] Input landing after `q=0` was sent, which the current guard can't see.
- [x] Probe sent while input is in flight, and a reply lost or delayed.
- [ ] Nested `ssh`, `su` and tmux. A nested `/bin/sh` is covered; the others have only the same `busy` path so far.
- [x] Full-screen program and password prompt: no probe is sent.
- [x] Disconnect between probe and reply: no replay.
- [x] A forged reply without the session nonce is rejected (unit test).
- [x] Mutation check: removing the handshake fails the suite.

## 2. Authorized interaction with programs running in a terminal

Status: **implemented** (branch `feat/terminal-program-input`). The original notes follow.

The Agent only runs shell commands at a shell prompt. Asked to send instructions to another Agent in a terminal, the full-screen guard correctly blocked it. PR #5 states this limit on the card and tells the model, but the capability doesn't exist.

Sending text to an interactive program is a different operation from executing a shell command, and needs:

- A separate operation type, approval card and permission.
- Explicit selection of the target program, with the card showing exactly what will be sent.
- A way to confirm which program is receiving the input, including in nested sessions.
- Rules for submit keys, bracketed paste and multi-line input per program.
- Password prompts stay excluded.
- A hosted Agent's output stays untrusted data, and can't grant permissions or change targets.

## 3. Persistent monitoring and authorized automatic actions

Status: **first increment implemented** (branch `feat/persistent-monitoring`).

| Requirement | Status |
| --- | --- |
| Process, thread, log and shared-memory targets on a selected host | Implemented. Threads and shared memory are Linux only; Windows local hosts aren't supported |
| Continuous sampling and notifications, with no message per sample | Implemented, with no model calls |
| Automatic actions with a scope, trigger, duration and run limit | Implemented: one exact command on one pinned terminal, through the broker |
| Pause, stop, revoke, history | Implemented |
| Read access separate from modification | Implemented. Shared memory is read-only; **modification isn't implemented** and needs its own authorization |
| The Agent reacting to a trigger with a model turn | **Outstanding.** A trigger notifies you and runs the approved command; the Agent reads status when asked |
| Monitors surviving an app restart | **Outstanding.** They stop with the app, in line with approvals not surviving a restart |
| System V shared memory | **Outstanding** |

A persistent task watching a process, thread, log or shared-memory region, rather than a one-time snapshot or a chat message per observation. This stays separate from scraping a terminal screen.

- Explicit selection of the host and monitoring target.
- Ongoing collection, visible status, and proactive notification when a condition is met.
- Automatic actions within a user-approved scope and trigger conditions, with no manual Enter each time.
- Separate read and write permissions, pause and stop controls, revocation, and an operation history.
- For shared memory: an explicit region or name, layout and access method. Modification needs its own authorization.

Design questions: where collection runs and how it survives disconnects, sleep and restarts; how unattended actions fit the broker's rule that approvals don't survive a restart; rate and cost limits for model calls. Automatic remote actions depend on section 1.

## 4. Visual process-monitor window

Status: **implemented** (branch `feat/process-monitor-window`). Thread-level and shared-memory views are not included. The original notes follow.

A separate window showing processes on the selected local or remote host.

- Process tree and list, with search and filtering.
- CPU and memory usage, with trends.
- Selecting a process for continued Agent monitoring (depends on section 3).
- Later: thread-level and shared-memory views.

`electron/remote-metrics.mjs` already collects host metrics and may be a starting point for remote hosts.

## 5. Draggable panels

- Drag a panel to the left, right, top or bottom of another to change the split layout.
- Move a panel into another tab.
- Preserve the resulting layout.
- Moving a panel preserves its live session rather than recreating the terminal.

Terminal views are already rendered flat and positioned from the layout tree, so a move can be a layout change without touching the xterm instance or session; `src/layout.js` needs a move operation. Workspace restore needs to handle the new layouts. Operations pin to session and epoch, so add a regression that a move can't affect execution targets.

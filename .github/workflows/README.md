# omniTerm workflows

The `omniterm-ci.yml` workflow has two jobs on Linux, macOS, and Windows:

- `terminal-cli` tests the optional command launcher with Node.js 22 and 24,
  including help without Codex, OpenSSH, or serial hardware installed.
- `desktop` restores the desktop's independent npm lockfile with Node.js 24,
  runs backend tests with mocked PTYs, and builds the Electron main process,
  preload, and React renderer.

The desktop job uses `npm ci --ignore-scripts` and sets
`ELECTRON_SKIP_BINARY_DOWNLOAD=1`. It skips Electron installation hooks and
node-pty's native build. It does not launch an Electron window, exercise a
real PTY, package an application, or test graphical interaction.

Neither job builds the inherited Codex Rust workspace or validates live SSH,
SFTP, serial-device, or authenticated Agent sessions. Native runtime and GUI
verification follow the
[desktop test plan](../../docs/omniterm/desktop-test-plan.md).

The original Codex workflows are preserved in
[`../upstream-workflows/`](../upstream-workflows/) as reference. They are outside
GitHub's active workflow directory because their release credentials, signing,
CLA rules, private infrastructure, and publishing destinations belong to the
upstream project. Do not move them back without adapting and reviewing those
dependencies. Shared upstream actions and scripts remain available for future
work. The original workflow strategy is in
[`../upstream-workflows/README.md`](../upstream-workflows/README.md).

Application signing, release distribution, and Rust-core CI remain separate
work from these source-level checks.

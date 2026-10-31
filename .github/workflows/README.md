# omniTerm workflows

The `omniterm-ci.yml` workflow has two jobs on Linux, macOS, and Windows:

- `terminal-cli` tests the optional command launcher with Node.js 22 and 24,
  including help without Codex, OpenSSH, or serial hardware installed.
- `desktop` restores the desktop's independent npm lockfile with Node.js 24,
  runs backend tests with a mocked terminal host, and builds the Electron main
  process, preload, and React renderer.
- `terminal-host` runs the Rust host's tests on all three platforms, builds it,
  and exercises a real PTY or ConPTY through it.
- `engine` tests the provider protocols and the dynamic-tools-only boundary, and
  type-checks the Agent binary on Ubuntu.

The desktop job uses `npm ci --ignore-scripts` and sets
`ELECTRON_SKIP_BINARY_DOWNLOAD=1`. It skips Electron installation hooks. It does not launch an Electron window,
package an application, or test graphical interaction.

No job validates live SSH, SFTP, serial-device, or authenticated Agent sessions. Native runtime and GUI
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

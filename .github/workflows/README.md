# omniTerm workflows

The `omniterm-ci.yml` workflow tests the omniTerm command launcher on Linux,
macOS, and Windows with Node.js 22 and 24. It also checks that the CLI help
works without Codex, OpenSSH, or serial hardware installed.

These tests cover the launcher. They do not build the inherited Codex Rust
workspace or validate live SSH, SFTP, serial-device, or Agent sessions.

The original Codex workflows are preserved in
[`../upstream-workflows/`](../upstream-workflows/) as reference. They are outside
GitHub's active workflow directory because their release credentials, signing,
CLA rules, private infrastructure, and publishing destinations belong to the
upstream project. Do not move them back without adapting and reviewing those
dependencies. Shared upstream actions and scripts remain available for future
work. The original workflow strategy is in
[`../upstream-workflows/README.md`](../upstream-workflows/README.md).

omniTerm releases and Rust-core CI will be introduced separately as the project
establishes its own build and distribution process.

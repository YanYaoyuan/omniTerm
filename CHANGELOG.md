# omniTerm changelog

## 0.2.0 — Native Preview

- Add the Rust native application with GPU terminal rendering, X11 forwarding and improved SFTP transfer handling.
- Add web-authorized SSH/SFTP asset windows and configurable terminal content highlighting.
- Let external coding agents bind existing terminals and SFTP pages, execute commands, read results and manage remote files through the bundled CLI.
- Publish the complete `omniterm-control` Skill separately, including its portable launcher and task-specific references.
- Fix external control discovery and transport behavior across Windows, macOS and Linux.

See [release notes](docs/omniterm/releases/v0.2.0.md) and [Skill installation](omniterm-cli/skills/README.md).

## 0.1.0

- Add the Rust terminal host and terminal-aware Agent with scoped context, command approval, cancellation and configurable providers.
- Add optional SSH approved execution, separate interactive-program input, monitoring while the app is running, and a process window.
- Add workspace docking for terminal, Agent, workspace and monitoring panels.
- Fix themed window chrome, provider settings save/close, clearer errors, monitor lifecycle races and rotated-log identity.
- Prepare native Windows x64, macOS arm64 and Ubuntu 22.04 amd64 packages with source manifests and SHA-256 checksums.

See [release notes](docs/omniterm/releases/v0.1.0.md) for acceptance scope and remaining limits.

## Upstream engine history

The changelog can be found on the [releases page](https://github.com/openai/codex/releases).

## Unreleased

- Fix missing command lifecycle events when unified exec cannot create a process, preserving approval rejection and sandbox retry behavior. Thanks @Marvinthebored ([#44557](https://github.com/openai/codex/issues/44557)).

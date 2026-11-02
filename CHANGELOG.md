# omniTerm changelog

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

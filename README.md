# omniTerm

**An open-source desktop terminal with a Codex Agent beside your sessions.**

[简体中文](README.zh-CN.md) · [Desktop guide](omniterm-desktop/README.md) · [Architecture](docs/omniterm/architecture.md) · [Roadmap](docs/omniterm/roadmap.md) · [Contributing](CONTRIBUTING.md)

omniTerm brings a graphical terminal workspace and remote connection tools together, taking inspiration from iTerm2 and MobaXterm and targeting **macOS, Linux, and Windows**. Its first Agent engine is built from the Codex Rust source in this repository.

**Status: early desktop implementation.** A local macOS arm64 application package has been built and opened with a visible shell prompt. Real PTY checks passed under Electron for command input, `pwd`, resize, Ctrl+C, and process cleanup. GUI keyboard/Agent interaction, live remote connections, and other operating systems remain unverified; this is not yet a validated three-platform release. See the [validation record](docs/omniterm/desktop-test-plan.md#验收记录).

## Desktop features

| Area | Current implementation |
| --- | --- |
| Local terminals | Independent shell tabs, folder selection, terminal search, copy/paste, and terminal display settings |
| Agent pane | A separate interactive Codex TUI alongside the terminal; keyboard input, approvals, and cancellation use the engine's existing interface |
| Connections | Graphical forms and saved profiles for SSH, SFTP, serial devices, and SSH forwarding |
| SSH / forwarding | System OpenSSH sessions with local, remote, or dynamic forwarding |
| SFTP | An interactive system SFTP client in a terminal tab |
| Serial | A terminal session running optional Python / pyserial miniterm |
| Local packaging | Packages the desktop app with the complete source-built Agent directory |

A graphical SFTP file browser, arbitrary terminal pane splitting, automatic session restoration, and structured Agent context sharing are still planned. The Agent does not automatically read other terminal tabs or acquire a remote SSH environment. Ordinary shell and connection sessions run with the user's permissions, separately from Codex's Agent sandbox and approval controls.

## Run the desktop app from source

Install **Node.js 22.12 or later**, npm, and the native build tools required by Electron/node-pty. Clone the repository:

```sh
git clone https://github.com/YanYaoyuan/omniTerm.git
cd omniTerm
```

Follow the [local Agent build guide](docs/omniterm/local-build.md) to compile Rust `codex` and its companion programs into `dist/omniterm-agent/`. The desktop app discovers this package during development and includes it when packaging. A local shell can be used without a model account; Agent use requires the engine's authentication and configuration.

Then start the desktop app:

```sh
cd omniterm-desktop
npm ci
npm run rebuild
npm start
```

`npm run rebuild` builds node-pty for Electron's native module ABI. Repeat it after changing Electron versions or platforms. The desktop has its own npm lockfile; its dependencies are separate from the inherited Codex pnpm workspace.

SSH, forwarding, and SFTP require system OpenSSH clients. Serial connections require Python 3 with `pyserial`. See the [desktop guide](omniterm-desktop/README.md) for platform prerequisites, controls, and packaging.

To build a local application package from `omniterm-desktop/`:

```sh
npm run package
```

Output is written to `dist/desktop/` at the repository root. The Agent package must match the application's platform and architecture. Signing, notarization, installers, and automatic updates are separate release work; no three-platform release is claimed here.

## Optional command-line entry

The lightweight CLI remains available for use in an existing terminal:

```sh
node omniterm-cli/bin/omniterm.js --help
node omniterm-cli/bin/omniterm.js doctor
node omniterm-cli/bin/omniterm.js agent --version
```

It provides `agent`, `shell`, `ssh`, `sftp`, `tunnel`, and `serial` commands. See the [CLI guide](omniterm-cli/README.md). No omniTerm npm package has been published.

## Development and verification

```sh
node --test omniterm-cli/test/cli.test.js
npm --prefix omniterm-desktop test
npm --prefix omniterm-desktop run build
```

CI is configured to test the CLI and build/test desktop code on macOS, Linux, and Windows. Desktop CI uses mocked backend tests and skips Electron downloads and native module build scripts. It does not launch the GUI, exercise real PTYs, build the Rust engine, or test remote servers and serial hardware. Use the [desktop interaction checklist](docs/omniterm/desktop-test-plan.md) for native runtime acceptance and record untested cases explicitly.

`omniterm-desktop/` contains the desktop app, `omniterm-cli/` the optional launcher, and `docs/omniterm/` the product documentation. The inherited `codex-rs/`, `codex-cli/`, and `sdk/` retain their internal names to keep upstream updates practical. Upstream organization-specific workflows are [archived](.github/upstream-workflows/README.md).

## License and origin

omniTerm uses the [Apache License 2.0](LICENSE). It is derived from [OpenAI Codex](https://github.com/openai/codex); original copyright and third-party notices are retained in [NOTICE](NOTICE) and the source tree. See [upstream provenance](docs/omniterm/upstream.md).

This is an independent project, not an official OpenAI, iTerm2, or MobaXterm product. These names describe the Agent foundation or product benchmarks, not an affiliation.

<!-- Modified for omniTerm from the upstream Codex README, 2026-10-30. -->

**Language / 语言: [English](README.md) · [简体中文](README.zh-CN.md)**

# omniTerm

**An open-source terminal workspace with a Codex Agent beside your local and remote sessions.**

[Download 0.0.1](https://github.com/YanYaoyuan/omniTerm/releases) · [Desktop guide](omniterm-desktop/README.md) · [Build the Agent](docs/omniterm/local-build.md) · [Report an issue](https://github.com/YanYaoyuan/omniTerm/issues) · [Contribute](CONTRIBUTING.md)

omniTerm brings shell tabs, split panes, SSH, a graphical SFTP browser, serial connections and port forwarding into one desktop app for **macOS, Linux and Windows**. A locally running, source-built Codex Agent shares the workspace. Select terminal output, send it to the Agent's input, and decide what to submit.

The project is **Apache-2.0 licensed** and at **version 0.0.1**. It combines familiar terminal and remote-operation workflows with an inspectable Agent implementation. This is an early release: platform packages and individual runtime checks should not be read as complete three-platform manual acceptance.

## Download

Get the available artifacts and their checksums from [GitHub Releases](https://github.com/YanYaoyuan/omniTerm/releases). The initial release targets are:

| System | Architecture | Package |
| --- | --- | --- |
| macOS | Apple Silicon / arm64 | `.zip` containing the app |
| Ubuntu 22.04 | amd64 / x86_64 | `.deb` |
| Windows | x64 | `.zip` |

Check each release's asset list and validation notes before downloading. The initial macOS and Windows packages do not have developer signing; use the operating system's normal per-app approval flow if prompted. An Intel Mac package is not part of this initial set.

Ubuntu users of the earlier `0.1.0~preview` trial package should install the downloaded `0.0.1` `.deb` explicitly: Debian version ordering considers this a downgrade. Settings and feature coverage are described in the [desktop guide](omniterm-desktop/README.md).

## What you can do today

| Workflow | Implemented in omniTerm |
| --- | --- |
| Local terminal workspace | Real PTY shells, tabs, nested horizontal/vertical splits, pane zoom, broadcast input, search, rename/reorder and a command palette |
| Agent collaboration | Dedicated Codex pane; starts in the selected local working directory; send selected terminal text into its draft without automatically submitting it |
| SSH connections | Saved profiles, import from `~/.ssh/config` including `Include`, aliases, and single/multiple `ProxyJump` jump hosts |
| Remote files | Graphical SFTP browser beside SSH/SFTP sessions; upload/download files and folders, rename, delete, create directories and staged drag-out downloads |
| Remote host metrics | CPU, memory, disk, network, load and uptime, plus GPU data when the remote host exposes supported tools/interfaces |
| Forwarding and devices | Local, remote and dynamic SSH forwarding; serial sessions through Python/pyserial |
| Resume your workspace | Restore local tabs, split layouts, tab names and working directories as new shell sessions |
| Appearance | 13 themes, custom background images, font/cursor controls and WebGL rendering with DOM fallback; transparency where the platform supports it |

The useful combination is **an open-source Agent and remote operations in the same workspace**: inspect a host, move a file, select an error, and ask Codex about it without switching terminal applications. SSH configuration continues to be interpreted by system OpenSSH, and the desktop and Agent sources can be inspected and extended together.

### Agent and platform boundaries

- **Local Agent process does not mean offline inference.** Model access follows Codex's authentication, provider and configuration; no model weights are bundled. Local shells do not require a model account.
- The Agent receives the text you explicitly send. It does not automatically read all tabs or inherit a remote SSH session. Its tool execution follows Codex's configured approvals and sandbox; ordinary shell, SSH, SFTP and serial sessions use your user permissions.
- Workspace restoration opens fresh local shells, up to 16 panes. It does not replay commands or restore scrollback, and remote/device/Agent sessions are not automatically reconnected.
- SSH/SFTP/forwarding require system OpenSSH. Serial requires Python 3 and `pyserial`. On Windows, the file/metrics panel uses a separate non-interactive SSH connection and needs key or SSH-agent authentication; macOS/Linux can reuse the terminal login through OpenSSH control sockets.
- Switching between opaque and transparent windows requires a restart. Linux needs a desktop compositor; native Windows window transparency is not supported by the current framed-window configuration. Running TUIs can retain their startup colors: after a light/dark theme change, restarting an idle Agent may be needed for readable input colors.

## How it compares

**Official sources checked on 2026-10-30.** This is a comparison of platforms, licensing and workflow focus, not a performance benchmark or a complete feature checklist. Features omitted from a row are not claimed to be absent.

| Product | Desktop platforms | Licensing | Workflow focus / Agent integration |
| --- | --- | --- | --- |
| **omniTerm 0.0.1** | **macOS, Linux, Windows** | **[Apache-2.0](LICENSE)** | **Local source-built Codex pane + SSH/SFTP browser, jump hosts, forwarding and serial in one workspace; early release** |
| [iTerm2](https://iterm2.com/index.html) | macOS | GPL v2 | Extensive [terminal, split-pane and shell integration](https://iterm2.com/features.html); [AI Chat](https://github.com/gnachman/iTerm2#key-features) with an [optional AI plugin](https://iterm2.com/ai-plugin.html) |
| [Xshell](https://www.netsarang.com/en/xshell-all-features/) | Windows | [Commercial license](https://www.netsarang.com/en/xshell/); [Home/School free license](https://netsarang.atlassian.net/wiki/spaces/ENSUP/pages/2027552778/Free+User+Registration) | SSH/SFTP/serial, session management, jump hosts, scripting and remote file management |
| [MobaXterm](https://mobaxterm.mobatek.net/features.html) | Windows | [Home/Professional editions](https://mobaxterm.mobatek.net/download.html), [product EULA](https://mobaxterm.mobatek.net/license.html) | Integrated SFTP, SSH gateways, serial, X11 server and RDP/VNC remote-desktop tools |
| [Warp](https://www.warp.dev/download) | macOS, Linux, Windows | [Open-source client: AGPL v3; UI framework: MIT](https://github.com/warpdotdev/warp#licensing) | Built-in coding agent and support for external CLI agents, including Codex; agent-focused development environment |
| [Tabby](https://github.com/Eugeny/tabby) | macOS, Linux, Windows | [MIT](https://github.com/Eugeny/tabby/blob/master/LICENSE) | SSH/serial, jump hosts and nested splits; plugin ecosystem includes SFTP and [MCP-based AI integration](https://github.com/Eugeny/tabby#plugins) |

Choose omniTerm when you want its **Apache-2.0 codebase, bundled Codex foundation and combined terminal/remote workflow**. iTerm2, Xshell and MobaXterm remain important references for terminal depth and operations tooling; omniTerm does not yet match their full feature sets. It does not bundle an X11 server, RDP/VNC clients or an enterprise credential manager. AI, open source and split panes are shared capabilities across this ecosystem, not exclusive omniTerm claims.

## Technology stack

| Layer | Technology | Role |
| --- | --- | --- |
| Desktop | Electron 45 | Native windows, packaging and isolated preload IPC |
| Interface | React 19, CSS | Tabs, pane layouts, connection forms, file browser and settings |
| Terminal | xterm.js 6 + WebGL/fit/search/Unicode addons | Terminal emulation, rendering, resizing and search |
| Processes | node-pty 1.1 | Real Unix PTYs / Windows ConPTY |
| Remote access | System OpenSSH + SFTP protocol | SSH configuration, jump hosts, forwarding and remote files |
| Agent | Rust Codex + code-mode host | Locally running Agent engine and execution support, packaged from this repository |
| Serial | Python 3 + pyserial miniterm | Optional serial-device sessions |
| Build | Node.js ≥22.12, npm, Vite, esbuild, Cargo | Desktop build and native Agent compilation |

The renderer uses `contextIsolation: true`, `nodeIntegration: false` and Electron's renderer sandbox. These controls separate the UI from privileged process APIs; they do not sandbox commands you run in a normal terminal tab.

## Run from source

Install Node.js **22.12+**, npm and your platform's native build tools: Xcode Command Line Tools on macOS, a C/C++ toolchain on Linux, or MSVC Build Tools and Windows SDK on Windows.

```sh
git clone https://github.com/YanYaoyuan/omniTerm.git
cd omniTerm
```

Use the [Agent build guide](docs/omniterm/local-build.md) to compile `codex` and its companions into `dist/omniterm-agent/`. Then start the desktop:

```sh
cd omniterm-desktop
npm ci
npm run rebuild
npm start
```

`npm run rebuild` rebuilds node-pty for Electron's ABI. Repeat it after changing Electron or the target platform. A local shell can run without an Agent package; the Agent pane and application packaging need a compatible Agent build.

```sh
# From omniterm-desktop/: create a native application package
npm run package
```

Output goes to `dist/desktop/` at the repository root. Electron, node-pty and the Agent must match the target OS and architecture. See the [desktop guide](omniterm-desktop/README.md) for controls and packaging, and the [architecture](docs/omniterm/architecture.md) for component boundaries.

## Development and validation

```sh
node --test omniterm-cli/test/cli.test.js
npm --prefix omniterm-desktop test
npm --prefix omniterm-desktop run build
```

The repository has three-platform code checks and separate native runtime/packaging checks. macOS GUI checks and Ubuntu installation, PTY and Electron checks have been performed for specific builds. Automated IPC input is not equivalent to keyboard/IME testing, and serial hardware, remote-server variants and Windows manual coverage still require dedicated acceptance. Use the [interaction checklist](docs/omniterm/desktop-test-plan.md) and the validation notes attached to each release.

The optional [CLI launcher](omniterm-cli/README.md) provides `agent`, `shell`, `ssh`, `sftp`, `tunnel`, `serial` and `doctor` in an existing terminal:

```sh
node omniterm-cli/bin/omniterm.js doctor
node omniterm-cli/bin/omniterm.js agent --version
```

Desktop code lives in `omniterm-desktop/`; the launcher is in `omniterm-cli/`. Inherited `codex-rs/`, `codex-cli/` and `sdk/` names are retained to make upstream updates practical. Contributions and reproducible bug reports are welcome through [Issues](https://github.com/YanYaoyuan/omniTerm/issues) and pull requests.

## License and origin

omniTerm uses [Apache-2.0](LICENSE) and is derived from [OpenAI Codex](https://github.com/openai/codex). Original copyright and third-party notices are retained in [NOTICE](NOTICE) and distributed dependency notices; third-party components retain their own licenses. See [upstream provenance](docs/omniterm/upstream.md).

omniTerm is an independent project. Product names in the comparison identify their respective products and do not imply affiliation or endorsement.

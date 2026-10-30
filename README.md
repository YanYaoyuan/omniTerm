# omniTerm

**An open-source, cross-platform terminal workspace with an Agent at its core.**

[简体中文](README.zh-CN.md) · [Architecture](docs/omniterm/architecture.md) · [Roadmap](docs/omniterm/roadmap.md) · [Contributing](CONTRIBUTING.md)

omniTerm aims to bring the terminal experience of iTerm2 and the remote-work toolbox of MobaXterm to **macOS, Linux, and Windows**, with a Codex-based Agent working alongside your sessions.

**Status: early CLI prototype.** This repository contains the Codex source foundation and an omniTerm command launcher. It uses your existing terminal and installed tools. A desktop terminal emulator, tabs, split panes, connection manager, and graphical SFTP browser are planned; they are not shipped yet.

## What works today

| Command           | Initial implementation                                                              |
| ----------------- | ----------------------------------------------------------------------------------- |
| `omniterm agent`  | Launch Codex with its existing authentication, configuration, and approval controls |
| `omniterm shell`  | Start a local shell in the current terminal                                         |
| `omniterm ssh`    | Launch the system OpenSSH client                                                    |
| `omniterm tunnel` | SSH local, remote, and dynamic forwarding (`-L`, `-R`, `-D`)                        |
| `omniterm sftp`   | Start the system SFTP client                                                        |
| `omniterm serial` | Serial console through optional Python `pyserial` miniterm                          |
| `omniterm doctor` | Check locally available dependencies                                                |

These commands are separate sessions. Agent access to SSH or serial session context is a future integration. Shell, SSH, SFTP, and serial commands are ordinary local tools and are not routed through the Agent's sandbox or approval system.

## Try the prototype

Install **Node.js 22 or later**, then:

```sh
git clone https://github.com/YanYaoyuan/omniTerm.git
cd omniTerm
node omniterm-cli/bin/omniterm.js --help
node omniterm-cli/bin/omniterm.js doctor

# Optional: install the command from local source
npm install -g ./omniterm-cli
omniterm --help
```

No omniTerm npm package or desktop installer has been published yet. The launcher has no npm runtime dependencies. Install the tools you need:

- **Agent:** build the Rust Agent and its companion programs from this checkout using the [local source-build guide](docs/omniterm/local-build.md). The launcher discovers the resulting `dist/omniterm-agent/` package. An existing Codex executable can also be selected with `OMNITERM_CODEX_BIN`; see the [launcher guide](omniterm-cli/README.md).
- **SSH, tunnels, SFTP:** system OpenSSH clients. Windows offers an OpenSSH Client optional feature.
- **Serial:** Python 3 with `pyserial` installed in the selected Python environment; see the launcher guide.

```sh
omniterm agent "Explain this repository"
omniterm shell
omniterm ssh user@example.com
omniterm tunnel -L 127.0.0.1:8080:localhost:80 user@example.com
omniterm tunnel -D 127.0.0.1:1080 user@example.com
omniterm sftp user@example.com
omniterm serial /dev/ttyUSB0 115200
# Windows: omniterm serial COM3 115200
```

## Product direction

1. **Terminal workspace:** desktop distribution for macOS, Linux, and Windows; tabs, panes, search, profiles, and PTY/ConPTY sessions.
2. **Agent mode:** conversation alongside the active terminal, explicit context sharing, visible execution targets, approvals, and execution history, initially using the Codex app-server protocol.
3. **Remote toolbox:** saved SSH connections, jump hosts, forwarding management, SFTP transfers, and serial profiles.

See the [roadmap and acceptance criteria](docs/omniterm/roadmap.md).

## Development

```sh
node --test omniterm-cli/test/cli.test.js
```

`codex-rs/`, `codex-cli/`, and `sdk/` retain the inherited Agent foundation. Internal `codex-*` names and protocols remain intact to make upstream updates practical. New code lives in `omniterm-cli/`; product design lives in `docs/omniterm/`.

The new CI checks the launcher on macOS, Linux, and Windows. It does not certify every Codex feature, remote server, or serial device. Upstream infrastructure and release workflows are [archived](.github/upstream-workflows/README.md); the full inherited Rust workspace is not covered by this initial CI.

## License and origin

omniTerm uses the [Apache License 2.0](LICENSE). It is derived from [OpenAI Codex](https://github.com/openai/codex); original copyright and third-party notices are retained in [NOTICE](NOTICE) and the source tree. See [upstream provenance](docs/omniterm/upstream.md).

This is an independent project, not an official OpenAI, iTerm2, or MobaXterm product. These names describe the Agent foundation or product benchmarks, not an affiliation.

<!-- Modified for omniTerm from the upstream Codex README, 2026-10-30. -->

# omniTerm CLI

An initial command launcher for omniTerm: Codex Agent, local shells, SSH, SSH port forwarding, SFTP, and serial consoles. It runs inside your existing terminal. Desktop tabs, split panes, graphical file transfer, and integrated remote Agent sessions are planned; they are not included in this CLI.

## Run from source

Requires Node.js 22 or later. This package has no npm runtime dependencies.

```sh
node omniterm-cli/bin/omniterm.js --help
node omniterm-cli/bin/omniterm.js doctor
```

Optionally install the local command from the repository root:

```sh
npm install -g ./omniterm-cli
omniterm --help
```

The package is private and has not been published to npm. A global install may require configuring npm's user prefix on your machine.

## Commands

```sh
# Codex Agent: arguments are passed through to Codex.
omniterm agent
omniterm agent exec "explain this repository"
omniterm agent --help

# Local shell. With no arguments, starts an interactive shell.
omniterm shell

# Native OpenSSH options, SSH config aliases, identities, and jump hosts work.
omniterm ssh -J bastion user@server
omniterm sftp user@server

# Local, remote, or SOCKS forwarding; Ctrl+C stops the foreground tunnel.
omniterm tunnel -L 8080:localhost:80 user@server
omniterm tunnel -R 9090:localhost:3000 user@server
omniterm tunnel -D 1080 user@server

# Python pyserial's miniterm: device and baud rate.
omniterm serial /dev/ttyUSB0 115200
# Windows:
omniterm serial COM3 115200
```

`tunnel` adds `ssh -N -o ExitOnForwardFailure=yes` and passes your remaining arguments to OpenSSH. Use OpenSSH's normal host-key verification and authentication. It does not store credentials or disable host-key checks. `sftp` opens the native interactive SFTP client; file-transfer commands are entered there. SSH and SFTP remain separate from the Agent's local session.

`serial` uses pyserial's miniterm keyboard controls; **Ctrl+]** exits by default. Use `omniterm serial --help` for miniterm options after installing its dependencies.

## Dependencies and platforms

| Capability              | Dependency                                                                         |
| ----------------------- | ---------------------------------------------------------------------------------- |
| Agent                   | Codex executable built from this repository or an existing Codex installation      |
| Shell                   | `$SHELL` or `/bin/sh` on macOS/Linux; PowerShell or `COMSPEC`/`cmd.exe` on Windows |
| SSH / forwarding / SFTP | OpenSSH client (`ssh`, `sftp`) on `PATH`                                           |
| Serial                  | Python 3 and `pyserial` installed in that interpreter's environment                |

On Windows, enable/install the OpenSSH Client feature if needed; native executable availability depends on the Windows installation. Unix shells, PowerShell, Python, and OpenSSH are external programs, not bundled with omniTerm.

Install pyserial in your chosen Python 3 environment, for example `python3 -m pip install pyserial` or `py -3 -m pip install pyserial` on Windows. If your Python installation requires a virtual environment, activate one first. The launcher never installs dependencies automatically.

On Windows, serial connections prefer `VIRTUAL_ENV\Scripts\python.exe`, then `python` or `python3` on `PATH`, and finally `py -3`. On macOS/Linux, they use `python3` or `python` on `PATH`. Install pyserial using the interpreter selected by `doctor` so the launcher can find it.

`doctor` checks dependencies locally and exits with status `1` if any capability is unavailable. A missing optional dependency only prevents that capability from working. It does not test remote hosts, authentication, serial hardware, or a model connection.

### Selecting the Agent

The launcher searches in this order:

1. `OMNITERM_CODEX_BIN`, when set (a single executable path or Codex's `.js` entrypoint).
2. This checkout's source-built package at `dist/omniterm-agent/bin/codex` (`codex.exe` on Windows).
3. This checkout's raw `codex-rs/target/debug/codex` then `codex-rs/target/release/codex` (`.exe` on Windows).
4. `codex` on `PATH` (including the standard global npm JS entrypoint on Windows).

After building and packaging the Rust Agent into `dist/omniterm-agent`, the launcher discovers it without any Codex installation on `PATH`. Keep that package directory intact, including its auxiliary executables and bundled tools. `scripts/build_codex_package.py` assembles the package from source-built artifacts; the Node launcher itself does not compile or download the Rust engine. Run `omniterm doctor` to check which executable was selected, then `omniterm agent --version` to verify that executable starts. This lookup support does not by itself establish that a Rust build or an authenticated Agent session has passed.

For custom Cargo target directories or a separately installed launcher, set `OMNITERM_CODEX_BIN` explicitly. It is a path, not a command string containing arguments. Windows `.cmd`/`.bat` overrides are rejected; point to the native executable or `@openai/codex/bin/codex.js` instead. Agent authentication, model configuration, approvals, and sandbox behavior remain those of the selected Codex installation, including its existing Codex home/configuration.

```sh
OMNITERM_CODEX_BIN=/path/to/codex omniterm agent
```

```powershell
$env:OMNITERM_CODEX_BIN = 'C:\path\to\codex.exe'
omniterm agent
```

## Development

```sh
node --test omniterm-cli/test/cli.test.js
```

Tests cover argument boundaries, executable selection, Windows command handling, dependency failures, exit codes, and signal forwarding. On Windows, Ctrl+C is delivered by the shared console, allowing the child shell or Agent to handle it; the launcher does not force-kill it on that event. Hosted CI does not provide full interactive-console coverage. Real Ctrl+C behavior, SSH/SFTP hosts, serial devices, and Agent authentication need interactive testing on each target platform before a product release.

Licensed under Apache-2.0; see [LICENSE](LICENSE) and [NOTICE](NOTICE).

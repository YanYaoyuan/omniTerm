# omniTerm CLI

A command launcher and external-agent interface for omniTerm. `session` controls terminals already open in the **native desktop app**; the other commands launch Codex Agent, local shells, SSH, forwarding, SFTP, and serial consoles in your existing terminal.

## 操作窗口里已经连接的资产（PR21 原生版）

需要包含外部接入通道的新原生构建。此前安装的 PR21 二进制没有此功能，需更新并重新打开窗口；Electron 版暂不提供这个入口。CLI 不重新登录资产，也不读取已保存的 SSH 密码或网页会话票据。

```sh
# 先启动新版桌面，连接资产；再在外部智能体所在终端执行：
node omniterm-cli/bin/omniterm.js session instances
node omniterm-cli/bin/omniterm.js session list
```

安装本地 CLI 后可直接使用 `omniterm`（安装方式见下方）。分发包中的 CLI 位于 `omniterm-cli/`，macOS 位于 App 的 `Contents/Resources/omniterm-cli/`；也可使用 `node <目录>/bin/omniterm.js`。需要 Node.js 22+。`omniterm ssh` 会新建连接；复用窗口连接应使用 `omniterm session`。

清单返回标签、初始连接信息、焦点、交互状态及 `ref: {sessionId, epoch}`。多实例时必须明确 `--instance INSTANCE_ID`，并在后续调用中保持它。以下占位符替换成清单中的实际值：

```sh
omniterm session read SESSION_ID --epoch EPOCH --lines 200

# 仅在已确认空的 bash 4.4+ / zsh 主提示符上运行一次。
# 脚本启用当前 shell 的就绪标记与探测键，SSH 同时开启该会话自动提交。
omniterm session setup SESSION_ID --epoch EPOCH --shell bash
omniterm session list

# 等 interaction.state=ready 且 autoSubmit=true 后执行。
omniterm session exec SESSION_ID --epoch EPOCH \
  --request-id inspect-001 --command 'hostname; id; pwd'

# 文件传入复杂脚本，避免本机 shell 提前展开变量。
omniterm session exec SESSION_ID --epoch EPOCH \
  --request-id inspect-002 --command-file ./inspect.sh --wait-ms 0
omniterm session result inspect-002 --wait-ms 30000
omniterm session cancel inspect-002

# 增量读取：LAST_SEQ_END 来自上次 read。
omniterm session read SESSION_ID --epoch EPOCH --since LAST_SEQ_END --lines 500

# 显式的键盘输入。--enter 提交；无该参数只输入文本。
omniterm session write SESSION_ID --epoch EPOCH --text 'y' --enter
```

`exec` 复用现有操作队列、连接代次绑定、SSH 行编辑器握手和 shell 完成信号，不依赖“安静几秒”或提示符文本猜测完成。`setup` 需要 emacs 键位，只修改当前 shell，不写启动配置；不要在密码提示、运行中的程序、未提交输入或续行提示符下调用。其他 shell 可读取和发送显式输入，但不能获得相同的完成保证。操作本机控制通道相当于使用当前用户的键盘权限；外部 CLI 的明确 exec 调用会授权这一条命令，无需再点内置 Agent 的批准卡片。

返回 JSON 包括 `instance`、`requestId`、`operation` 与 `output`；`completed` 才能说明收到命令完成信号。命令输出是合并的 PTY 文本（含可能的回显/提示符），不是分别捕获的 stdout/stderr。默认返回最近 200 行；`read --lines` 最多 2000 行。检查 `omittedLines`、`missingBytes` 和 read 的 `truncated`，缓冲区有上限且沿用现有脱敏规则。

CLI 退出码：`0` 成功/读/发送成功，`1` 错误或被阻止/取消，`2` 尚未结束或结果未知，`3` 命令完成但远端退出码非零。`--wait-ms` 默认 30000，`0` 立即返回；它只控制 CLI 等待。`--timeout-ms` 默认 120000，范围 1000–900000，超时不保证停止远端进程。`cancel` 尝试中断原操作所属终端。

每条新命令使用唯一 request-id。断开或 `outcomeUnknown` 后先查询同一 ID；相同 exec 参数和 ID 返回原操作，参数不同则拒绝。不要因退出码 2 自动用新 ID 重跑。记录只在当前进程有效；已结束操作沿用底层约 200 条的保留上限，超过上限的旧结果会明确不可用，仍不会重发原命令。每实例外部 exec 最多 4096 个 ID，满后需结束任务并重启应用。`write` 不具备幂等保护，只报告 `sent`、`completionKnown:false`；读输出确认状态，不自动重发。

接入通道只监听 `127.0.0.1` 的随机端口，以私有发现文件的随机 token 认证。Linux 目录为 `${XDG_CONFIG_HOME:-~/.config}/omniTerm/automation`，macOS 为 `~/Library/Application Support/omniTerm/automation`，Windows 为 `%APPDATA%/omniTerm/automation`。Unix 目录/文件权限为 0700/0600。CLI 不展示 token，并忽略崩溃残留的失效实例；正常关闭删除发现文件。桌面和 CLI 应以同一操作系统用户运行。可同时指定 `OMNITERM_AUTOMATION_DIR` 覆盖目录；设置 `OMNITERM_DISABLE_AUTOMATION=1` 启动桌面可关闭入口。

### 给 Codex / Claude Code 的 skill

可直接分发 [skills/omniterm-control/SKILL.md](skills/omniterm-control/SKILL.md)。也可把整个 `omniterm-control` 文件夹放进智能体的 skill 目录，例如 Codex 的 `~/.codex/skills/` 或 Claude Code 的 `~/.claude/skills/`。运行 CLI 的智能体需与 omniTerm 在同一操作系统中；WSL、容器和远程开发机不能自动共享宿主机的发现文件与本机端口。

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
node --test omniterm-cli/test/*.test.js
```

Tests cover argument boundaries, executable selection, Windows command handling, dependency failures, exit codes, and signal forwarding. On Windows, Ctrl+C is delivered by the shared console, allowing the child shell or Agent to handle it; the launcher does not force-kill it on that event. Hosted CI does not provide full interactive-console coverage. Real Ctrl+C behavior, SSH/SFTP hosts, serial devices, and Agent authentication need interactive testing on each target platform before a product release.

Licensed under Apache-2.0; see [LICENSE](LICENSE) and [NOTICE](NOTICE).

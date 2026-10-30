# omniTerm

**让 Codex Agent 与终端会话并肩工作的开源桌面终端。**

[English](README.md) · [桌面指南](omniterm-desktop/README.md) · [架构设计](docs/omniterm/architecture.md) · [开发路线图](docs/omniterm/roadmap.md) · [参与贡献](CONTRIBUTING.md)

omniTerm 面向 **macOS、Linux 和 Windows**，以 iTerm2 的终端体验、MobaXterm 的远程工具集为参考，在图形工作台中整合终端与 Agent。首个 Agent 引擎来自本仓库的 Codex Rust 源码。

**当前阶段：桌面初版实现。** 已完成 macOS arm64 本机应用打包并实际打开窗口，能看到真实 Shell 提示符。在 Electron 中通过了真实 PTY 的命令输入、`pwd`、尺寸变化、Ctrl+C 与进程清理检查。GUI 键盘与 Agent 交互、真实远程连接及其他操作系统仍未验收，尚未完成三平台发行验证。详见 [验证记录](docs/omniterm/desktop-test-plan.md#验收记录)。

## 桌面能力

| 功能 | 当前实现 |
| --- | --- |
| 本地终端 | 独立 Shell 多标签、目录选择、终端搜索、复制粘贴与终端显示设置 |
| Agent 分栏 | 在终端旁运行独立 Codex TUI，通过键盘完成输入、审批与取消 |
| 连接管理 | SSH、SFTP、串口与 SSH 转发的图形表单和已保存连接配置 |
| SSH / 端口转发 | 调用系统 OpenSSH，提供本地、远程及动态转发 |
| SFTP | 在终端标签中运行系统交互式 SFTP 客户端 |
| 串口 | 通过可选的 Python / pyserial miniterm 建立终端会话 |
| 本机打包 | 将桌面应用与从源码构建的完整 Agent 包一同打包 |

图形 SFTP 文件浏览、任意终端分屏、会话自动恢复和结构化 Agent 上下文共享仍待开发。Agent 不会自动读取其他终端标签，也不会因打开 SSH 标签而获得远程执行环境。普通 Shell 和连接会话使用当前用户权限，独立于 Codex Agent 的沙箱与审批机制。

## 从源码启动桌面应用

准备 **Node.js 22.12 或更新版本**、npm，以及 Electron/node-pty 所需的原生编译工具，然后克隆仓库：

```sh
git clone https://github.com/YanYaoyuan/omniTerm.git
cd omniTerm
```

按照 [本地 Agent 构建指南](docs/omniterm/local-build.md) 编译 Rust `codex` 和辅助程序，生成 `dist/omniterm-agent/`。开发时桌面应用会发现该包，打包时会将它放入应用资源目录。本地 Shell 不要求模型账号；使用 Agent 需要完成引擎的认证与配置。

随后启动桌面应用：

```sh
cd omniterm-desktop
npm ci
npm run rebuild
npm start
```

`npm run rebuild` 为 Electron 的原生模块 ABI 重建 node-pty；更换 Electron 版本或平台后需要重新执行。桌面应用使用独立的 npm 锁文件，与继承的 Codex pnpm workspace 分开管理。

SSH、转发和 SFTP 需要系统 OpenSSH 客户端；串口需要安装了 `pyserial` 的 Python 3。平台前提、操作方式与打包细节见 [桌面指南](omniterm-desktop/README.md)。

在 `omniterm-desktop/` 目录中生成本机应用包：

```sh
npm run package
```

输出位于仓库根目录 `dist/desktop/`。Agent 包必须与应用的平台和架构一致。签名、公证、安装器与自动更新属于后续发行工作，当前不宣称三平台已经发布。

## 可选 CLI 入口

仍可在已有终端中使用轻量命令入口：

```sh
node omniterm-cli/bin/omniterm.js --help
node omniterm-cli/bin/omniterm.js doctor
node omniterm-cli/bin/omniterm.js agent --version
```

它提供 `agent`、`shell`、`ssh`、`sftp`、`tunnel` 和 `serial` 命令，详见 [CLI 指南](omniterm-cli/README.md)。尚未发布 omniTerm npm 包。

## 开发与验证

```sh
node --test omniterm-cli/test/cli.test.js
npm --prefix omniterm-desktop test
npm --prefix omniterm-desktop run build
```

CI 配置覆盖 macOS、Linux、Windows 的 CLI 测试，以及桌面代码构建和后端模拟测试。桌面 CI 跳过 Electron 下载与原生模块构建脚本，不启动 GUI、不连接真实 PTY，也不编译 Rust 引擎或验证远程服务器与串口设备。原生运行验收请按 [桌面交互清单](docs/omniterm/desktop-test-plan.md) 执行，并明确记录未测试项目。

桌面代码位于 `omniterm-desktop/`，可选命令入口位于 `omniterm-cli/`，产品文档位于 `docs/omniterm/`。`codex-rs/`、`codex-cli/`、`sdk/` 保留上游内部命名，便于持续同步。具体后续范围见 [路线图](docs/omniterm/roadmap.md)。

## 开源与贡献

项目采用 [Apache-2.0](LICENSE)，保留 [NOTICE](NOTICE) 和第三方声明。欢迎通过 [Issues](https://github.com/YanYaoyuan/omniTerm/issues) 和 PR 参与。

omniTerm 是基于 OpenAI Codex 的独立项目，不是 OpenAI、iTerm2 或 MobaXterm 的官方产品。[来源与改动记录](docs/omniterm/upstream.md)。

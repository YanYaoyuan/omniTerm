# omniTerm

**以 Agent 为核心的开源跨平台终端工作台。**

[English](README.md) · [架构设计](docs/omniterm/architecture.md) · [开发路线图](docs/omniterm/roadmap.md) · [参与贡献](CONTRIBUTING.md)

omniTerm 面向 **macOS、Linux 和 Windows**，以 iTerm2 的终端体验、MobaXterm 的远程工具集为产品对标，核心亮点是与终端会话结合的 **Agent 模式**。第一阶段使用本仓库的 Codex 作为 Agent 基础。

**当前阶段：CLI 原型。** 已保留完整 Codex 源码并增加统一命令入口，通过现有终端和外部工具提供基础能力。桌面终端模拟器、多标签、分屏、连接管理器和图形 SFTP 浏览器尚未实现。

## 当前能力

| 命令              | 当前实现                                    |
| ----------------- | ------------------------------------------- |
| `omniterm agent`  | 启动 Codex，沿用其认证、配置和审批能力      |
| `omniterm shell`  | 在当前终端启动本地 Shell                    |
| `omniterm ssh`    | 调用系统 OpenSSH 客户端                     |
| `omniterm tunnel` | SSH 本地、远程和动态端口转发                |
| `omniterm sftp`   | 调用系统 SFTP 客户端                        |
| `omniterm serial` | 通过可选的 Python `pyserial` 打开串口控制台 |
| `omniterm doctor` | 检查所需工具是否可用                        |

这些命令目前是独立会话。Agent 不会自动读取 SSH 或串口会话内容；上下文共享与远程执行审批属于后续集成工作。普通终端命令也不受 Codex 的沙箱和审批机制管理。

## 快速开始

需要 **Node.js 22 或更新版本**：

```sh
git clone https://github.com/YanYaoyuan/omniTerm.git
cd omniTerm
node omniterm-cli/bin/omniterm.js --help
node omniterm-cli/bin/omniterm.js doctor

# 可选：从本地源码安装命令
npm install -g ./omniterm-cli
omniterm --help
```

尚未发布 omniTerm npm 包或桌面安装包。命令入口不依赖其他 npm 运行库；按需准备 Codex、OpenSSH，以及安装了 `pyserial` 的 Python 3 环境。Windows 可安装系统的 OpenSSH Client 可选功能。

```sh
omniterm agent "解释这个仓库的架构"
omniterm shell
omniterm ssh user@example.com
omniterm tunnel -L 127.0.0.1:8080:localhost:80 user@example.com
omniterm tunnel -D 127.0.0.1:1080 user@example.com
omniterm sftp user@example.com
omniterm serial /dev/ttyUSB0 115200
# Windows: omniterm serial COM3 115200
```

可通过 `OMNITERM_CODEX_BIN` 指定 Codex 可执行文件。详细依赖、平台差异和参数见 [CLI 指南](omniterm-cli/README.md)；从源码编译 Agent 见 [构建说明](docs/install.md)。

## 下一步

1. **桌面终端：** 跨平台窗口、多标签、分屏、搜索、主题、PTY/ConPTY 和会话生命周期。
2. **Agent 工作区：** 终端旁的 Agent 面板、明确的上下文共享、执行目标、审批和操作记录，优先对接 Codex app-server。
3. **远程工具箱：** SSH 连接配置、跳板机、端口转发管理、SFTP 文件传输和串口配置。

具体范围和验收标准见 [路线图](docs/omniterm/roadmap.md)。CI 覆盖新命令入口的 macOS、Linux、Windows 测试，不代表所有 Agent 功能、服务器和串口硬件已完成兼容性验证。

## 开源与贡献

项目采用 [Apache-2.0](LICENSE)，保留 [NOTICE](NOTICE) 和第三方声明。欢迎通过 [Issues](https://github.com/YanYaoyuan/omniTerm/issues) 和 PR 参与。

`codex-rs/`、`codex-cli/`、`sdk/` 保留为上游 Agent 基础；`codex-*` 内部名称暂不改动，以便持续同步上游。新增代码位于 `omniterm-cli/`。

```sh
node --test omniterm-cli/test/cli.test.js
```

omniTerm 是基于 OpenAI Codex 的独立项目，不是 OpenAI、iTerm2 或 MobaXterm 的官方产品。[来源与改动记录](docs/omniterm/upstream.md)。

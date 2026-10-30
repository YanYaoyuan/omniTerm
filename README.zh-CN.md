**语言 / Language：[简体中文](README.zh-CN.md) · [English](README.md)**

# omniTerm

**让本地终端、远程连接和 Codex Agent 在同一个开源桌面工作台中协作。**

[下载 0.0.1](https://github.com/YanYaoyuan/omniTerm/releases) · [桌面指南](omniterm-desktop/README.md) · [构建 Agent](docs/omniterm/local-build.md) · [反馈问题](https://github.com/YanYaoyuan/omniTerm/issues) · [参与贡献](CONTRIBUTING.md)

omniTerm 面向 **macOS、Linux 和 Windows**，把 Shell 多标签、分屏、SSH、图形 SFTP 文件面板、串口和端口转发放在同一个桌面应用中。旁边是从本仓库源码构建、在本机运行的 Codex Agent：选中终端输出，发送到 Agent 输入框，由你决定何时提交。

项目采用 **Apache-2.0**，当前版本为 **0.0.1**。核心价值是把日常终端操作、远程运维和可审阅的 Agent 实现组合起来。这是早期版本：提供平台包、通过某项运行检查，不代表三平台的全部功能都已完成人工验收。

## 下载

请在 [GitHub Releases](https://github.com/YanYaoyuan/omniTerm/releases) 获取已上传的安装包、校验值和版本说明。首发目标为：

| 系统 | 架构 | 包格式 |
| --- | --- | --- |
| macOS | Apple Silicon / arm64 | 含应用的 `.zip` |
| Ubuntu 22.04 | amd64 / x86_64 | `.deb` |
| Windows | x64 | `.zip` |

下载前请查看对应 Release 的附件清单与验证记录。首发 macOS、Windows 包尚无开发者签名；遇到系统提示时，按操作系统针对单个应用的正常确认流程打开。首发范围不包含 Intel Mac 安装包。

如果安装过此前的 `0.1.0~preview` Ubuntu 试用包，请明确选择下载的 `0.0.1` `.deb` 安装：Debian 的版本排序会将这次安装识别为降级。操作方式与平台差异见 [桌面指南](omniterm-desktop/README.md)。

## 现在可以做什么

| 工作流 | 当前已经实现 |
| --- | --- |
| 本地终端工作区 | 真实 PTY Shell、多标签、任意嵌套左右/上下分屏、窗格最大化、广播输入、搜索、标签命名/排序、命令面板 |
| Agent 协作 | 独立 Codex 分栏；在选中的本地工作目录启动；将终端选中文字放入 Agent 草稿，不自动提交 |
| SSH 连接 | 保存连接配置；导入含 `Include` 的 `~/.ssh/config`；使用 SSH 别名及单级/多级 `ProxyJump` 跳板机 |
| 远程文件 | SSH/SFTP 会话旁的图形文件面板；文件与文件夹上传/下载、重命名、删除、新建目录，以及先下载再拖出的文件交互 |
| 远程主机监控 | CPU、内存、磁盘、网络、负载、运行时间；远端提供受支持工具或接口时显示 GPU 数据 |
| 转发与设备 | 本地、远程、动态 SSH 端口转发；通过 Python/pyserial 使用串口 |
| 工作区恢复 | 重新打开本地标签、分屏布局、标签名和工作目录，创建新的 Shell 会话 |
| 外观 | 13 套主题、自定义背景图、字体/光标设置、WebGL 渲染及 DOM 回退；在支持的平台使用透明背景 |

omniTerm 的实用优势在于 **开源 Agent 与远程工具处于同一个工作区**：查看主机、传输文件、选中报错，再交给 Codex 分析，减少应用之间的切换。SSH 配置继续由系统 OpenSSH 解释；桌面和 Agent 源码都可检查、修改与扩展。

### Agent 与平台边界

- **Agent 在本机运行，不等于模型推理离线。** 模型访问取决于 Codex 的认证、服务提供方与配置，安装包不包含模型权重。本地 Shell 不需要模型账号。
- Agent 接收你主动发送的文本，不会自动读取所有标签，也不会继承远程 SSH 会话。Agent 工具执行遵循 Codex 配置的审批与沙箱；普通 Shell、SSH、SFTP、串口会话使用当前用户权限。
- 工作区恢复最多打开 16 个新的本地 Shell 窗格，不重放命令、不恢复滚屏内容；远程连接、设备和 Agent 会话不会自动重连。
- SSH、SFTP、转发依赖系统 OpenSSH；串口依赖 Python 3 和 `pyserial`。Windows 文件/监控面板使用独立的非交互 SSH 连接，需要密钥或 SSH Agent 认证；macOS/Linux 可通过 OpenSSH 控制套接字复用终端登录。
- 在不透明与透明窗口之间切换需要重启。Linux 需要桌面合成器；当前有边框窗口配置不支持 Windows 原生窗口透明。运行中的 TUI 可能缓存启动时的颜色，深浅主题切换后，可能需要在 Agent 空闲时重新启动它以恢复输入区可读性。

## 与其他终端比较

**官方资料核对日期：2026-10-30。** 下表比较平台、许可证和工作流侧重，不是性能排名或完整功能清单；未列出的功能不代表竞品不支持。

| 产品 | 桌面平台 | 许可证 | 工作流侧重 / Agent 集成 |
| --- | --- | --- | --- |
| **omniTerm 0.0.1** | **macOS、Linux、Windows** | **[Apache-2.0](LICENSE)** | **本地源码构建的 Codex 分栏，结合 SSH/SFTP 文件面板、跳板机、转发和串口；处于早期阶段** |
| [iTerm2](https://iterm2.com/index.html) | macOS | GPL v2 | 丰富的[终端、分屏与 Shell 集成](https://iterm2.com/features.html)；提供 [AI Chat](https://github.com/gnachman/iTerm2#key-features) 与[可选 AI 插件](https://iterm2.com/ai-plugin.html) |
| [Xshell](https://www.netsarang.com/en/xshell-all-features/) | Windows | [商业许可证](https://www.netsarang.com/en/xshell/)；[家庭/学校免费许可证](https://netsarang.atlassian.net/wiki/spaces/ENSUP/pages/2027552778/Free+User+Registration) | SSH/SFTP/串口、会话管理、跳板机、脚本与远程文件管理 |
| [MobaXterm](https://mobaxterm.mobatek.net/features.html) | Windows | [Home/Professional 版本](https://mobaxterm.mobatek.net/download.html)，遵循[产品 EULA](https://mobaxterm.mobatek.net/license.html) | 集成 SFTP、SSH 网关、串口、X11 服务器及 RDP/VNC 远程桌面工具 |
| [Warp](https://www.warp.dev/download) | macOS、Linux、Windows | [客户端开源：AGPL v3；UI 框架：MIT](https://github.com/warpdotdev/warp#licensing) | 内置编程 Agent，也支持 Codex 等外部 CLI Agent；侧重 Agent 开发环境 |
| [Tabby](https://github.com/Eugeny/tabby) | macOS、Linux、Windows | [MIT](https://github.com/Eugeny/tabby/blob/master/LICENSE) | SSH/串口、跳板机与嵌套分屏；插件生态包含 SFTP 和[基于 MCP 的 AI 集成](https://github.com/Eugeny/tabby#plugins) |

如果你希望同时获得 **Apache-2.0 代码基础、随应用提供的 Codex 引擎和统一的终端/远程工作流**，可以试用 omniTerm。iTerm2、Xshell 和 MobaXterm 在终端深度与运维工具方面仍是重要参考，omniTerm 尚未覆盖它们的完整功能集；当前也不内置 X11 服务器、RDP/VNC 客户端或企业凭据管理器。AI、开源和分屏是生态中多款产品共有的能力，不是 omniTerm 的独占声明。

## 技术栈

| 层次 | 技术 | 用途 |
| --- | --- | --- |
| 桌面运行时 | Electron 45 | 原生窗口、打包与隔离的 preload IPC |
| 界面 | React 19、CSS | 标签、分屏布局、连接表单、文件面板与设置 |
| 终端 | xterm.js 6 + WebGL/fit/search/Unicode 插件 | 终端模拟、渲染、尺寸调整与搜索 |
| 终端后端 | Rust（portable-pty、vt100） | PTY 会话、屏幕模型与 Agent 终端代理，运行在 Electron 主进程之外 |
| 远程访问 | 系统 OpenSSH + SFTP 协议 | SSH 配置、跳板机、端口转发和远程文件 |
| Agent | Rust Codex + code-mode host | 在本机运行的 Agent 引擎及执行支持，从本仓库构建并打包 |
| 串口 | Python 3 + pyserial miniterm | 可选串口设备会话 |
| 构建 | Node.js ≥22.12、npm、Vite、esbuild、Cargo | 桌面构建与原生 Agent 编译 |

界面使用 `contextIsolation: true`、`nodeIntegration: false` 与 Electron renderer sandbox，隔离 UI 和特权进程接口。这些设置不代表普通终端标签中的命令也受 Agent 沙箱限制。

## 从源码运行

准备 **Node.js 22.12+**、npm 和当前平台的原生编译工具：macOS 使用 Xcode Command Line Tools，Linux 使用 C/C++ 工具链，Windows 使用 MSVC Build Tools 与 Windows SDK。

```sh
git clone https://github.com/YanYaoyuan/omniTerm.git
cd omniTerm
```

按 [Agent 构建指南](docs/omniterm/local-build.md) 编译 `codex` 和辅助程序，生成 `dist/omniterm-agent/`，随后启动桌面应用：

```sh
cd omniterm-desktop
npm ci
npm run build:native
npm start
```

`npm run build:native` 编译终端后端（需要 Rust 1.85+）；修改 `native/terminal-host` 或更换目标平台后需重新执行。没有 Agent 包时仍可使用本地 Shell；omniTerm Agent 及应用打包需要兼容的 Agent 构建。

```sh
# 在 omniterm-desktop/ 目录中生成本机应用包
npm run package
```

输出位于仓库根目录的 `dist/desktop/`。Electron、终端后端和 Agent 必须匹配目标系统与架构。操作与打包细节见 [桌面指南](omniterm-desktop/README.md)，组件边界见 [架构设计](docs/omniterm/architecture.md)。

## 开发与验证

```sh
node --test omniterm-cli/test/cli.test.js
npm --prefix omniterm-desktop test
npm --prefix omniterm-desktop run build
```

仓库提供三平台代码检查，以及单独的原生运行/打包检查。特定构建已进行 macOS GUI 检查和 Ubuntu 安装、PTY、Electron 检查；自动化 IPC 输入不等同于真实键盘或输入法验收，串口硬件、不同远程服务器及 Windows 人工覆盖仍需专门验证。请结合[交互验收清单](docs/omniterm/desktop-test-plan.md)与每个 Release 的验证记录判断适用范围。

可选的 [CLI 启动器](omniterm-cli/README.md) 在已有终端中提供 `agent`、`shell`、`ssh`、`sftp`、`tunnel`、`serial` 和 `doctor`：

```sh
node omniterm-cli/bin/omniterm.js doctor
node omniterm-cli/bin/omniterm.js agent --version
```

桌面代码位于 `omniterm-desktop/`，启动器位于 `omniterm-cli/`。`codex-rs/`、`codex-cli/`、`sdk/` 保留上游内部命名，方便持续同步。欢迎通过 [Issues](https://github.com/YanYaoyuan/omniTerm/issues) 和 PR 提交可复现问题或参与开发。

## 开源与来源

omniTerm 采用 [Apache-2.0](LICENSE)，基于 [OpenAI Codex](https://github.com/openai/codex)。原有版权和第三方声明保留在 [NOTICE](NOTICE) 及分发包的依赖声明中，第三方组件继续遵循各自许可证。详见[上游来源与改动记录](docs/omniterm/upstream.md)。

omniTerm 是独立项目。比较表中的产品名称用于标识对应产品，不代表官方关联或背书。

# omniTerm Desktop

omniTerm 的桌面工作台使用 Electron、React、xterm.js 和 Rust 终端后端，将本地终端、
远程工具入口与 omniTerm Agent 放在同一个窗口中。
这是早期桌面版本；构建、打包与真实交互验证的结果分别记录。
已有 macOS GUI、Ubuntu 安装与原生 PTY、隔离服务器 SSH/SFTP/跳板机检查；
每个平台的具体覆盖以 [Release 说明](https://github.com/YanYaoyuan/omniTerm/releases)
和 [交互验证记录](../docs/omniterm/desktop-test-plan.md#验收记录) 为准。

## 安装 0.0.1

从 [GitHub Releases](https://github.com/YanYaoyuan/omniTerm/releases/tag/v0.0.1)
下载对应架构的文件及 SHA-256 校验值。安装包已包含 Electron、原生 PTY 和完整 Agent，
不需要另装 Node.js、Rust 或 Codex。

- **macOS Apple Silicon：** 解压 `omniTerm-0.0.1-macos-arm64.zip`，将 `omniTerm.app`
  复制到“应用程序”后打开。当前没有 Intel Mac 包，也没有 Developer ID 签名或 Apple 公证。
- **Windows x64：** 完整解压 `omniTerm-0.0.1-windows-x64.zip`，运行其中的
  `omniTerm.exe`；不要单独移动 exe，旁边的资源目录同样必需。当前没有 Authenticode 签名。
- **Ubuntu 22.04 amd64：** 在下载目录执行以下命令，然后从应用菜单打开 omniTerm：

```sh
sudo apt install ./omniterm_0.0.1-1_amd64.deb
```

此前安装过 `0.1.0~preview` 的用户会看到版本降级提示，这是首次正式版本改为 `0.0.1`
的版本排序结果。请核对文件名后确认。程序用普通用户启动，Agent 首次使用需要自己的认证配置。
Linux 安装器会安装声明的运行依赖，并推荐中文字体；macOS/Windows 的 SSH 与串口依赖见下文。
发布构建步骤与来源校验见 [发行指南](../docs/omniterm/releasing.md)。

## 工作台

- **本地终端：** 每个标签连接独立的真实 PTY/shell；支持新建、切换、关闭、
  窗口尺寸同步、终端滚动和搜索。
- **Agent 分栏：** 点击“启动 Agent”打开独立的 Codex TUI 会话，
  可直接使用键盘处理输入、选择、审批和取消；普通终端保持可用。
- **SSH、SFTP、串口、端口转发：** 图形表单收集连接信息，打开对应的终端会话。
  保存的连接配置便于再次连接；凭据仍由底层工具处理。
- **导入 SSH 配置：** 启动时读取 `~/.ssh/config`（含 `Include`），发现新主机后询问是否导入，
  确认前不保存任何内容；也可在命令面板或“设置”中手动导入。导入的连接只保存别名，
  由系统 ssh 继续应用配置文件中的全部设置。
- **跳板机：** `~/.ssh/config` 中的 `ProxyJump` 直接生效，连接表单也可填写一级或多级跳板；
  终端、SFTP、端口转发、文件面板和主机监控都经由跳板机连接。
- **SFTP 文件面板：** SSH 登录完成后，侧边栏直接显示远程文件，支持拖放上传、
  拖出下载、文件夹传输、重命名、删除和新建文件夹。
- **主机监控：** 终端下方实时显示远程主机的 CPU、内存、GPU、磁盘、网络、负载和运行时间。
- **分屏与命令面板：** 任意嵌套分屏、窗格最大化、广播输入，以及可搜索全部操作、
  连接和标签的命令面板。
- **工作区恢复：** 重新打开应用时恢复本地标签、分屏布局、标签名与各窗格目录。
- **主题：** 13 套内置主题，从标题栏、命令面板或“设置”切换，配色应用到整个界面。
- **背景：** 窗口可设为不透明、半透明（可调不透明度）或透明，并可设置自定义背景图片，
  调整图片强度和填充方式。切换透明窗口需要重启应用；Linux 需要运行桌面合成器。
- **终端设置：** 字体、字号、行高、光标、GPU 渲染、选中即复制与回滚行数。

SSH、SFTP 与转发使用系统 OpenSSH；串口使用 Python / pyserial miniterm。
这些客户端仍需安装在本机。会话进程不会在应用退出后保留。

omniTerm Agent 面板顶部显示上下文范围、执行目标和供应商/模型。Agent 只读取选定的终端，
命令须经你批准，且只在选定的目标终端运行，见[实现状态](../docs/omniterm/agent-workspace-status.md)。
你手动在普通终端中运行的命令不受这套审批管理。

## 从源码运行

需要 Node.js **22.12 或更新版本**、npm、Rust 1.85+ 和当前平台的原生编译环境：
macOS 使用 Xcode Command Line Tools，Linux 使用 C/C++ 编译工具链，
Windows 使用 MSVC Build Tools 与 Windows SDK。

先在仓库根目录按 [Agent 源码构建指南](../docs/omniterm/local-build.md)
生成 `dist/omniterm-agent/`。该包包括从 Rust 源码构建的 `codex`、
`codex-code-mode-host` 及必要资源。普通本地 shell 不需要模型账号，
使用 Agent 时需完成所选引擎的认证和配置。

然后运行桌面应用：

```sh
cd omniterm-desktop
npm install
npm run build:native
npm start
```

`npm run build:native` 编译 `native/terminal-host` 并放到 `dist/terminal-host/`；
修改它或更换平台后需要重新执行。`npm start` 先构建主进程、preload 和 React 界面，再启动 Electron。
桌面依赖和构建输出位于此目录，Agent 构建产物位于仓库根目录的 `dist/`。
完成依赖安装与终端后端构建后，也可以在仓库根目录运行 `npm run desktop`。

## 本机打包

在 `omniterm-desktop/` 中运行：

```sh
npm run package
```

打包脚本要求仓库根目录已有完整的 `dist/omniterm-agent/` 和 `dist/terminal-host/`，
并将其复制为应用的 `resources/` 下同名目录（macOS 位于 `.app/Contents/Resources/`）。
打包后的 Agent 使用随应用提供的本地源码构建包。

输出位于仓库根目录 `dist/desktop/`，按本机操作系统和架构生成应用目录。
仓库根目录的 `npm run desktop:package` 是同一打包流程的入口。
这一步生成开发应用包；签名、公证、安装器和自动更新尚需独立配置。
Electron、终端后端与 Agent 包必须面向同一目标平台和架构。
应在对应系统分别构建并验收，不能把单个平台的包视为跨平台发行完成。

## 交互与连接

打开“本地终端”建立 shell 标签；“打开文件夹”可以在所选目录新建终端。
点击标签切换会话，使用关闭按钮结束该标签所属会话。
macOS 使用 Command；Linux 与 Windows 使用 `Ctrl+Shift`，使 `Ctrl+W`、`Ctrl+D`、
`Ctrl+F`、`Ctrl+K` 等 shell 按键仍然发送到终端。完整列表见“设置 → 键盘快捷键”。

| 操作 | macOS | Linux / Windows |
| --- | --- | --- |
| 命令面板 | `⇧⌘P` | `Ctrl+Shift+P` |
| 新建标签 / 关闭窗格 | `⌘T` / `⌘W` | `Ctrl+Shift+T` / `Ctrl+Shift+W` |
| 向右 / 向下分屏 | `⌘D` / `⇧⌘D` | `Ctrl+Shift+E` / `Ctrl+Shift+O` |
| 切换窗格 | `⌘[` `⌘]`，`⌥⌘` + 方向键 | `Ctrl+Shift+[` `]`，`Ctrl+Alt` + 方向键 |
| 最大化 / 还原窗格 | `⇧⌘↩` | `Ctrl+Shift+Enter` |
| 广播输入到标签内所有窗格 | `⌥⌘I` | `Ctrl+Shift+I` |
| 发送选中内容到 Agent | `⇧⌘A` | `Ctrl+Shift+A` |
| 显示 / 隐藏 Agent | `⌘J` | `Ctrl+Shift+J` |
| 搜索 / 清屏 | `⌘F` / `⌘K` | `Ctrl+Shift+F` / `Ctrl+Shift+K` |
| 字号 | `⌘+` `⌘-` `⌘0` | `Ctrl++` `Ctrl+-` `Ctrl+0` |
| 切换标签 | `Ctrl+Tab`，`⌘1…9` | `Ctrl+Tab`，`Alt+1…9` |

**分屏：** 每个标签可任意嵌套左右/上下分屏，拖动分隔线调整大小，双击分隔线平均分配。
本地 shell 分屏沿用当前目录（shell 通过 OSC 7 报告目录时使用实时目录）；
SSH/SFTP 分屏使用同一连接配置重新连接。广播输入开启后，键盘输入同时写入该标签所有窗格。

**工作区恢复：** 默认开启，可在设置中关闭。布局在每次变化时保存，因此崩溃或强制退出后
也能恢复。每个本地窗格以新的 shell 在上次目录（优先 OSC 7 实时目录）打开，
不会重新执行任何命令，也不恢复滚屏内容；目录已不存在时在默认目录打开并给出提示。
SSH、SFTP、串口、端口转发和 Agent 窗格不会自动重连，恢复时被移除。最多恢复 16 个窗格。

**与 Agent 协作：** 在终端中选中文本后使用“发送到 Agent”，只有选中的内容会以粘贴方式
进入 Agent 输入框，不会自动提交，也不会共享其他输出。Agent 在当前焦点 shell 的目录启动。

**其他：** 五套内置配色、字体与行高、光标样式、WebGL GPU 渲染（不可用时自动回退）、
Unicode 11 字符宽度、选中即复制、`Cmd/Ctrl+点击`打开链接、拖放文件插入转义后的路径、
双击标签重命名、拖动标签排序、中键关闭标签，以及大小写/正则搜索。

在连接工具中选择 SSH、SFTP、串口或端口转发，填写当前表单要求的字段后连接。
SSH 的主机密钥确认、密码或密钥认证发生在终端内；不要把密码写入连接配置。
端口转发依赖 SSH 连接持续存在，结束所属会话时应关闭相应转发。
串口退出遵循 pyserial miniterm 的键盘规则，默认使用 `Ctrl+]`。

### SFTP 文件面板与主机监控

SSH 和 SFTP 标签成为当前窗格后，侧边栏切换到“文件”视图，下方出现主机监控栏；
侧边栏顶部可切回“工作区”。

- **登录：** macOS 和 Linux 上，面板通过 OpenSSH 控制套接字（`ControlMaster`）
  复用终端已完成的登录，不会再次询问凭据，应用也不接触密码。套接字位于私有临时目录，
  连接随标签关闭；应用异常退出时最多保留 30 秒。Windows OpenSSH 不支持控制套接字，
  面板改用独立的非交互连接，因此需要密钥或 SSH Agent 认证。
- **上传：** 把文件或文件夹拖到列表，或拖到某个文件夹行上。
- **下载：** 双击文件、使用右键菜单或选中后点击下载。也可以把列表项拖到本地：
  操作系统只能拖动已存在的文件，所以第一次拖动会先取回文件（最大 2 GiB）并标记“可拖出”，
  再次拖动即可放到本地文件夹。
- **安全：** 覆盖已有文件和删除前都会确认；传输先写入临时文件，完成后才替换目标；
  文件夹传输跳过符号链接，并拒绝会逃出下载目录的远程文件名。
- **监控：** 打开标签时在远程运行一个只读的 POSIX `sh` 循环，每 3 秒读取 `/proc` 等系统信息，
  不在远程安装任何程序，离开标签后停止。支持 Linux 和 macOS 主机；GPU 数据来自
  `nvidia-smi` 和 AMD 的 sysfs 接口，其他 GPU 不显示。
- **关闭：** 在设置中关闭“SSH 文件面板与主机监控”后，新连接恢复为不带额外选项的普通 `ssh`。

关闭窗口或退出应用会清理所属 PTY 会话。测试时需要检查子进程和监听端口
确实消失，不能只以标签关闭作为清理成功的证据。

## 开发与验收

```sh
npm run build
npm test
```

按照 [桌面交互验收清单](../docs/omniterm/desktop-test-plan.md)
验证 `pwd` 输入、Ctrl+C、中英文输入、尺寸变化、标签生命周期、Agent 按键、
远程连接表单校验，以及退出应用后的进程清理。
构建与自动化测试不能替代真实 SSH/SFTP 服务或串口设备测试。

主进程负责 PTY 和配置；renderer 通过受限 preload 桥调用明确的会话接口。
界面代码不应直接访问 Node.js 或任意文件路径。连接参数、会话所有权、窗口导航、
IPC 监听清理和子进程退出都需要在修改时保持验证。

项目使用 [Apache-2.0](LICENSE)，来源与第三方声明见 [NOTICE](NOTICE)。
贡献方式见 [贡献指南](../CONTRIBUTING.md)，安全报告见 [安全政策](../SECURITY.md)。

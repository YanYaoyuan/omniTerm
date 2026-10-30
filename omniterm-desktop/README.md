# omniTerm Desktop

omniTerm 的桌面工作台使用 Electron、React、xterm.js 和 node-pty，将本地终端、
远程工具入口与基于 Codex 的 Agent 放在同一个窗口中。
这是早期桌面版本；构建、打包与真实交互验证的结果需要分别记录。
macOS arm64 的本机 `.app` 已打包并实际打开窗口，真实 Electron PTY 测试已通过。
GUI 键盘与 Agent 交互、远程服务和其他平台尚未验收，详见
[验证记录](../docs/omniterm/desktop-test-plan.md#验收记录)。

## 工作台

- **本地终端：** 每个标签连接独立的真实 PTY/shell；支持新建、切换、关闭、
  窗口尺寸同步、终端滚动和搜索。
- **Agent 分栏：** 点击“启动 Agent”打开独立的 Codex TUI 会话，
  可直接使用键盘处理输入、选择、审批和取消；普通终端保持可用。
- **SSH、SFTP、串口、端口转发：** 图形表单收集连接信息，打开对应的终端会话。
  保存的连接配置便于再次连接；凭据仍由底层工具处理。
- **分屏与命令面板：** 任意嵌套分屏、窗格最大化、广播输入，以及可搜索全部操作、
  连接和标签的命令面板。
- **终端设置：** 配色、字体、字号、行高、光标、GPU 渲染、选中即复制与回滚行数。

SSH、SFTP 与转发使用系统 OpenSSH；串口使用 Python / pyserial miniterm。
这些客户端仍需安装在本机。SFTP 当前提供交互式客户端终端，
**图形文件浏览器尚未实现**。会话进程不会在应用退出后自动恢复。

Agent 分栏运行独立引擎会话；它不会自动读取其他标签中的 SSH、串口或 shell
上下文。普通终端会话也不受 Codex Agent 的沙箱和审批机制管理。

## 从源码运行

需要 Node.js **22.12 或更新版本**、npm 和当前平台的原生模块编译环境。
`node-pty` 需要匹配 Electron 的原生模块 ABI：macOS 使用 Xcode Command Line
Tools，Linux 使用 C/C++ 编译工具链，Windows 使用 MSVC Build Tools 与 Windows SDK。

先在仓库根目录按 [Agent 源码构建指南](../docs/omniterm/local-build.md)
生成 `dist/omniterm-agent/`。该包包括从 Rust 源码构建的 `codex`、
`codex-code-mode-host` 及必要资源。普通本地 shell 不需要模型账号，
使用 Agent 时需完成所选引擎的认证和配置。

然后运行桌面应用：

```sh
cd omniterm-desktop
npm install
npm run rebuild
npm start
```

`npm run rebuild` 为当前 Electron 重建 node-pty；更换 Electron 版本或平台后
需要重新执行。`npm start` 先构建主进程、preload 和 React 界面，再启动 Electron。
桌面依赖和构建输出位于此目录，Agent 构建产物位于仓库根目录的 `dist/`。
完成依赖安装与原生模块重建后，也可以在仓库根目录运行 `npm run desktop`。

## 本机打包

在 `omniterm-desktop/` 中运行：

```sh
npm run package
```

打包脚本要求仓库根目录已有完整的 `dist/omniterm-agent/`，并将其复制为应用的
`resources/omniterm-agent/` 资源目录（macOS 位于 `.app/Contents/Resources/`）。
打包后的 Agent 使用随应用提供的本地源码构建包。

输出位于仓库根目录 `dist/desktop/`，按本机操作系统和架构生成应用目录。
仓库根目录的 `npm run desktop:package` 是同一打包流程的入口。
这一步生成开发应用包；签名、公证、安装器和自动更新尚需独立配置。
Electron、node-pty 原生模块与 Agent 包必须面向同一目标平台和架构。
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

**与 Agent 协作：** 在终端中选中文本后使用“发送到 Agent”，只有选中的内容会以粘贴方式
进入 Agent 输入框，不会自动提交，也不会共享其他输出。Agent 在当前焦点 shell 的目录启动。

**其他：** 五套内置配色、字体与行高、光标样式、WebGL GPU 渲染（不可用时自动回退）、
Unicode 11 字符宽度、选中即复制、`Cmd/Ctrl+点击`打开链接、拖放文件插入转义后的路径、
双击标签重命名、拖动标签排序、中键关闭标签，以及大小写/正则搜索。

在连接工具中选择 SSH、SFTP、串口或端口转发，填写当前表单要求的字段后连接。
SSH 的主机密钥确认、密码或密钥认证发生在终端内；不要把密码写入连接配置。
端口转发依赖 SSH 连接持续存在，结束所属会话时应关闭相应转发。
串口退出遵循 pyserial miniterm 的键盘规则，默认使用 `Ctrl+]`。

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

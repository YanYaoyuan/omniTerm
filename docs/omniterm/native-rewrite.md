# omniTerm 原生 Rust 重写

目标：去掉 Electron 和 Web 运行时，整个桌面应用用 Rust 原生实现，终端由 GPU 直接绘制。在功能对齐之前，Electron 版继续作为发布版本，原生版在 `omniterm-native/` 中逐阶段推进，两者共用同一个 Rust 终端后端。

## 架构

| 层 | 选择 | 理由 |
| --- | --- | --- |
| 窗口与事件 | eframe 0.37（winit 0.30） | 三平台窗口、高 DPI、输入法、剪贴板、全屏、透明窗口都已具备 |
| 终端绘制 | 自己的 wgpu 30 渲染器，嵌在 egui 的渲染通道里 | 每个窗格一次实例化绘制：背景合并成矩形，字形从图集采样；内容不变的窗格不重建 |
| 字形 | cosmic-text 0.19（swash 光栅化） | 每个字符和样式只整形、光栅化一次，之后从 2048² 图集复用；中日韩和 emoji 由回退字体处理 |
| 终端仿真 | alacritty_terminal 0.26 | Alacritty 和 Zed 使用的仿真器，支持滚动回看、选择、重排、宽字符 |
| PTY、会话、Agent 执行边界 | 现有的 `omniterm-terminal-host`，作为库直接链接 | 与 Electron 版同一份代码、同一套请求；不再需要子进程和帧编码 |
| 界面外壳（标签、面板、对话框） | egui | 外壳只占每帧很小的开销，终端本身不经过 egui 的文字排版 |

数据路径：PTY 读线程 → 主机的输出帧 → 输出线程直接喂给仿真器（不经过界面线程）→ 下一帧只重建发生变化的窗格。仿真器要回给程序的应答（例如光标位置查询）和用户输入走同一个写线程，按顺序写入，界面线程从不等待 PTY。

## 阶段

| 阶段 | 内容 | 状态 |
| --- | --- | --- |
| 0 | 窗口、GPU 终端绘制、本地 Shell、标签、分屏与拖动调整、窗格最大化、全屏（F11 / ⌃⌘F）、选择与复制粘贴、括号粘贴、输入法（中文）、滚动回看、主题、字号、基准测试与 CI 冒烟测试 | 本分支 |
| 1 | 设置与持久化（主题、字体、布局、恢复工作区）、搜索、鼠标上报、OSC 7/133（当前目录、提示符标记）、超链接、透明与背景图、广播输入、命令面板 | 待做 |
| 2 | 连接：SSH 配置导入、保存的连接、SFTP 文件面板、串口、隧道、远程资源指标 | 待做 |
| 3 | omniTerm Agent：模型服务配置（系统钥匙串保存密钥）、Agent 引擎通信、上下文选择、审批卡片、SSH 握手执行 | 待做 |
| 4 | 监控面板与进程窗口、停靠工作区（面板拖放） | 待做 |
| 5 | 三平台打包与签名、自动更新、和 Electron 版功能对齐后切换发布 | 待做 |

## 功能对照（Electron → 原生）

| Electron 模块 | 原生对应 | 阶段 |
| --- | --- | --- |
| `TerminalView.jsx`（xterm.js + WebGL） | `grid.rs`、`gpu.rs`、`glyphs.rs`、`terminal.rs` | 0 |
| `session-manager.mjs`、`terminal-host.mjs` | `backend.rs`（直接链接主机） | 0 |
| `layout.js` 窗格部分 | `layout.rs` | 0 |
| `shortcuts.js` | `app.rs` 快捷键 | 0 / 1 |
| `themes.js` | `theme.rs`（目前 3 个主题） | 0 / 1 |
| `command-plan.mjs`、`shell-integration.mjs` | 本地 Shell 已有，其余待移植 | 0 / 1 |
| `workspace.js`、设置 | 待移植 | 1 |
| `ssh-config.mjs`、`profile-store.mjs`、`remote-manager.mjs`、`sftp-client.mjs`、`remote-metrics.mjs` | 待移植 | 2 |
| `agent-*.mjs`、`provider-store.mjs`、`AgentPanel.jsx` | 待移植 | 3 |
| `monitor-*.mjs`、`local-probe.mjs`、`ProcessWindow.jsx`、停靠布局 | 待移植 | 4 |
| `package*.mjs`、`window-chrome.mjs` | 待移植 | 5 |

## 运行

```sh
cd omniterm-native
cargo run --release                    # 打开窗口
cargo run --release -- --bench 32      # 无窗口基准：仿真吞吐、Agent 屏幕模型吞吐、整帧构建耗时
cargo run --release -- --smoke out.png # 打开窗口，在真实 Shell 里执行一条命令，截图后退出
```

快捷键：macOS 用 ⌘，其他平台用 Ctrl+Shift。T 新标签、W 关闭窗格、E/D 向右分屏、O/⇧D 向下分屏、Enter 最大化窗格、[ ] 切换窗格、C/V 复制粘贴、= - 0 字号；F11（macOS ⌃⌘F）全屏。Ctrl+C 有选中内容时复制，否则发送中断。

## 已知限制（阶段 0）

- 只有本地 Shell；没有 SSH 面板、Agent、监控、设置持久化。
- 不支持鼠标上报（vim、htop 的鼠标操作），连字不合并。
- 非 macOS 平台上 Ctrl+V 会被当作粘贴，不会把 ^V 发给程序。
- 程序通过 OSC 52 写剪贴板暂不支持，等有权限提示后再开放。
- macOS 的 CI 只构建、单测和基准测试，不开真实窗口；Linux（lavapipe 软件 Vulkan）和 Windows（WARP）在 CI 里开真实窗口做冒烟测试。

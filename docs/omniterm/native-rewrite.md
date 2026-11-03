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
| 0 | 窗口、GPU 终端绘制、本地 Shell、标签、分屏、全屏、选择与复制粘贴、输入法、滚动回看、基准测试与 CI 冒烟测试 | 已完成 |
| 1 | 标题栏与窗口控件、侧栏、命令面板、全部快捷键、设置（13 个主题、字体、光标、透明窗口、背景图）、工作区恢复、搜索、链接、鼠标上报、文件拖放、广播输入、标签重命名与排序 | 已完成 |
| 2 | 保存的连接（与桌面版共用 profiles.json）、SSH 配置导入、SSH / SFTP / 串口 / 隧道会话、SFTP 文件面板与传输、远程主机指标 | 已完成 |
| 3 | omniTerm Agent：模型服务（共用 model-providers.json，密钥在系统钥匙串）、Agent 引擎、上下文、审批卡片、工具 | 已完成 |
| 4 | 监控面板与进程窗口、停靠工作区（拖动面板到窗口任一边缘） | 已完成 |
| 5 | 三平台安装包与签名、自动更新 | 待做：CI 目前上传三平台可执行文件 |

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

## 与桌面版的差异

- 停靠：面板拖到窗口的上、下、左、右边缘，同一边缘的面板可调整分割；单个终端窗格还不能停靠到面板旁边，也不能拖动窗格到其他标签。
- SFTP 文件不能直接拖出到系统文件管理器，请用“下载”。
- 连字不合并；程序通过 OSC 52 写剪贴板不支持（桌面版同样不支持）。
- 非 macOS 平台上 Ctrl+V 是粘贴，不会把 ^V 发给程序。
- 安装包、签名与自动更新尚未接入，CI 上传三平台可执行文件。
- macOS 的 CI 只构建、单测和基准测试，不开真实窗口；Linux（lavapipe 软件 Vulkan）和 Windows（WARP）在 CI 里开真实窗口做冒烟测试。

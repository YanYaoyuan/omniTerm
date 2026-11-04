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
| 4 | 监控面板与进程窗口、停靠工作区（拖动面板或单个终端窗格到窗口任一边缘）、就绪标记与 SSH 自动执行 | 已完成 |
| 5 | 三平台安装包：macOS `omniTerm.app`（ZIP，ad hoc 签名）、Windows 便携 ZIP、Linux `.deb` 与 tar.gz，附 SHA-256 与清单 | 已完成（与桌面版一样不签名、无自动更新） |

## 功能对照（Electron → 原生）

| Electron 模块 | 原生对应 | 阶段 |
| --- | --- | --- |
| `TerminalView.jsx`（xterm.js + WebGL） | `grid.rs`、`gpu.rs`、`glyphs.rs`、`terminal.rs` | 0 |
| `session-manager.mjs`、`terminal-host.mjs` | `backend.rs`（直接链接主机） | 0 |
| `layout.js` 窗格部分 | `layout.rs` | 0 |
| `shortcuts.js` | `app.rs` 快捷键 | 0 / 1 |
| `themes.js` | `theme.rs`（13 个主题） | 0 / 1 |
| `command-plan.mjs`、`shell-integration.mjs` | `core/plan.rs`、`backend.rs`、`assets/shell-integration.txt` | 0 / 1 |
| `workspace.js`、设置 | `core/workspace.rs`、`core/settings.rs` | 1 |
| `ssh-config.mjs`、`profile-store.mjs`、`remote-manager.mjs`、`sftp-client.mjs`、`remote-metrics.mjs` | `core/ssh_config.rs`、`core/profile.rs`、`remote/` | 2 |
| `agent-*.mjs`、`provider-store.mjs`、`AgentPanel.jsx` | `agent/`、`ui/agent_panel.rs` | 3 |
| `monitor-*.mjs`、`local-probe.mjs`、`ProcessWindow.jsx`、停靠布局 | `monitor/`、`ui/monitor_view.rs`、`ui/process_window.rs`、`ui/dock.rs` | 4 |
| `package*.mjs`、`window-chrome.mjs` | `scripts/package.mjs`、`ui/chrome.rs` | 5 |

## 运行

```sh
cd omniterm-native
cargo run --release                    # 打开窗口
cargo run --release -- --bench 32      # 无窗口基准：仿真吞吐、Agent 屏幕模型吞吐、整帧构建耗时
cargo run --release -- --smoke out.png # 打开窗口，在真实 Shell 里执行一条命令，截图后退出
```

快捷键：macOS 用 ⌘，其他平台用 Ctrl+Shift。T 新标签、W 关闭窗格、E/D 向右分屏、O/⇧D 向下分屏、Enter 最大化窗格、[ ] 切换窗格、C/V 复制粘贴、= - 0 字号；F11（macOS ⌃⌘F）全屏。Ctrl+C 有选中内容时复制，否则发送中断。

### 本地 Shell

在“设置 → 本地 Shell”填写程序名称或完整路径（例如 `/bin/zsh`、`pwsh.exe`、
`wsl.exe`），并逐项添加启动参数。程序路径和每项参数都不需要外层引号：包含空格的
路径或参数仍是一个完整值，应用不会把它们拼成 Shell 命令。

程序和参数列表都留空时沿用原有系统自动选择；程序留空但填写了参数时，用这些参数
替换自动选择的 Shell 的默认参数。指定程序后，参数列表完全按填写顺序传递。
设置仅应用到随后新建、分屏和恢复的本地终端，现有会话继续使用启动时的程序。
旧设置文件无需迁移；“恢复系统默认”可清空自定义配置。

异步分屏的请求绑定到原标签和目标窗格。关闭或移走目标后，迟到的结果会回收会话，
不会重新创建标签，也不会弹出已取消请求的错误。

## 与桌面版的差异

- 停靠：面板和单个终端窗格都可以拖到窗口的上、下、左、右边缘，与同一边缘的面板并排并可调整分割；停靠的窗格拖回标签栏或其他窗格即可还原。
- 就绪标记：Agent 的终端选择器里对未知交互状态的 SSH 或本地 Shell 提供“启用就绪标记”，需要握手的终端可勾选“自动执行”，与桌面版一致。
- SFTP 拖出：与桌面版一样，第一次拖动文件把它下载到私有临时目录（最大 2 GiB），行尾显示“可拖出”后再次拖动即可放到系统文件管理器；临时目录在 SSH 标签关闭时删除。Windows 用 OLE，macOS 用 NSDraggingSession；Linux 的 X11 会话用自带的 XDND 拖放来源（独立连接、根窗口指针抓取），Wayland 会话在 winit 的连接上用 `wl_data_device` 发起拖动，都以 `text/uri-list` 提供文件。
- X11 转发（桌面版没有，原生版新增，参照 MobaXterm）：SSH 连接可选“关闭 / 受限（-X）/ 信任（-Y）”，远程图形程序显示在本机。Linux 用当前 `DISPLAY`（X11 或 XWayland）；macOS 用 XQuartz 并指定它的 xauth；Windows 先找 127.0.0.1:6000+N 上已运行的 X 服务器，没有就带新生成的 MIT-MAGIC-COOKIE-1 自动启动已安装的 VcXsrv 或 Xming（不关闭访问控制），并让 ssh 把 omniTerm 自身当作 `xauth` 取得这个授权；Windows 上总是信任模式（没有可生成受限授权的 xauth）。找不到 X 服务器时照常连接，并提示如何安装。这个选项保存在 `native-profile-x11.json`，不写进与桌面版共用的 `profiles.json`（桌面版会拒绝未知字段）。
- 连字不合并；程序通过 OSC 52 写剪贴板不支持（桌面版同样不支持）。
- 非 macOS 平台上 Ctrl+V 是粘贴，不会把 ^V 发给程序。
- 打包：`node omniterm-native/scripts/package.mjs` 在 `dist/release-native` 生成安装包，发布流程把源码构建的 Agent 一起打进去；CI 用 `--without-agent` 只打包应用本身。和桌面版一样不做 Developer ID 签名、公证或自动更新。
- macOS 的 CI 只构建、单测和基准测试，不开真实窗口；Linux（lavapipe 软件 Vulkan）和 Windows（WARP）在 CI 里开真实窗口做冒烟测试。

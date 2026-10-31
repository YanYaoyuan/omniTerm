# omniTerm Agent 工作区实现状态

对应 [开发方案](agent-workspace-plan.md) 与 [Issue #4](https://github.com/YanYaoyuan/omniTerm/issues/4)。
本文记录代码已实现的内容和已经验证的范围。**真实模型服务与三平台 GUI 验收尚未执行**，
因此 Issue 的验收项不能因本文而标记完成。

## 与方案的差异

方案要求本阶段保留 node-pty。按维护者后续决定，终端后端改为 Rust：

| 方案 | 实际实现 |
| --- | --- |
| node-pty 与 `session-manager.mjs` | `omniterm-desktop/native/terminal-host`，Electron 主进程的 Rust 子进程 |
| `workspace-registry.mjs`、`terminal-context.mjs`、`agent-terminal-broker.mjs` | 同一 Rust 进程内的 `host.rs`、`screen.rs`、`interaction.rs`、`broker.rs` |
| `agent-runtime.mjs`、`provider-store.mjs`、`src/agent/` | 按方案实现，另有 `agent-service.mjs`、`agent-tools.mjs` |

Electron、React 和 xterm.js 保持不变。SSH 配置、SFTP 面板和主机指标（`remote-manager.mjs` 等）
仍在主进程：它们主要等待外部进程，不是终端数据路径。

## 终端后端

主进程通过 stdin（每行一个 JSON 请求）和 stdout（带长度前缀的二进制帧）与 host 通信。
Host 退出时所有 PTY 结束；stdin 关闭（包括 Electron 崩溃）时 host 清理全部会话。

在单核机器上用 20 MB 连续输出测量吞吐，每种配置运行多次：

| 配置 | 吞吐 |
| --- | --- |
| node-pty 直接读取（无缓冲、无解析，退出时丢失约 10 KB） | 28–30 MB/s |
| Rust host，每次读取都解析屏幕 | 12–14 MB/s |
| Rust host，当前实现（读屏时才解析） | 29–37 MB/s，无丢失 |

因此该改动的实际收益是**吞吐与 node-pty 同级，并把 PTY、缓冲和解析移出 Electron 主线程**；
不应描述为大幅提速。这是单核环境下到 Node 客户端的数据，未包含 renderer 和 xterm.js。

屏幕解析是延迟的：任何读取屏幕状态的操作先补齐解析，所以执行前的校验总是基于最新状态。
超过 1 MiB 的突发输出中，较早部分只提取模式切换与命令标记，不渲染文本；这部分字节数以
`missingBytes` 报告，上下文标记为截断。

## Agent 执行边界

- `features.dynamic_tools_only` 是新增的引擎开关：开启后模型只能看到桌面提供的工具，
  内置、MCP、扩展和托管工具全部不注册。桌面每次启动引擎都开启它，同时使用 `environments: []`。
- 若仍出现内置工具条目（`commandExecution` 等），桌面立即中止该轮并结束引擎。
- 模型工具不包含批准能力；批准只来自主窗口 IPC。
- 操作绑定 `sessionId + epoch`，批准绑定 `interactionRevision`，执行前重新校验。
  用户输入、备用屏幕切换、关闭和重连都会使旧操作失效；广播输入是用户输入，同样如此。
- 同一终端的 Agent 写入串行执行。操作只存在于 host 内存，引擎或应用重启后不会恢复。
- 已写入的命令不重放：结果不明时状态为 `outcomeUnknown`。

### 就绪判断

| 终端 | 判定来源 | 不能确认时 |
| --- | --- | --- |
| Unix 本地 shell | 带会话 nonce 的 OSC 133 标记（面板中“启用就绪标记”，支持 zsh 与 bash 4.4+）；前台进程组、终端模式和备用屏幕只用于判定“不可提交” | `unknown`：只能输入、不自动回车 |
| SSH | 同上；ssh 客户端始终在前台，本地信号不说明远端状态 | 同上 |
| Windows 本地 shell | 当前没有可靠信号 | 同上 |
| SFTP、串口、隧道 | 不接受 shell 命令 | 可作为上下文，不能作为执行目标 |

shell 进程在前台不等于处于提示符：`read` 等内置命令运行在 shell 内，会把提交的命令当作自身输入，
因此没有标记时绝不自动提交。bash 4.4 之前没有 `PS0`，无法标记命令开始，脚本会拒绝启用（macOS 自带的
bash 3.2 属于此类）。未带 nonce 的 OSC 133 不参与就绪判断。没有 shell integration 时退出码为未知，不会编造。

已知限制：nonce 会出现在输入回显中，这是尽力防护，不是认证；真正的边界仍是逐条批准和目标绑定。
上下文预算优先保留选区和尾部输出，尚未按失败命令优先裁剪。选区与 `@` 引用只作用于本机窗口。

## 模型服务

| 配置 | 协议 | 默认 Base URL |
| --- | --- | --- |
| OpenAI | Responses | `https://api.openai.com/v1` |
| OpenAI（现有登录） | Responses | 使用引擎现有登录 |
| Qwen / 百炼 | Chat Completions；Responses 需能力测试 | `https://dashscope.aliyuncs.com/compatible-mode/v1` |
| Claude / Anthropic | Anthropic Messages | `https://api.anthropic.com/v1` |
| Kimi / Moonshot | Chat Completions | `https://api.moonshot.ai/v1` |
| OpenRouter | Chat Completions；Responses 需能力测试 | `https://openrouter.ai/api/v1` |
| 自定义 | 显式选择已实现协议 | 用户填写 |

引擎配置新增 `wire_api = "chat"` 与 `wire_api = "anthropic_messages"`。适配器只转换普通 JSON
函数工具；freeform、namespace 和托管工具不转换，会记录警告后省略。工具参数完整且通过 JSON
解析后才产生调用，截断或非法参数使本轮失败。Anthropic 单次输出上限目前固定为 8192 token。

API Key 经 Electron `safeStorage` 加密保存；Linux 的 `basic_text` 后端视为不可用，密钥只留在本次
运行的内存中。Key 通过环境变量只传给对应引擎进程，不进入参数、配置、日志或 renderer。
API Key 配置使用独立的 `CODEX_HOME`，不读取用户的引擎配置；“现有登录”配置读取用户现有目录，
但内置工具与 hooks 仍被关闭。

每种配置有独立引擎进程，切换后新建对话，此前内容不发送给新服务。修改协议、端点、模型、仅聊天设置
或 API Key 后，下一轮同样结束旧引擎和任务、撤销放行授权并新建对话；只改名称或上下文预算时保留对话。模型设置中的测试会实际调用
一次工具，结果记录为“连接/工具已验证”；修改协议、端点或模型后验证失效。

## 验证记录

| 项目 | 环境 | 结果 |
| --- | --- | --- |
| Rust host 单元测试 | Linux x86_64，Rust 1.95 | 62 项通过 |
| 桌面 JS 测试 | Linux，Node 22 | 108 项通过（host 为模拟） |
| 真实 PTY | `npm run test:pty`，Linux bash | 通过：输入、目录、尺寸、Unicode、Ctrl+C、退出、进程清理 |
| 批准后执行 | 本地 bash，直接调用 host | 通过：未批准不写入；批准后执行并返回输出 |
| Rust host 与真实 PTY/ConPTY | CI：Ubuntu、macOS、Windows | 通过 |
| 引擎协议适配与配置测试 | CI：Ubuntu，`codex-api` 等 | 通过；使用模拟流，未连接真实服务 |
| `dynamic_tools_only` 工具边界 | CI：Ubuntu，`codex-core` 单元测试 | 通过；验证工具注册，未经真实 app-server |

**未执行：** Electron 窗口与新面板的实际操作；真实 app-server 回环；任何真实模型服务；真实 SSH
目标；打包应用验证脚本 `verify-desktop.mjs`。
Issue 的 A 批次（真包 app-server 回环）和 E 批次必须在这些环境完成后才算通过。
记录测试时请写明模型、端点和平台组合。

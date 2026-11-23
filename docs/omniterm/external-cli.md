# 原生桌面外部 CLI

当前实现位于 PR21 原生应用：`omniterm-native` 链接的 terminal-host 暴露本机通道，`omniterm-cli` 的 `session` 子命令调用该通道。桌面必须更新到包含此功能的构建；旧进程无法在运行中加载新接口。Electron 进程不启动该通道。

使用步骤和边界见 [CLI 指南](../../omniterm-cli/README.md)，可直接交给外部智能体的说明见 [omniterm-control skill](../../omniterm-cli/skills/omniterm-control/SKILL.md)。原生分发包同时携带 CLI 与 skill，Node.js 22+ 由使用者提供。

## 本机协议 v1

每个原生进程在用户数据目录的 `automation/` 下创建随机 UUID 命名的 JSON 文件，内容为 `protocol:1`、`instance`、`pid`、随机 loopback `port`、随机 `token`。同用户读取此文件即获得控制能力。Unix 权限为目录 0700、文件 0600；Windows 沿用用户 AppData ACL。不要把发现文件纳入日志或版本控制。

传输为单次 TCP 连接的一行 UTF-8 JSON，客户端固定连接 `127.0.0.1`，没有远程地址配置。请求：

```json
{"token":"<private>","method":"read","params":{"terminal":"<session UUID>","epoch":1,"maxLines":200,"sinceSeq":123}}
```

响应为 `{"ok":true,"result":...}` 或 `{"ok":false,"error":"..."}`，均以换行结束。每连接一个请求，最大 1 MiB，读写超时 5 秒，最多 16 个处理连接。CLI 执行等待通过 result 轮询完成，不占用服务端长连接。不是 HTTP/WebSocket 服务。

仅允许 `ping`、`list`、`read`、`setup`、`write`、`exec`、`result`、`cancel`；不能透传任意 Host 方法或请求新建会话。除 list/ping 外的方法通过显式 session ID + epoch 或本实例已登记的 request-id 绑定目标；唯一的 8 位以上 session 前缀可用。read 使用一次性、限于单会话的 context task，沿用屏幕渲染、增量序号、截断报告和脱敏。write 是明确的原始输入，沿用用户输入对 broker 的接管处理。

exec 为每个 request-id 创建独立 broker task，目标固定为 session + epoch。通过预检查后只批准一次、排队一次；SSH 自动提交仍需要用户已允许的会话设置以及行编辑器的即时握手。setup 是外部调用者显式授权发送的集成脚本及 SSH opt-in。外部接口不为内置 Agent 创建执行授权，两个任务的操作隔离。

服务记录 request-id、完整请求参数及 operation ID，先登记再授权/执行。重复请求返回原操作，修改参数拒绝；即便底层已回收旧结果，也不会重新派发。幂等表不淘汰，达到 4096 条时拒绝新 exec，应用重启后清空；底层已结束操作保留约 200 条。每次最终结果读取会结束其 task 并撤销授权。关闭/重连/键盘接管/完成信号超时沿用 broker 状态，不猜测成功、不自动重发、不自动把 exec 退化为 write。

`OMNITERM_AUTOMATION_DIR` 指定发现目录；`OMNITERM_DISABLE_AUTOMATION=1` 禁止启动服务。退出先停止接入并移除发现文件，再关闭会话。崩溃残留由 CLI 的鉴权 ping 过滤，不因旧文件而假定进程仍存活。

## 验证

```sh
node --test omniterm-cli/test/*.test.js
cargo test --locked --manifest-path omniterm-desktop/native/terminal-host/Cargo.toml
cargo test --release --locked --manifest-path omniterm-native/Cargo.toml
```

terminal-host 测试使用真实 bash PTY 和 loopback 通道；以 SSH 类型会话验证远端探测握手、输出与退出码、重复请求、过时代次和目标隔离。CLI 测试以独立本机服务验证多实例选择、增量读取、长任务结果轮询、退出码、断开时的未知结果和脚本参数边界。真实公司资产、MFA、Windows/macOS 实机仍需对应环境验收，不能由隔离测试代替。

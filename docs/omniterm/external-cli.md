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

终端允许 `ping`、`list`、`info`、`read`、`search`、`setup`、`write`、`exec`、`result`、`cancel`；不能透传任意 Host 方法或请求新建会话。除 list/ping 外的方法通过显式 session ID + epoch 或本实例已登记的 request-id 绑定目标；唯一的 8 位以上 session 前缀可用。read 使用一次性、限于单会话的 context task，沿用屏幕渲染、增量序号、截断报告和脱敏。write 是明确的原始输入，沿用用户输入对 broker 的接管处理。

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

## 文件页面与目标绑定

`session bind` / `files bind` 在客户端独占创建一个不含 token 的目标 JSON，字段为 version、kind、instance，以及 terminal/epoch 或 fileSession/generation。`--target` 校验类型并固定实例与连接，不允许叠加冲突的 ID/代次或 instance。终端新增 info/search，wait 在客户端轮询状态或输出序号，超时退出码 2。

原生 UI 在创建文件 Manager 后给同一鉴权通道注册限定的 `files.*` 扩展；仅允许 list/info/ls/stat/mkdir/rename/remove/upload/download/transfers/cancel-transfer/result/cancel，不暴露任意 Host 方法。独立 SFTP 窗口无需终端也可发现文件会话。generation 由 Remote 对象的随机 UUID 与认证尝试代次组成；替换对象或重连都会使旧绑定失效。操作在服务端验证代次并持有当时的 Client，不在执行时重新查找新连接。

ls 使用同一目录读取与序号保留逻辑同步页面，CLI 返回有界分页，最新导航请求决定页面位置。传输使用同一并发队列和进度，外部完成记录独立保留，不被 GUI 清空或最近 20 条列表裁剪丢弃。默认 no-overwrite，显式目录递归；上传无覆盖提交使用 SFTP v3 rename，下载无覆盖提交使用同目录硬链接的排他创建，不因检查后出现同名目标而覆盖它。显式覆盖沿用 GUI 的服务器兼容替换策略。

所有文件工作异步运行，接入线程不会等待长 SFTP 请求。每个文件 request-id 保存动作、完整参数和原目标；同参重试返回原结果，异参拒绝。文件 ID 表最多 4096 项，不自动淘汰再重跑。SFTP 连接错误返回结果未知；协议明确拒绝和本地检查失败返回 failed，递归失败/取消不承诺撤销已完成的子项。元数据请求不能安全中断，取消接口只中断传输。协议依然为 v1，ping capabilities 包含 terminal/files；不带文件扩展的旧版应更新后重启。

原生内置 Agent 引擎退出只失效自身 task，保留外部智能体的已批准/排队/运行操作；broker 测试验证任务之间互不撤销。SFTP 请求超时会将客户端标记失效并关闭对应通道，避免未知请求之后继续发新写入；必须显式重新连接后取得新 generation。

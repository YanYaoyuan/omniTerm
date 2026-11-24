# 定位与恢复

仅在连接、绑定或结果异常时读取此文。恢复不能扩大原任务的目标与权限。

## CLI 或实例不可用

- 定位脚本只查找 CLI，不创建连接或读写资产。`--locate` 能区分 CLI 路径问题与后续实例问题；`OMNITERM_CLI_ENTRY` 为明确的 JS 入口路径，不是含参数的命令字符串。
- 找不到实例时核对实际桌面启动器。旧进程不能加载新接口，更新二进制后需从真实安装位置重新打开窗口；不要未经用户同意关闭仍在工作的资产会话。`omniterm` 同名命令也可能是 GUI。
- 桌面与 CLI 应以同一用户、在同一操作系统及可共享文件系统中运行。WSL/容器/远程开发机需在 omniTerm 所在系统调用 CLI，仅转发端口不能建立同用户授权。

发现目录默认是用户数据目录的 `automation/`：Linux `~/.config/omniTerm/automation`（遵循绝对的 XDG_CONFIG_HOME）、macOS `~/Library/Application Support/omniTerm/automation`、Windows `%APPDATA%/omniTerm/automation`。两端都遵循绝对的 `OMNITERM_DATA_DIR`，更优先的 `OMNITERM_AUTOMATION_DIR` 直接覆盖发现目录；`OMNITERM_DISABLE_AUTOMATION=1` 启动桌面会关闭入口。不要打印或分享发现文件中的 token。

## 绑定失效

实例 UUID 属于单个进程；终端 epoch、文件 generation 在重连/换连接后失效。重新列出并核对原资产，创建新的绑定文件。不能为绕过拒绝而编辑旧文件中的代次，更不能悄悄换到另一个窗口。原进程仍在时，可按原 instance 和 request-id 查询原操作；绑定本身不恢复已经消失的连接。

## 查询操作结果

| 情况 | 下一步 |
| --- | --- |
| queued / running，或等待退出码 2 | 用原 request-id、原 instance 查询 `session result` / `files result`。无需重发 exec/upload 等动作。 |
| outcomeUnknown / takenOver、网络断开或客户端被中断 | 命令/文件操作可能已经发生。先查询原 ID，再按任务读取输出或 stat/ls、核对文件；结果未确认前不换新 ID 重做。 |
| completed | 终端检查远端 exitCode；文件检查落点及 skipped。 |
| failed / cancelled | 查看具体错误及实际状态。递归任务可能已有部分子项完成，取消不保证整体回滚。 |
| 未知 request-id / 原进程已退出 | 不能跨重启恢复去重记录；先核实真实状态，再决定是否开始新操作。 |

同一实例内同一个 ID、动作及完整参数返回原操作；改参数会拒绝。查询时保留 CLI JSON 中返回的 instance/requestId（包括 stderr 中的记录），不要把查询失败当成任务未执行。

CLI 退出码：0 完成或读/发送成功；1 请求失败、阻止或取消；2 仍在进行或结果未知。终端另有 3，表示命令完成但远端退出码非零。不要让执行工具因退出码 2 自动重跑有副作用的操作。

`cancel` 只提出中断/取消请求，再查最终状态；任务完成太快时仍可能是 completed。已派发的 SFTP 元数据请求不能安全中断，会拒绝取消并提示查询。SFTP 请求超时会使当前 Client 失效，显式重连后取得新 generation。

记录只保存在当前进程内。终端底层保留最近约 200 个已结束操作，旧结果被回收时仍不会重发原命令；每实例最多 4096 个外部 exec ID。文件操作最多 4096 个 ID，UI 清空传输历史不清除 CLI 结果。达到上限时先结束当前工作，再安排重新启动。

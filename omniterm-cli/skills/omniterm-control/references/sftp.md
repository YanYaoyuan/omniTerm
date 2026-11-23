# 操作 SFTP 页面

`omniterm files` 复用页面已有的 SFTP 连接、目录状态和传输队列，覆盖网页授权的独立文件窗口与 SSH 终端附属文件面板。不会通过终端执行 shell，也不重新索取/保存网页密码或票据。仅操作用户任务授权的文件和目录。

## 选择并绑定文件会话

```sh
omniterm files list --instance INSTANCE_ID
omniterm files bind FILE_SESSION_ID --generation GENERATION --instance INSTANCE_ID --out files.json
omniterm files info --target files.json
```

清单的 `ref` 包含 `fileSessionId` 与 `generation`。用 `label`、`endpoint`、`terminalRef` 确认目标；附属面板可按 terminalRef 与已选资产终端对应，独立网页 SFTP 窗口可能没有 terminalRef。不要仅按相同主机名猜测资产，尤其是堡垒机多资产共用网关的情况。清单只列已经存在的文件连接；没有页面时让用户打开对应 SFTP 页面或在窗口中完成登录。

generation 包含连接身份与认证尝试代次。页面更换连接、断开后重连或进程重启会使旧绑定失效；重新确认后使用新的目标文件。不要把终端绑定文件交给 files，也不要把文件绑定交给 session。认证未完成时在窗口中处理密码/MFA，CLI 不自动重连。

## 浏览和元数据

```sh
omniterm files ls --target files.json --path /home/user --limit 200
omniterm files ls --target files.json --path /home/user --offset 200 --limit 200
omniterm files stat --target files.json --path '/home/user/report.txt'
```

`ls` 导航实际页面；`info` 给出页面状态及当前目录的有界快照。操作结果在 `operation.result` 中。目录列表含 `totalEntries`、`offset`、`truncated`、`nextOffset`；用 nextOffset 继续读取，不把第一页当作整个目录。每次新的 ls 是新的目录观察，不承诺跨多次请求期间目录不变。

所有远端路径必须绝对；从 info 的 path/home 或已确认的目录确定路径。不要把本地 shell 的 `~`、变量或相对路径直接当成远端路径。路径通过协议字符串发送，无需添加 shell 转义；本地调用命令仍需正确引用带空格的参数。

## 上传、下载和目录

```sh
omniterm files upload --target files.json --local ./report.txt --remote /home/user/report.txt --request-id upload-001
omniterm files download --target files.json --remote /home/user/report.txt --local ./report-copy.txt --request-id download-001
omniterm files upload --target files.json --local ./folder --remote /home/user/folder --recursive --request-id upload-tree-001
```

目标路径是完整精确路径，包含最终文件/目录名，不会自动追加源文件名。本地路径由 CLI 转为绝对路径，必须能在 omniTerm 进程所在文件系统访问；容器/WSL 中同名路径不等于宿主机同一文件。

默认拒绝覆盖已有目标；只有任务确实需要替换时才传 `--overwrite`。目录传输需要 `--recursive`；覆盖目录时合并其内容、允许替换其中的同名文件。源根部符号链接不接受，递归中链接/特殊文件和过深项目可能跳过。检查 `skipped`，非零时不要声称完整复制。传输使用临时文件和现有队列，页面显示进度。下载的父目录须已存在；先明确本地落点。服务器不支持原子替换时，显式覆盖沿用应用现有的替换语义。

```sh
omniterm files mkdir --target files.json --path /home/user/new-folder --request-id mkdir-001
omniterm files rename --target files.json --path /home/user/a.txt --to /home/user/b.txt --request-id rename-001
omniterm files remove --target files.json --path /home/user/b.txt --request-id remove-001
omniterm files remove --target files.json --path /home/user/empty-folder --directory --request-id rmdir-001
```

rename 默认拒绝已有目标；`--overwrite` 只用于任务明确需要的替换。remove 删除一个文件/链接；`--directory` 只删除空目录，没有递归删除。删除和改名需要明确的非根路径，不通过 `.` 或 `..` 指代目标。文件变更会触发页面刷新。

## 跟踪和恢复

每个 ls/stat/文件变更/传输都有 request-id，默认自动生成；有副作用的操作先自行选定唯一 ID。`--wait-ms 0` 立即返回，默认等待 30000 ms：

```sh
omniterm files result upload-001 --target files.json --wait-ms 30000
omniterm files cancel upload-001 --target files.json
omniterm files transfers --target files.json
omniterm files cancel-transfer --target files.json --transfer-id TRANSFER_ID
```

`operation.status` 为 queued/running 时继续查询；completed 表示 SFTP 操作或传输完成；failed/cancelled 需要检查实际结果；outcomeUnknown 表示连接已断、结果无法确认，不能盲目重跑。CLI 退出码：0 完成/读取，1 失败/取消，2 进行中或结果未知。

同一实例内，同一个 ID、动作和完整参数只返回原操作；更改动作或参数拒绝。同一实例最多 4096 个文件操作 ID，重启后不保留。UI 清除已完成传输不清除 CLI 保存的结果；即使页面重连，仍可按原 ID 查询原操作，不能改用新 ID 自动重试。传输取消仅提出取消请求；之后轮询 result，完成太快时可能仍返回 completed。已派发的 mkdir/rename/remove 等协议请求不能安全中断，会拒绝取消并提示查询。

连接断开、超时、客户端中断后，通过 result 和 stat/ls 或下载校验实际状态，再决定下一步。failed 的递归任务可能已有部分文件完成；不要把整个目录任务当作事务回滚，也不要未经判断重传或覆盖已有数据。

transfers 列出页面现有传输（包括手动发起的项目，最多最近 200 项），info 也包含此队列。cancel-transfer 以文件会话绑定和该会话内部的 transferId 取消页面任务；不要把另一文件会话的数字 ID 混用。结果查询仍优先使用自己记录的 request-id。

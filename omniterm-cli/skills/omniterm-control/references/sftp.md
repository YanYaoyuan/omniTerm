# SFTP 页面

`omniterm files` 复用既有页面、连接与传输队列，覆盖独立网页文件窗口及 SSH 附属面板。没有页面或认证未完成时，由用户在窗口打开/登录；CLI 不创建连接，也不通过独立 SFTP 请求 shell。

## 选择和绑定

```sh
omniterm files list --instance INSTANCE_ID
omniterm files bind FILE_SESSION_ID --generation GENERATION --instance INSTANCE_ID --out files.json
omniterm files info --target files.json --limit 50
```

占位符替换成清单值。`ref.fileSessionId` 和 `ref.generation` 固定文件连接；用 `label`、`endpoint` 和 `terminalRef` 确认资产。附属面板可按 terminalRef 与资产终端对应，独立窗口可能没有 terminalRef。堡垒机网关相同不意味着资产相同。

后续使用 `--target files.json` 代替 ID/generation/instance。绑定类型不可与终端混用。重连/更换连接后重新核对并创建新绑定；异常处理见 [recovery.md](recovery.md)。

## 浏览和元数据

```sh
omniterm files ls --target files.json --path /home/user --limit 50
omniterm files ls --target files.json --path /home/user --offset NEXT_OFFSET --limit 50
omniterm files stat --target files.json --path '/home/user/report.txt'
```

`ls` 导航实际页面；info 是当前页面快照。目录结果在 `operation.result` 中，检查 `totalEntries`、`offset`、`truncated`、`nextOffset` 并按需续读。新 ls 是新的观察，不保证多页读取期间目录不变。

远端路径必须绝对，从 info 的 path/home 确定；不把本地 `~`、变量或相对路径当作远端路径。路径通过协议发送，本地调用命令仍需引用空格等字符。

## 传输与文件变更

下面 request-id 是占位符，每个新操作替换成新的唯一 ID：

```sh
omniterm files upload --target files.json --local ./report.txt --remote /home/user/report.txt --request-id NEW_UPLOAD_ID
omniterm files download --target files.json --remote /home/user/report.txt --local ./copy.txt --request-id NEW_DOWNLOAD_ID
omniterm files upload --target files.json --local ./folder --remote /home/user/folder --recursive --request-id NEW_TREE_ID
```

目标为包含名字的精确路径，不追加源名。本地路径由 CLI 转成绝对路径，必须在应用所在文件系统可访问；下载父目录须已存在。

默认拒绝覆盖；任务明确需要替换时才传 `--overwrite`。目录需 `--recursive`；覆盖目录会合并并允许替换同名文件。源根部链接不接受，递归中的链接/特殊文件或过深项目可能跳过，检查 `skipped`。不要把 failed/cancelled 的目录任务视作整体回滚。显式覆盖沿用 GUI 的服务器兼容替换语义，并非所有服务器都保证原子替换。

```sh
omniterm files mkdir --target files.json --path /home/user/new-folder --request-id NEW_MKDIR_ID
omniterm files rename --target files.json --path /home/user/a.txt --to /home/user/b.txt --request-id NEW_RENAME_ID
omniterm files remove --target files.json --path /home/user/b.txt --request-id NEW_REMOVE_ID
omniterm files remove --target files.json --path /home/user/empty-folder --directory --request-id NEW_RMDIR_ID
```

rename 默认拒绝已有目标，需要替换时显式 `--overwrite`。remove 只删除一个文件/链接；`--directory` 只删空目录，没有递归删除。删除/改名使用明确非根路径，不用 `.` / `..`。变更同步刷新页面，删除/改名当前目录后页面跟随导航。

## 队列和结果

```sh
omniterm files result ORIGINAL_REQUEST_ID --target files.json --wait-ms 30000
omniterm files cancel ORIGINAL_REQUEST_ID --target files.json
omniterm files transfers --target files.json
omniterm files cancel-transfer --target files.json --transfer-id TRANSFER_ID
```

ls/stat/变更/传输均有 request-id，默认自动生成；有副作用的操作先保留自己的唯一 ID。`--wait-ms 0` 立即返回，默认等待 30000 ms。查询原 ID 取得 `operation.status` 和 bytes/total/skipped，不重发原动作等待结果。

transfers/info 包含页面队列（含手动项目，最多最近 200 项）。cancel-transfer 仅取消该文件会话中的数字 transferId，不混用另一会话的 ID。超时、未知结果、取消与进程重启的恢复规则见 [recovery.md](recovery.md)。

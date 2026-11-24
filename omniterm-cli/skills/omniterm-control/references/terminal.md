# 资产终端

先发现实例并确认目标；多实例时清单和 bind 都使用相同 `--instance`：

```sh
omniterm session instances
omniterm session list --instance INSTANCE_ID
omniterm session bind SESSION_ID --epoch EPOCH --instance INSTANCE_ID --out asset.json
omniterm session info --target asset.json
omniterm session read --target asset.json --lines 80
```

占位符替换成清单里的实际值。`ref.sessionId` 和 `ref.epoch` 标识连接；用 `label`、`kind`、`connection.configured` 和必要输出确认资产。初始连接信息不能证明嵌套 SSH / su / tmux 后实际所在机器。

绑定文件固定实例和连接代次，不含控制 token。`--target` 代替 ID/epoch/instance，不与另一个选择器混用。bind 独占创建文件；新绑定使用新文件名。旧绑定或进程重启时见 [recovery.md](recovery.md)。

## 就绪与执行

`info` 中 `interaction.state == "ready"` 且 `autoSubmit == true` 才能 tracked exec。只在确认空的 bash 4.4+ / zsh 主提示符下运行 setup：

```sh
omniterm session setup --target asset.json --shell bash
omniterm session wait --target asset.json --until ready --wait-ms 10000
omniterm session info --target asset.json
```

zsh 用 `--shell zsh`。setup 修改当前 shell 的提示符钩子和探测键绑定，为该 SSH 会话开启自动执行，不写启动配置；需 emacs 键位。已就绪且可自动执行时不重复发送脚本。不要在密码/MFA、编辑器、运行中的程序、未提交输入或续行提示符下 setup。状态或 shell 不明确时由用户在窗口回到空提示符；其他 shell 只有显式输入，不具备相同的完成追踪。

为每条新命令生成新的唯一 ID，下面 ID 仅作示例，不能跨任务复用：

```sh
omniterm session exec --target asset.json --request-id NEW_REQUEST_ID --command 'hostname; id; pwd'
omniterm session exec --target asset.json --request-id ANOTHER_NEW_ID --command-file ./task.sh --wait-ms 0
omniterm session result ANOTHER_NEW_ID --target asset.json --wait-ms 30000
omniterm session cancel ANOTHER_NEW_ID --target asset.json
```

命令在该终端当前 shell 中执行，保留目录和环境。整个远端命令需对本地调用 shell 正确引用，防止本机先执行变量、`$(...)` 或重定向；复杂脚本优先 UTF-8 `--command-file`。

只在 `operation.status == "completed"` 时解释 `operation.exitCode`：零成功，其他数字为远端命令失败。`output.lines` 是合并的 PTY 文本，可能含回显/提示符。`--wait-ms` 默认 30000，零立即返回；`--timeout-ms` 默认 120000，范围 1000–900000，超时不保证命令停止。

## 读取与交互

```sh
omniterm session search --target asset.json --query 'error' --matches 20
omniterm session read --target asset.json --since LAST_SEQ_END --lines 80
omniterm session wait --target asset.json --until output --since LAST_SEQ_END --wait-ms 10000
```

保存 read 的 `seqEnd` 用于下一次 `--since`。read 最多 2000 行；检查 `truncated`、`omittedLines`、`missingBytes`。等待到输出不代表命令完成；无新输出可返回空 lines。缓冲区有界并沿用应用脱敏规则；大文件使用 SFTP。

仅在用户任务需要键盘交互、且已确认当前程序时：

```sh
omniterm session write --target asset.json --text 'y' --enter
```

不加 `--enter` 只输入文本；`--text-file` 从 UTF-8 文件读取。write 无 shell 就绪检查和 request-id 去重，`sent`、`completionKnown:false` 不证明命令运行或成功。发送后读取确认状态，不为确认而重复写入。

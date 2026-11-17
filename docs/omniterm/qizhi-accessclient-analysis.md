# 齐治 AccessClient 4.6.20：Windows 安装包静态分析

分析对象是用户提供的 `AccessClient.exe`，24,413,328 字节，SHA-256：
`7dfb2505e5912c065cff3c331e8fb9f5b5f14eed06eaec649b64befde7410b3d`。
只通过发行版的 innoextract/objdump 解包和静态分析，未运行安装包或其子程序。
厂商二进制、反汇编及解包内容仅放在被 Git 忽略的本地缓存中，不纳入项目分发。

## 已确认的接口线索

- 安装程序标识 `AccessClient 4.6.20`，使用 Inno Setup 5.5.7。
- 网页唤起前缀为 `accessclient://`。LoaderShell 的前缀处理位于 VA `0x424cd0`，
  跳过 15 个字符；也处理 Windows 命令行外围的双引号。
- 解码路径从 VA `0x4226e0` 调用 URL 百分号解码、Base64 解码、zlib 解压和 JSON 解析。
  URL 解码函数位于 VA `0x427240`，Base64 分派位于 `0x41c750`，
  zlib 调用位于 `0x4227dc`；原程序的解压缓冲区大小是 2 MiB。
- 会话请求路径可见 `host`、`port`、`use_ssl`、`url_path`、`header`、`is_batch`、`batch_url`，
  以及 `opc_host`、`opc_port`、`opc_use_ssl`、`scsession_url_path`、`opc_header` 等键。
  这些键不是固定的公司服务地址；不能据此猜测接口路径或使用过期票据测试。
- 有 `proxy_protocol=websocket`、`guiservice_token`、`sessionData` 等代理线索。
  包内有 `sessproxy-client.exe` 和 `tcp2ws.exe`，Go 符号显示 WebSocket/TCP 转换逻辑。
- LoaderShell 中有 PuTTY、Xshell、SecureCRT、MobaXterm、WinSCP、FileZilla、Xftp、
  RDP 和 Java 图形会话的分派；SSH/SFTP 不是全部功能的同义词。
- LoaderShell 的 UTF-16 参数表还包含短字段名：`hn` → `HostName`、`pn` → `PortNumber`、
  `un` → `UserName`、`pw` → `PWD`、`pr` → `Proto`、`env` → `Environment`、
  `ws` → `websid`，以及 `sn/sh/sa/st` 对应的会话名称、主机、账号和标题。
  这些来自安装包中的静态表，不是根据公司接口猜测的名称；数字协议枚举仍不作推断。

以上说明可以继续研究 Ubuntu 的原生接入层，不代表代理握手、票据换取或所有会话类型已经兼容。
只靠在本地运行普通 `ssh 堡垒机IP`，也不能等同于复现网页发放的资产会话。

## 离线诊断接口（早期实现）

原生版新增 `--inspect-accessclient`，从标准输入读一个启动链接，离线解码后只输出：

- 根结构、内容字节数、根成员数；
- 固定白名单中出现过的字段名、JSON 类型、非空字段名集合，以及未知字段数；
- 固定白名单中的客户端名称和协议提示；
- 是否识别到非空认证材料、会话请求或非空代理字段的布尔值；
- 已验证的固定票据前缀标签（例如 `otp1-ticket`），不包含票据本身。

结构报告版本 2 补齐了上述短字段。未识别到认证字段不代表启动请求没有认证材料；
同样，空的 `proxy.addr` 字段不证明本次会话需要代理。字段类型和非空标记用于避免这种误判。

不输出主机、账号、路径、密码、票据、请求头、未知键或其他任意值。
编码输入和解压结果都有大小限制，错误信息也不会回显输入。
不会请求堡垒机、启动会话、执行 JSON 中的程序路径或运行 Windows 程序。
输入 URL/Base64/zlib/JSON 的畸形值和压缩炸弹有回归测试。

另有 `scripts/accessclient-inspect.py` 作为可选的临时 URI 接收器。
它不会自动注册为系统默认程序；启用前应确认当前 `accessclient://` 处理器并征得用户同意。
浏览器会通过 URI 启动参数把原始链接交给接收器；接收器只将其经 stdin 交给原生解码器，
不再复制到子进程参数或环境变量。默认落盘的只有上述脱敏结构，文件权限为 0600。

用户随后明确要求将完整连接参数保存到本机，因此另增加一次性捕获开关：
在本机状态目录创建 `accessclient-capture-next.json`，内容为 `{"enabled":true}`，
仅下一个已验证的 WinSCP、FileZilla 或 Xftp 请求会将完整解码 JSON 写入
`accessclient-connection-parameters.json` 的 `parameters` 字段。文件可能包含认证材料，
权限为 0600、父目录为 0700，不在仓库内，不上传，不在诊断或进程输出中回显。
保存成功即消耗开关；后续请求不再保存或覆盖。无效请求与 SSH 请求不会消耗开关。
这项显式保存不建立资产连接，也不将临时认证材料写入普通连接配置或系统钥匙串。

另支持一次性的 SSH 路由对照：在状态目录创建 `accessclient-compare-next.json`，
用 `enabled: true` 和 `expected` 指定已保存连接的 `gatewayHost`、`gatewayPort`、
`routeUser`、`gatewayUser`、`assetHost`、`assetUser`。下一个有效字符客户端请求会消耗此开关，
对照 `hn/pn/un/sh/sa`，只在 `accessclient-route-comparison.json` 写入匹配布尔值、
字段缺失标记、是否包含会话引用及认证材料。不会保存请求中的主机、账号、密码、票据或链接，
不会比较或读取钥匙串密码，也不发起连接；字段的实际认证语义仍需另行确认。
无效请求和文件客户端请求不消耗该开关，后续字符请求不会覆盖已有对照结果。

## 早期联调记录

1. 用户选择临时启用诊断接收器后，在网页手动点击一次资产启动。
2. 只查看本地脱敏诊断，确认实际使用的请求/代理分支，不要求用户在聊天发送原始链接。
3. 依据真实结构实现会话请求和 SSH/SFTP 适配，并用隔离服务验证生命周期、票据处理和取消。
4. 通过公司环境验收后，再提供正式 Ubuntu 唤起入口；图形协议另行实现和验收。

本机临时接收器已在用户明确同意后注册，并使用完全虚构的数据验证了系统 URI 分派。
原来的 MIME 关联已备份；目前仍只生成脱敏诊断，尚未打通网页到资产的完整连接。

首次实际网页唤起已经成功解码为 22 字段的对象，客户端标签为 `putty`。
旧报告只识别到 3 个字段，无法据此判断认证和代理分支。由于没有保存原始链接，
需要在更新到版本 2 的诊断器后重新点击网页启动，不能从旧报告恢复票据或重新解析。

第二次真实唤起已使用版本 2：识别了 22 字段中的 21 个，包含非空的 `hn/pn/un/pw/ws`，
`env` 为空，`proxy.addr` 与 `control_mode` 为 null。`pw` 没有匹配 `OTP1:` 固定前缀；
不能据此认定其为可直接提交给 SSH 的明文密码，也不能假定本次需要 WebSocket 代理。
同日另检查了用户提供的透传配置导出，确认存在标准 SSH 的三段路由用户名入口，
可以独立于网页票据分支继续接入和验收。详见 `bastion-ssh.md`。

## SFTP 网页唤起的实际结构

用户在文件传输配置选择 WinSCP 后点击 SFTP 启动，临时接收器成功得到新报告：
对象共 18 个字段，客户端标签为 `winscp`，含非空的 `host`、`port`、`user`、
`pwd` 与 `PWD`。`proxy.addr`、`control_mode` 为 null；6 个未知字段未输出名称或值。
这证明已接收到独立的文件传输启动请求，不证明已完成服务器认证或文件访问。

静态核对 LoaderShell：FileZilla 的 URL 构造分支（约 VA `0x42d500–0x42dd10`）
与 WinSCP 分支（约 VA `0x45ac00–0x45b3d0`）读取 `host/proto/port/path/user/pwd`，
协议为空时使用固定的 `sftp`。小写 `pwd` 经 URL 编码函数 `0x459960` 处理后加入
会话地址；WinSCP 另拼接 `/sessionname=`。没有运行这些客户端，也没有读取票据值。

因此可将已验证的连接字段交给 omniTerm 内置 SFTP，而不要求 Ubuntu 安装这些 Windows
客户端。网页启动适配仍需实现独立的文件连接、仅在内存传递的临时认证材料和会话生命周期，
并在本地隔离服务与实际授权环境分别验证。不能把有权限的 SFTP 请求等同于有权限的 Shell
或主机监控，也不能因为交互 SSH 拒绝增加通道而假定独立的文件传输入口同样不可用。

## 2026-11-17：字符启动请求与手工透传的对照

用户确认同一资产、同一系统账号和 SSH 端口的新 Web 会话可用。一次性对照收到新的
PuTTY 字符启动请求，确认 `hn/pn` 匹配已保存的网关入口，`sh/sa` 匹配目标资产和账号；
`un` 等于裸堡垒机账号，不等于原生连接所用的三段路由用户名。请求含非空 `pw` 和 `ws`，
`env` 为空，未声明代理。对照仅保存布尔值，没有保留或重放认证材料，也未建立 SSH 连接。

这证明两种启动方式的参数不同，不能证明 `pw` 的实际认证语义、有效期或编码方式，
也不能单凭此差异解释此前的资产连接失败。独立系统 SSH 检查曾在认证与终端通道建立后
收到服务器的账号锁定提示；该提示不证明 Web 账号整体被锁，更不证明资产不可达。
PR #21 的手工账号透传和现有诊断 URI 处理器尚未实现消费网页 SSH 启动参数的连接链路。
下一次实际接入验证应使用新发放的参数，避免复用旧请求或反复尝试保存的固定密码。

## 本机原生版：网页拉起 SSH 与 SFTP 窗口

后续工作副本加入 `--open-accessclient`：启动链接从标准输入传入，支持已观测的
`app=putty`、`hn/pn/un/pw/ws/sh/sa` 参数。Ubuntu 的 `accessclient://` 处理器可指向
`scripts/accessclient-open.py`；选择 PuTTY 后点击资产，实际打开原生 OmniTerm 独立 SSH 窗口。
WinSCP 类型的文件会话同样通过此入口打开独立 SFTP 文件窗口，沿用目录浏览、上传下载、
覆盖确认、传输队列和取消功能，不创建 SSH 终端或监控通道。读取网页文件入口的
`host/port/user` 和小写 `pwd`，沿用导入适配器的字段冲突、协议和代理检查；大写 `PWD`
不能替代缺失的小写凭据。不需要保存或导入导出 JSON，网页材料不成为永久连接配置。
其他客户端类型继续交给离线诊断器，不运行网页指定的客户端程序、命令或 URL。

网页认证字段只交给该连接第一次匹配的 SSH 密码提示，不写入系统钥匙串或连接配置；
额外验证码仍需手动输入。会话引用通过私有 SSH 配置中的 `SHTERM_WEBSID` 传递。
认证内容不进入后续进程的命令行、环境变量或终端输入；临时文件使用私有目录和 0600 权限，
首次读取后删除凭据，SSH 结束或窗口关闭后清理其余认证状态。
网页窗口不恢复或保存主窗口工作区；SSH 窗口不启动共享连接、附属 SFTP 或主机监控。
参数已使用时必须从网页重新启动，不把旧授权当作可重连的永久配置。

本机验证包含启动链接 stdin 边界测试、参数与生命周期单测，以及真实 OpenSSH + 回环 SSH
服务器验证一次性密码和会话环境引用。该结果不替代公司堡垒机的实际接入验收，也不代表
带 WebSocket 代理、二次 HTTP 取票、非空扩展环境或其他字符客户端格式已实现。
SFTP 增量验证还包括真实 OpenSSH + 隔离 SFTP 服务的目录读取和上传下载往返，确认一次
认证、一个 SFTP 子系统、没有 shell/exec 请求；实际原生文件窗口在 Xvfb 中显示目录并正常关闭。
公司文件入口的目录读取及传输仍需使用新发放的网页参数实测。


## Ubuntu URI 接收器安装

将构建后的 `omniterm-app`、`scripts/accessclient-open.py` 和
`scripts/accessclient-inspect.py` 放在同一个安装目录。在用户桌面文件目录添加
`x-scheme-handler/accessclient` 的处理器，`Exec` 指向该目录中的
`accessclient-open.py %u`（也可显式使用 `/usr/bin/python3`），然后用
`xdg-mime default <desktop-id> x-scheme-handler/accessclient` 选择它。
接收器不会自行修改 MIME 关联；应按用户选定的安装目录生成 desktop 文件并备份旧关联。
从网页选择 PuTTY 打开字符窗口，选择 WinSCP 打开 SFTP 窗口。每次使用新发放的启动链接。

浏览器传给操作系统的原始链接会出现在接收器的初始参数中，这是系统 URI 分派边界；
接收器之后只经 stdin 传递，不将链接或票据复制到原生应用的 argv、环境变量或日志。
当前支持不带额外代理的已观测字段组合；非空代理、扩展环境、HTTP 换票和其他协议
仍需要单独适配。macOS/Windows 的 URI 注册尚未随此 Linux 接收器实现。

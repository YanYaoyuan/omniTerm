# BYTE · Little moments

先完成角色图画，再实现原生交互。创意方向是「小动物陪伴 + 一个隐藏小游戏」。MobaXterm 的[企鹅屏保与小游戏](https://mobaxterm.mobatek.net/documentation.html)提供了终端也可以有趣的灵感；BYTE 的形象、素材和交互为 omniTerm 新设计。

![角色设定](../../../omniterm-native/assets/easter-eggs/byte-concept.png)

## 图画

奶油色小猫、藏蓝轮廓、橘色脸颊，蓝色 `>_` 围巾。三种互动姿态：抱着笔记本打盹、抬爪回应摸摸、跳起接住小鱼干数据包。另有四帧奔跑循环，用于桌面追逐。

透明素材为 [byte-sprites.png](../../../omniterm-native/assets/easter-eggs/byte-sprites.png)，运行时按透明间隔取三帧。图片只解码一次，上传前按渲染器的最大纹理尺寸适配；深浅主题共用透明底，卡片和文字使用当前主题色。

设计图与透明素材由内置 image_gen 生成，提示词见[素材记录](../../../omniterm-native/assets/easter-eggs/prompts.md)。未使用 CLI/API fallback。

## 发现与互动

- 连续点击标题栏品牌五次，进入「BYTE 的小憩角落」。也可在命令面板搜索 `彩蛋`、`BYTE`、`摸鱼` 或 `snake`。
- 点猫咪或「摸摸脑袋」，小猫抬爪回应；「投喂小鱼干」展示追逐动作；可以让它回去打盹。
- 发现后状态栏出现一个小猫入口，右键可隐藏。发现状态、静态插画选项和贪吃蛇最佳成绩保存在原有 UI 状态文件中。
- 数据包贪吃蛇：方向键/WASD 转向，空格开始/暂停，按钮也可操作，Esc 关闭。离开窗口或切换到猫咪页会暂停。

## 整个桌面的追逐

在猫咪页点击「放到桌面追鼠标」，BYTE 会离开小憩面板，成为独立的透明置顶桌宠。鼠标移到其他应用或 omniTerm 窗口外，它也会追过去；主窗口失焦、最小化后仍继续。靠近鼠标约 95 个逻辑点时停下，转向时翻转图画，闲置后打盹。

- 点击穿透整块桌宠窗口，不抢键盘焦点、不改变鼠标位置；暂停和收回在小憩面板、状态栏小猫的右键菜单中。
- 关闭面板可以继续正常使用终端。主窗口最小化时产生的字形上传保留到恢复后的 GPU 绘制，避免最小化期间执行 CLI 后丢失文字图集。关闭 omniTerm 会同时关闭桌宠；重启后不会自动放出。
- 使用独立 deferred viewport，只刷新小猫窗口；奔跑约 30 fps，停下后约 10 Hz 读取鼠标，暂停时不再轮询。
- Windows 使用 `GetCursorPos` / 虚拟桌面范围；macOS 使用 Quartz 全局位置与显示器范围；Linux 使用 X11 `QueryPointer`。没有键盘钩子、事件注入或远程连接操作。
- Linux 需要 X11 和窗口合成器。Wayland 不提供此实现所需的全局坐标与任意定位，界面会说明暂不支持。Windows/macOS 代码由各平台 CI 编译检查；真实 GUI 验证目前在 Linux X11 完成。
- [byte-run.png](../../../omniterm-native/assets/easter-eggs/byte-run.png) 是根据原猫咪生成的透明奔跑图集；提示词和来源在素材记录中。

![桌面追逐（隔离 X11 桌面）](byte-desktop-preview.png)

## 与工作区相处

- 游戏只有独立的本地状态，没有后端、SSH、SFTP 或文件操作接口。
- 浮层打开时主终端及停靠终端不接收键盘输入；关闭帧仍拦截输入，直到已按下的键全部松开。
- 游戏控制键先消费，再绘制按钮，避免空格同时暂停和激活按钮。
- 未放出桌宠时，浮层关闭后没有动画刷新定时器。静态插画选项控制面板呼吸动画；桌宠可单独暂停。
- 小窗口使用滚动区域容纳内容，保留退出按钮；游戏食物只生成在空格，满盘后进入完成状态。

## 原生窗口预览

![喂食互动](byte-cat-preview.png)

![数据包贪吃蛇](byte-snake-preview.png)

![暗色主题](byte-dark-preview.png)

## 已完成的验证

- 原生单测：165 通过，12 项既有专用服务测试在普通单测中忽略。新增 13 项覆盖游戏成长、反向输入、腾空尾格、碰撞、暂停、满盘获胜、入口、透明素材、1024/2048 纹理上限、最小窗口控件可见性、空格消费及关闭时的按键释放保护，以及桌面追逐的停止距离、缩放、负坐标和边缘限制、从外部按钮重新打开面板时的点击保护。
- release 构建和 all-targets clippy（warnings denied）通过。
- 独立 Xvfb + 临时配置的真实原生窗口验证：五连击、命令面板搜索 BYTE、摸摸/喂食、亮暗主题、开始/暂停、得分、失焦暂停、正常退出和最佳成绩保存。
- 游戏期间按 WASD/空格/Enter、关闭时继续按住 W，终端输出序号保持 2127，增量输出为空；释放按键后 tracked exec 输出 AFTER_EASTER_OK，exitCode 为 0。
- 彩蛋仍打开时 tracked exec 输出 WORK_CONTINUES_OK，exitCode 为 0。测试仅使用隔离的本地 bash，无真实资产、凭据或模型请求。

- 桌宠 GUI 回归：真实 Openbox + xcompmgr + Xvfb 验证跨应用追逐、置顶、不抢焦点、主窗口最小化后继续、暂停/恢复、点击穿透、透明底、收回/再放出，以及退出时一起关闭。运行中的外部 CLI tracked exec 输出 `DESKTOP_PET_CLI_OK`，exitCode 为 0。

复现桌宠回归（只启动临时桌面和配置，不连接真实资产）：

```bash
python3 omniterm-native/scripts/test-desktop-pet.py \
  --binary omniterm-native/target/release/omniterm-app \
  --xvfb /path/to/Xvfb --compositor /path/to/xcompmgr \
  --wm /path/to/openbox --output /tmp/omniterm-pet-check
```

解包运行 Openbox 时，可用 `LD_LIBRARY_PATH` 指向解包的库目录、`XDG_DATA_DIRS` 指向解包的 `usr/share`，并通过 `--wm-config` 提供 `etc/xdg/openbox/rc.xml`。

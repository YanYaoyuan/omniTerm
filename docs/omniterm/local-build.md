# 从源码构建 omniTerm 的 Agent

omniTerm 的 Agent 基础位于本仓库的 `codex-rs/`。使用
[`scripts/build_codex_package.py`](../../scripts/build_codex_package.py)
编译并组装完整的本地 Agent 包，输出到 `dist/omniterm-agent/`。
Node.js 命令入口负责发现和启动这个包，编译工作由 Rust 工具链完成。

本文说明构建步骤与检查方式，不代表所有平台已经完成构建或端到端验收。
跨平台 launcher 测试与 Rust Agent 构建是独立的验证工作。

## 准备环境

- **Rust 1.95.0：** 版本及组件以
  [`codex-rs/rust-toolchain.toml`](../../codex-rs/rust-toolchain.toml) 为准。
  安装 rustup，并确保 `cargo`、`rustc` 在 `PATH` 中。
- **Python 3.11 或更新版本：** 用于运行打包脚本。
- **Node.js 22 或更新版本：** 用于通过 omniTerm 命令入口运行 Agent。
- **macOS：** 安装 Xcode Command Line Tools，确保 `xcrun` 和 C/C++ 编译器可用；
  原生依赖构建还需要 CMake。
- **Linux：** 准备目标平台的 C/C++ 编译器、链接器、CMake、`pkg-config` 和所需开发库。
  脚本默认选择 musl 目标，需要对应 Rust target 与 musl 构建环境；本机 glibc 构建见下文。
- **Windows：** 安装 Visual Studio C++ Build Tools、Windows SDK 和 CMake，
  在对应架构的 MSVC 开发环境中运行 PowerShell；脚本使用 MSVC target。

首次构建需要网络访问以获取 Rust 依赖和第三方构建资源。
为 Cargo 构建输出和依赖缓存预留足够磁盘空间。

## 构建本地包

在仓库根目录运行。macOS 或已配置相应目标工具链的 Linux：

```sh
export CODEX_REPO_ROOT="$PWD"
python3 scripts/build_codex_package.py --package-dir dist/omniterm-agent --cargo-profile dev-small
```

Windows PowerShell：

```powershell
$env:CODEX_REPO_ROOT = (Get-Location).Path
python scripts/build_codex_package.py --package-dir dist/omniterm-agent --cargo-profile dev-small
```

确保以上 `python3` 或 `python` 命令实际运行的是 Python 3.11 或更新版本。
`dev-small` 是本仓库的开发构建 profile。脚本在 `codex-rs/` 中调用 Cargo，
使用该目录固定的 Rust 工具链，并仅选择当前包所需的二进制目标。

Linux 默认目标为 `x86_64-unknown-linux-musl` 或
`aarch64-unknown-linux-musl`。在 x86_64 glibc Linux 上做本机构建时，可以明确指定：

```sh
export CODEX_REPO_ROOT="$PWD"
python3 scripts/build_codex_package.py --package-dir dist/omniterm-agent --cargo-profile dev-small --target x86_64-unknown-linux-gnu
```

ARM64 glibc Linux 对应 `--target aarch64-unknown-linux-gnu`。
`--target` 选择构建目标，所需编译器、链接器与 Rust target 仍需在本机准备。

若输出目录已经存在，脚本会拒绝覆盖。需要重新生成该构建产物时，在确认
`dist/omniterm-agent/` 仅包含可替换的构建输出后，加上 `--force`。

## 编译与下载的内容

未传入预编译覆盖参数时，脚本从仓库源码编译：

- `codex` 主程序。
- `codex-code-mode-host` 辅助程序。
- Linux 上的 `bwrap`，或 Windows 上的 `codex-command-runner.exe` 和
  `codex-windows-sandbox-setup.exe` 等平台资源。

Codex 主程序和 host 来自此次 Cargo 源码构建。网络获取的资源还包括 Rust
依赖，以及 V8 和 ripgrep 等第三方组件：macOS/Linux 打包器下载并校验匹配的
V8 库与绑定文件，Windows 沿用 Cargo 的 MSVC V8 构建资源路径；ripgrep
根据仓库清单下载并校验大小与 SHA-256。具体规则见
[打包器说明](../../scripts/codex_package/README.md)。

成功后，完整包包含以下关键文件；平台资源随目标系统变化：

```text
dist/omniterm-agent/
├── codex-package.json
├── bin/
│   ├── codex[.exe]
│   └── codex-code-mode-host[.exe]
├── codex-path/
│   └── rg[.exe]
└── codex-resources/             # 按平台包含辅助资源
```

保留整个包目录，主程序需要同包的辅助程序和资源。
`dist/` 是被 Git 忽略的构建输出，不随源码提交。

## 检查并启动

构建成功后，先直接检查本次产物。macOS/Linux：

```sh
./dist/omniterm-agent/bin/codex --version
```

Windows PowerShell：

```powershell
& .\dist\omniterm-agent\bin\codex.exe --version
```

然后从仓库根目录检查 omniTerm 选择的 Agent：

```sh
node omniterm-cli/bin/omniterm.js doctor
node omniterm-cli/bin/omniterm.js agent --version
node omniterm-cli/bin/omniterm.js agent
```

若未设置 `OMNITERM_CODEX_BIN`，launcher 优先发现本仓库的
`dist/omniterm-agent/bin/codex`（Windows 为 `codex.exe`）。显式设置的
`OMNITERM_CODEX_BIN` 优先级更高；检查 `doctor` 输出，确认实际运行的路径。
`doctor` 也检查 SSH 和串口等可选依赖，缺少这些工具会导致它返回 `1`，
不代表已构建的 Agent 不可用。

版本检查验证二进制可启动。模型认证、工具执行与真实 Agent 会话需要另外验证；
认证、模型和审批配置仍由所选 Codex 引擎管理。更多命令与依赖说明见
[CLI 指南](../../omniterm-cli/README.md)。

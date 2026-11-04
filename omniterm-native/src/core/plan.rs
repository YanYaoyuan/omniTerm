// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! How each kind of session starts, as the desktop app's `command-plan.mjs` decides it: the
//! program, its arguments and environment, and for SSH the shared login the file panel uses.
//! Arguments are always a list, never a shell string, so a profile value cannot become syntax.

use super::paths::find_executable;
use super::profile::{Forward, Kind, Profile};
use super::settings::LocalShell;
use serde_json::{Value, json};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long a shared SSH login outlives its last user, bounding it if omniTerm itself dies.
const CONTROL_PERSIST_SECONDS: u32 = 30;

pub struct Request<'a> {
    pub kind: Kind,
    pub cwd: &'a str,
    pub profile: Option<&'a Profile>,
    /// Whether an SSH session shares its login with the file panel and metrics.
    pub companion: bool,
    pub local_shell: &'a LocalShell,
}

/// What the file panel and metrics need to reach the same host.
#[derive(Clone, Debug, PartialEq)]
pub struct Companion {
    pub executable: String,
    pub destination: String,
    pub control_path: Option<String>,
    pub args: Vec<String>,
}

#[derive(Debug)]
pub struct Plan {
    /// The request for the terminal host's `session.create`.
    pub host: Value,
    pub title: String,
    pub powershell: bool,
    pub companion: Option<Companion>,
}

pub fn local_shell() -> Result<(String, Vec<String>, bool), String> {
    if cfg!(windows) {
        let comspec = std::env::var("COMSPEC").ok();
        for name in ["pwsh", "powershell"].into_iter().map(String::from).chain(comspec).chain(["cmd".to_string()]) {
            if let Some(path) = find_executable(&name) {
                let path = path.to_string_lossy().into_owned();
                let powershell = is_powershell(&path);
                return Ok((path, if powershell { vec!["-NoLogo".into()] } else { Vec::new() }, powershell));
            }
        }
    } else {
        for name in std::env::var("SHELL").ok().into_iter().chain(["/bin/sh".to_string()]) {
            if let Some(path) = find_executable(&name) {
                return Ok((path.to_string_lossy().into_owned(), vec!["-l".into()], false));
            }
        }
    }
    Err("找不到本地 Shell。请设置 SHELL（Unix），或安装 PowerShell / 设置 COMSPEC（Windows）。".into())
}

impl LocalShell {
    pub fn resolve(&self) -> Result<(String, Vec<String>, bool), String> {
        let program = self.program.trim();
        if program.len() > 4096 || program.chars().any(char::is_control) {
            return Err("Shell 程序路径过长或含有控制字符。".into());
        }
        if self.args.len() > 64 || self.args.iter().any(|arg| arg.len() > 8192 || arg.contains('\0')) {
            return Err("Shell 最多支持 64 个启动参数，每项最多 8192 字节且不能包含空字符。".into());
        }
        if program.is_empty() {
            let (program, defaults, powershell) = local_shell()?;
            return Ok((program, if self.args.is_empty() { defaults } else { self.args.clone() }, powershell));
        }
        let path = find_executable(program).ok_or_else(|| format!("找不到或无法执行自定义 Shell：{program}。请在设置中选择可执行程序，或清空程序以使用系统默认。"))?;
        // Resolve relative paths before the PTY changes into the terminal's directory.
        let path = std::path::absolute(path).map_err(|error| format!("无法解析 Shell 程序路径：{error}"))?;
        let program = path.to_string_lossy().into_owned();
        let powershell = is_powershell(&program);
        Ok((program, self.args.clone(), powershell))
    }
}

fn is_powershell(path: &str) -> bool {
    let name = Path::new(path).file_stem().and_then(|name| name.to_str()).unwrap_or_default().to_ascii_lowercase();
    name == "pwsh" || name == "powershell"
}

fn require(name: &str) -> Result<String, String> {
    find_executable(name)
        .map(|path| path.to_string_lossy().into_owned())
        .ok_or_else(|| format!("找不到 {name}。请安装 OpenSSH 客户端并确保它在 PATH 中。"))
}

pub fn python() -> Result<(String, Vec<String>), String> {
    if cfg!(windows)
        && let Some(environment) = std::env::var_os("VIRTUAL_ENV")
        && let Some(path) = find_executable(&Path::new(&environment).join("Scripts").join("python.exe").to_string_lossy())
    {
        return Ok((path.to_string_lossy().into_owned(), Vec::new()));
    }
    let names: &[&str] = if cfg!(windows) { &["python", "python3", "py"] } else { &["python3", "python"] };
    for name in names {
        if let Some(path) = find_executable(name) {
            return Ok((path.to_string_lossy().into_owned(), if *name == "py" { vec!["-3".into()] } else { Vec::new() }));
        }
    }
    Err("找不到 Python 3。串口连接需要 Python 3 和 pyserial。".into())
}

/// Whether Python can run pyserial's terminal; at most five seconds.
fn has_pyserial(executable: &str, args: &[String]) -> bool {
    let child = Command::new(executable)
        .args(args)
        .args(["-c", "import sys; assert sys.version_info.major >= 3; import serial.tools.miniterm"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return false };
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                return false;
            }
        }
    }
}

/// `user@host` for ssh, with IPv6 bracketed for sftp, which parses `host:path`.
fn destination(profile: &Profile, kind: Kind) -> String {
    let full = profile.host.as_deref().unwrap_or_default();
    let (user, mut host) = match full.rsplit_once('@') {
        Some((user, host)) => (Some(user.to_string()), host.to_string()),
        None => (None, full.to_string()),
    };
    let user = profile.username.clone().or(user);
    if kind == Kind::Sftp && host.contains(':') && !host.starts_with('[') {
        host = format!("[{host}]");
    }
    match user {
        Some(user) => format!("{user}@{host}"),
        None => host,
    }
}

fn session_env() -> serde_json::Map<String, Value> {
    let mut env: serde_json::Map<String, Value> = std::env::vars().map(|(key, value)| (key, Value::String(value))).collect();
    for (key, value) in [("TERM", "xterm-256color"), ("COLORTERM", "truecolor"), ("TERM_PROGRAM", "omniTerm")] {
        env.insert(key.into(), Value::String(value.into()));
    }
    env
}

/// Builds a session plan. May block for a few seconds (the pyserial check), so it runs on a
/// worker thread, never the UI thread. `control_path` names a fresh socket for a shared login.
pub fn build(request: &Request, control_path: impl FnOnce() -> Option<String>) -> Result<Plan, String> {
    let cwd = request.cwd;
    if !Path::new(cwd).is_absolute() || !Path::new(cwd).is_dir() {
        return Err(format!("工作目录不存在或不是目录：{cwd}"));
    }
    let profile = match request.profile {
        Some(profile) => Profile { kind: Some(request.kind), ..profile.clone() }.validate(true)?,
        None if request.kind == Kind::Shell || request.kind == Kind::Agent => Profile { kind: Some(request.kind), ..Profile::default() },
        None => return Err("这种连接需要连接配置。".into()),
    };
    let mut powershell = false;
    let mut companion = None;
    let mut connection = json!({});
    let (executable, args) = match request.kind {
        Kind::Shell => {
            let (shell, args, is_powershell) = request.local_shell.resolve()?;
            powershell = is_powershell;
            (shell, args)
        }
        Kind::Agent => {
            let agent = crate::agent::engine::resolve()?;
            let mut args = agent.args;
            args.extend(["--no-daemon".to_string(), "--no-alt-screen".to_string()]);
            (agent.executable, args)
        }
        Kind::Serial => {
            let (python, mut args) = python()?;
            if !has_pyserial(&python, &args) {
                return Err(format!("串口连接需要 Python 3 和 pyserial。请为 {python} 安装 pyserial（python -m pip install pyserial）。"));
            }
            args.extend(["-m".to_string(), "serial.tools.miniterm".to_string()]);
            args.push(profile.device.clone().unwrap_or_default());
            args.push(profile.baud.unwrap_or(115_200).to_string());
            (python, args)
        }
        Kind::Ssh | Kind::Sftp | Kind::Tunnel => {
            let shares_login = request.companion && matches!(request.kind, Kind::Ssh | Kind::Sftp);
            let socket = if shares_login { control_path() } else { None };
            let mut args = Vec::new();
            if request.kind == Kind::Tunnel {
                args.extend(["-N", "-o", "ExitOnForwardFailure=yes"].map(String::from));
            }
            if let Some(socket) = &socket {
                args.extend(["-o".into(), "ControlMaster=auto".into(), "-o".into(), format!("ControlPath={socket}"), "-o".into(), format!("ControlPersist={CONTROL_PERSIST_SECONDS}")]);
            }
            let mut common = Vec::new();
            if let Some(port) = profile.port {
                common.extend([if request.kind == Kind::Sftp { "-P" } else { "-p" }.to_string(), port.to_string()]);
            }
            if let Some(key) = &profile.key_path {
                common.extend(["-i".to_string(), key.clone()]);
            }
            // `-o` rather than `-J`: older sftp lacks -J, and the file panel reuses these.
            if let Some(jump) = &profile.proxy_jump {
                common.extend(["-o".to_string(), format!("ProxyJump={jump}")]);
            }
            args.extend(common.iter().cloned());
            if request.kind == Kind::Tunnel {
                let forward = profile.forward_type.unwrap_or(Forward::L);
                let mut spec = format!("127.0.0.1:{}", profile.bind_port.unwrap_or_default());
                if forward != Forward::D {
                    let target = profile.target_host.clone().unwrap_or_default();
                    let target = if target.contains(':') && !target.starts_with('[') { format!("[{target}]") } else { target };
                    spec = format!("{spec}:{target}:{}", profile.target_port.unwrap_or_default());
                }
                args.extend([format!("-{forward:?}"), spec]);
            }
            args.push(destination(&profile, request.kind));
            let program = require(if request.kind == Kind::Sftp { "sftp" } else { "ssh" })?;
            let full = profile.host.as_deref().unwrap_or_default();
            let (user, host) = match full.rsplit_once('@') {
                Some((user, host)) => (Some(user), host),
                None => (None, full),
            };
            connection = json!({ "host": host, "user": profile.username.as_deref().or(user), "port": profile.port });
            if shares_login {
                let ssh_common: Vec<String> = common.iter().map(|arg| if arg == "-P" { "-p".to_string() } else { arg.clone() }).collect();
                companion = Some(Companion { executable: require("ssh")?, destination: destination(&profile, Kind::Ssh), control_path: socket, args: ssh_common });
            }
            (program, args)
        }
    };
    let title = profile.name.clone().unwrap_or_else(|| match request.kind {
        Kind::Shell => "本地终端".into(),
        Kind::Agent => "Agent".into(),
        Kind::Ssh => format!("SSH · {}", profile.host.as_deref().unwrap_or_default()),
        Kind::Sftp => format!("SFTP · {}", profile.host.as_deref().unwrap_or_default()),
        Kind::Serial => format!("串口 · {}", profile.device.as_deref().unwrap_or_default()),
        Kind::Tunnel => format!("隧道 · {}", profile.host.as_deref().unwrap_or_default()),
    });
    let kind = serde_json::to_value(request.kind).map_err(|error| error.to_string())?;
    let host = json!({ "executable": executable, "args": args, "cwd": cwd, "env": session_env(), "kind": kind, "title": title, "connection": connection });
    Ok(Plan { host, title, powershell, companion })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_destinations_keep_users_and_bracket_ipv6_for_sftp() {
        let profile = Profile { kind: Some(Kind::Sftp), host: Some("me@::1".into()), ..Profile::default() };
        assert_eq!(destination(&profile, Kind::Sftp), "me@[::1]");
        assert_eq!(destination(&profile, Kind::Ssh), "me@::1");
        let named = Profile { username: Some("ops".into()), host: Some("box".into()), ..Profile::default() };
        assert_eq!(destination(&named, Kind::Ssh), "ops@box");
    }

    #[test]
    fn a_missing_directory_is_refused_before_anything_starts() {
        let request = Request { kind: Kind::Shell, cwd: "/definitely/not/here", profile: None, companion: false, local_shell: &LocalShell::default() };
        assert!(build(&request, || None).unwrap_err().contains("工作目录"));
    }

    #[test]
    fn custom_shell_arguments_are_passed_verbatim() {
        let (program, _, _) = local_shell().unwrap();
        let args = vec!["argument with spaces".into(), "$(literal); & | >".into(), String::new()];
        let shell = LocalShell { program: program.clone(), args: args.clone() };
        let cwd = std::env::current_dir().unwrap();
        let request = Request { kind: Kind::Shell, cwd: cwd.to_str().unwrap(), profile: None, companion: false, local_shell: &shell };
        let plan = build(&request, || None).unwrap();
        assert_eq!(plan.host["executable"], program);
        assert_eq!(plan.host["args"], json!(args));
    }

    #[test]
    fn missing_custom_shell_is_reported_instead_of_falling_back() {
        let shell = LocalShell { program: "/omniterm-missing-shell/program".into(), args: vec![] };
        assert!(shell.resolve().unwrap_err().contains("自定义 Shell"));
        assert!(LocalShell { args: vec!["bad\0argument".into()], ..LocalShell::default() }.resolve().is_err());
    }

    #[test]
    fn empty_shell_configuration_preserves_automatic_discovery() {
        assert_eq!(LocalShell::default().resolve().unwrap(), local_shell().unwrap());
        let shell = LocalShell { program: String::new(), args: vec!["--custom-argument".into()] };
        assert_eq!(shell.resolve().unwrap().1, shell.args);
    }
}

// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! How each kind of session starts, as the desktop app's `command-plan.mjs` decides it: the
//! program, its arguments and environment, and for SSH the shared login the file panel uses.
//! Arguments are always a list, never a shell string, so a profile value cannot become syntax.

use super::paths::find_executable;
use super::profile::{Forward, Kind, Profile, X11};
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
    /// Something the user should know although the session starts, such as X11 forwarding
    /// left off for want of an X server.
    pub warning: Option<String>,
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
        // sftp also parses the destination as a path. Pass routing usernames via
        // the shared -o User option so '/' can never change destination parsing.
        Some(user) if !user.contains('/') => format!("{user}@{host}"),
        _ => host,
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
    let mut warning = None;
    let mut display = None;
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
            if let Some(user) = profile.username.as_deref().filter(|user| user.contains('/')) {
                common.extend(["-o".to_string(), format!("User={user}")]);
            }
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
            // Only the interactive session forwards X11; the shared login's other channels never do.
            if request.kind == Kind::Ssh && profile.x11 != X11::Off {
                match super::x11::resolve() {
                    Ok(found) => {
                        // Windows has no xauth to make an untrusted cookie, so ssh needs -Y there.
                        let trusted = profile.x11 == X11::Trusted || cfg!(windows);
                        args.extend(["-o".to_string(), "ForwardX11=yes".to_string(), "-o".to_string(), format!("ForwardX11Trusted={}", if trusted { "yes" } else { "no" })]);
                        if let Some(xauth) = &found.xauth {
                            args.extend(["-o".to_string(), format!("XAuthLocation={xauth}")]);
                        }
                        display = Some(found.display);
                    }
                    Err(error) => warning = Some(format!("X11 转发未启用：{error}")),
                }
            }
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
    let mut env = session_env();
    if request.kind == Kind::Ssh && profile.remember_password {
        env.extend(super::ssh_auth::prepare(&profile)?.into_iter().map(|(key, value)| (key, Value::String(value))));
    }
    if let Some(display) = display {
        env.insert("DISPLAY".into(), Value::String(display));
    }
    let host = json!({ "executable": executable, "args": args, "cwd": cwd, "env": env, "kind": kind, "title": title, "connection": connection });
    Ok(Plan { host, title, powershell, warning, companion })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembering_passwords_uses_askpass_only_for_opted_in_ssh_connections() {
        if find_executable("ssh").is_none() { return; }
        let cwd = std::env::current_dir().unwrap();
        let shell = LocalShell::default();
        let profile = Profile { kind: Some(Kind::Ssh), host: Some("gateway".into()), username: Some("fixture".into()), port: Some(60022), remember_password: true, ..Profile::default() };
        let request = Request { kind: Kind::Ssh, cwd: cwd.to_str().unwrap(), profile: Some(&profile), companion: false, local_shell: &shell };
        let plan = build(&request, || None).unwrap();
        let env = &plan.host["env"];
        assert_eq!(env["SSH_ASKPASS_REQUIRE"], "force");
        assert_eq!(env["OMNITERM_SSH_ASKPASS"], "1");
        let state = Path::new(env["OMNITERM_SSH_AUTH_STATE"].as_str().unwrap());
        assert!(!state.exists(), "planning stores no password or first-answer data");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(state.parent().unwrap().metadata().unwrap().permissions().mode() & 0o777, 0o700);
        }
        let ordinary = Profile { remember_password: false, ..profile.clone() };
        let ordinary_plan = build(&Request { profile: Some(&ordinary), ..request }, || None).unwrap();
        assert!(ordinary_plan.host["env"].get("OMNITERM_SSH_AUTH_STATE").is_none());
        std::fs::remove_dir(state.parent().unwrap()).unwrap();
    }

    #[test]
    fn ssh_destinations_keep_users_and_bracket_ipv6_for_sftp() {
        let profile = Profile { kind: Some(Kind::Sftp), host: Some("me@::1".into()), ..Profile::default() };
        assert_eq!(destination(&profile, Kind::Sftp), "me@[::1]");
        assert_eq!(destination(&profile, Kind::Ssh), "me@::1");
        let named = Profile { username: Some("ops".into()), host: Some("box".into()), ..Profile::default() };
        assert_eq!(destination(&named, Kind::Ssh), "ops@box");
    }

    #[test]
    fn bastion_routing_user_is_preserved_for_terminal_sftp_and_companions() {
        if find_executable("ssh").is_none() || find_executable("sftp").is_none() { return; }
        let cwd = std::env::current_dir().unwrap();
        let shell = LocalShell::default();
        let user = "alice/192.0.2.10/root";
        for kind in [Kind::Ssh, Kind::Sftp, Kind::Tunnel] {
            let profile = Profile { kind: Some(kind), host: Some("gateway".into()), username: Some(user.into()), port: Some(60022), forward_type: Some(Forward::D), bind_port: Some(8080), ..Profile::default() };
            let request = Request { kind, cwd: cwd.to_str().unwrap(), profile: Some(&profile), companion: true, local_shell: &shell };
            let plan = build(&request, || None).unwrap();
            let args: Vec<&str> = plan.host["args"].as_array().unwrap().iter().map(|value| value.as_str().unwrap()).collect();
            assert!(args.windows(2).any(|pair| pair == ["-o", "User=alice/192.0.2.10/root"]));
            assert_eq!(args.last(), Some(&"gateway"));
            if let Some(companion) = plan.companion {
                assert_eq!(companion.destination, "gateway");
                assert!(companion.args.windows(2).any(|pair| pair == ["-o", "User=alice/192.0.2.10/root"]));
                // OpenSSH's actual parser checks the generated options without
                // connecting, resolving DNS, or reading personal configuration.
                #[cfg(unix)]
                {
                    let output = Command::new(&companion.executable).args(["-F", "/dev/null", "-G"]).args(&companion.args).arg(&companion.destination).output().unwrap();
                    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
                    assert!(String::from_utf8_lossy(&output.stdout).lines().any(|line| line == format!("user {user}")));
                }
            }
            assert_eq!(plan.host["connection"]["user"], user);
        }
    }

    #[cfg(unix)]
    #[test]
    fn x11_forwarding_reaches_only_the_interactive_ssh_session() {
        if find_executable("ssh").is_none() {
            return;
        }
        let cwd = std::env::current_dir().unwrap();
        let shell = LocalShell::default();
        let profile = Profile { kind: Some(Kind::Ssh), host: Some("box".into()), x11: X11::Restricted, ..Profile::default() };
        let request = Request { kind: Kind::Ssh, cwd: cwd.to_str().unwrap(), profile: Some(&profile), companion: true, local_shell: &shell };
        let plan = build(&request, || Some("/tmp/omniterm-test.sock".into())).unwrap();
        let args: Vec<&str> = plan.host["args"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        match super::super::x11::resolve() {
            Ok(display) => {
                assert!(args.windows(2).any(|pair| pair == ["-o", "ForwardX11=yes"]), "{args:?}");
                assert!(args.contains(&"ForwardX11Trusted=no"));
                assert_eq!(plan.host["env"]["DISPLAY"], display.display);
                assert!(plan.warning.is_none());
            }
            Err(_) => {
                assert!(!args.iter().any(|arg| arg.starts_with("ForwardX11")));
                assert!(plan.warning.unwrap().contains("X11"));
            }
        }
        assert!(plan.companion.unwrap().args.iter().all(|arg| !arg.contains("X11")), "the file panel's channels never forward X11");
        let off = Profile { x11: X11::Off, ..profile.clone() };
        let request = Request { profile: Some(&off), ..request };
        let plan = build(&request, || None).unwrap();
        assert!(!plan.host["args"].as_array().unwrap().iter().any(|arg| arg.as_str().is_some_and(|arg| arg.contains("X11"))));
    }

    /// Runs in CI against a loopback sshd and Xvfb (`OMNITERM_E2E_SSH=user@127.0.0.1`): the
    /// exact command a session would run, with a remote X client, in both modes.
    #[cfg(unix)]
    #[test]
    #[ignore = "needs a loopback sshd and an X server"]
    fn x11_forwarding_end_to_end() {
        let target = std::env::var("OMNITERM_E2E_SSH").expect("OMNITERM_E2E_SSH names the loopback SSH target");
        let cwd = std::env::current_dir().unwrap();
        let shell = LocalShell::default();
        for mode in [X11::Restricted, X11::Trusted] {
            let profile = Profile { kind: Some(Kind::Ssh), host: Some(target.clone()), x11: mode, ..Profile::default() };
            let request = Request { kind: Kind::Ssh, cwd: cwd.to_str().unwrap(), profile: Some(&profile), companion: false, local_shell: &shell };
            let plan = build(&request, || None).unwrap();
            assert!(plan.warning.is_none(), "{:?}", plan.warning);
            let mut args: Vec<String> = plan.host["args"].as_array().unwrap().iter().map(|arg| arg.as_str().unwrap().to_string()).collect();
            let destination = args.pop().unwrap();
            args.extend(["-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=accept-new"].map(String::from));
            args.push(destination);
            args.push("echo \"remote-display=$DISPLAY\"; xdpyinfo | grep -m1 'name of display'".into());
            let env = plan.host["env"].as_object().unwrap();
            let output = Command::new(plan.host["executable"].as_str().unwrap())
                .args(&args)
                .env_clear()
                .envs(env.iter().map(|(key, value)| (key, value.as_str().unwrap_or_default())))
                .stdin(Stdio::null())
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success() && stdout.contains("remote-display=localhost:") && stdout.contains("name of display"),
                "{mode:?}: {stdout}\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            // Evidence in the CI log that each mode really ran end to end.
            println!("x11-e2e {mode:?} ok: {}", stdout.trim().replace('\n', " | "));
        }
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

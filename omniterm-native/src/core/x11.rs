// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! X11 forwarding for SSH sessions: remote graphical programs open as windows on this
//! desktop, as in MobaXterm. This module finds the local X display ssh forwards to.
//!
//! - Linux: the session's `DISPLAY` (an X11 session, or XWayland under Wayland).
//! - macOS: XQuartz's `DISPLAY`, with XQuartz's own `xauth`, which ssh does not look for.
//! - Windows: an X server already listening on 127.0.0.1:6000+N, or else an installed
//!   VcXsrv or Xming that omniTerm starts with a fresh MIT-MAGIC-COOKIE-1. Windows has no
//!   `xauth`, so ssh is pointed at omniTerm itself, which answers `xauth list` with that
//!   cookie (see `xauth_shim`); the server never runs without access control.

use std::path::PathBuf;

/// Where ssh forwards X11 connections, and the `xauth` it should use, if not its default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Display {
    pub display: String,
    pub xauth: Option<String>,
}

#[cfg(unix)]
pub fn resolve() -> Result<Display, String> {
    let display = std::env::var("DISPLAY").ok().filter(|display| !display.trim().is_empty());
    if cfg!(target_os = "macos") {
        let xauth = PathBuf::from("/opt/X11/bin/xauth");
        let xauth = xauth.is_file().then(|| xauth.to_string_lossy().into_owned());
        return match display {
            Some(display) => Ok(Display { display, xauth }),
            None if PathBuf::from("/opt/X11/bin/Xquartz").exists() => Err("XQuartz 已安装但还没有提供显示。请先打开 XQuartz；刚安装的需要注销并重新登录一次。".into()),
            None => Err("X11 转发需要 X 服务器：请安装 XQuartz（xquartz.org）后注销并重新登录。".into()),
        };
    }
    display.map(|display| Display { display, xauth: None }).ok_or_else(|| "没有可用的 X 显示（DISPLAY 未设置）。Wayland 会话请确认已启用 XWayland。".into())
}

#[cfg(windows)]
pub fn resolve() -> Result<Display, String> {
    windows::resolve()
}

#[cfg(not(any(unix, windows)))]
pub fn resolve() -> Result<Display, String> {
    Err("这个平台不支持 X11 转发。".into())
}

/// Stops the X server omniTerm started, when the app exits.
pub fn shutdown() {
    #[cfg(windows)]
    windows::shutdown();
}

/// When ssh runs omniTerm as its `xauth` (`omniTerm.exe list 127.0.0.1:0.0`), prints the
/// cookie of the server omniTerm started and returns the exit code; `None` for a normal start.
pub fn xauth_shim(args: &[String]) -> Option<i32> {
    // ssh runs `xauth [-f file] list DISPLAY` for trusted forwarding and
    // `xauth -f file generate DISPLAY ...` for untrusted, which needs a real xauth.
    let rest = match args {
        [first, file, rest @ ..] if first == "-f" && !file.is_empty() => rest,
        rest => rest,
    };
    match rest {
        [command, display] if command == "list" => {
            #[cfg(windows)]
            if let Some(line) = windows::cookie_line(display) {
                println!("{line}");
            }
            #[cfg(not(windows))]
            let _ = display;
            Some(0)
        }
        [command, ..] if command == "generate" && args.first().is_some_and(|first| first == "-f") => Some(1),
        _ => None,
    }
}

#[cfg(any(windows, test))]
/// The display number in `host:N.S`, `:N` or `unix:N`.
fn display_number(display: &str) -> Option<u16> {
    let (_, rest) = display.rsplit_once(':')?;
    rest.split('.').next()?.parse().ok()
}

#[cfg(any(windows, test))]
/// One Xauthority entry that any address accepts: family, address, display number, name,
/// data, each length-prefixed, big-endian, as Xau writes them.
fn xauthority(number: u16, cookie: &[u8; 16]) -> Vec<u8> {
    const FAMILY_WILD: u16 = 0xffff;
    let mut out = FAMILY_WILD.to_be_bytes().to_vec();
    for field in [&b""[..], number.to_string().as_bytes(), b"MIT-MAGIC-COOKIE-1", cookie] {
        out.extend((field.len() as u16).to_be_bytes());
        out.extend(field);
    }
    out
}

#[cfg(any(windows, test))]
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(windows)]
mod windows {
    use super::{Display, display_number, hex, xauthority};
    use crate::core::paths;
    use std::net::{SocketAddr, TcpStream};
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DISPLAYS: std::ops::Range<u16> = 0..10;
    const COOKIE_FILE: &str = "native-x11-cookie";
    const AUTH_FILE: &str = "native-x11.auth";

    /// The X server omniTerm started, kept so it can be stopped on exit.
    static SERVER: Mutex<Option<Child>> = Mutex::new(None);

    fn listening(number: u16) -> bool {
        TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], 6000 + number)), Duration::from_millis(150)).is_ok()
    }

    pub fn resolve() -> Result<Display, String> {
        // One at a time, so sessions opening together start one server between them.
        let mut server = SERVER.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let xauth = shim_path();
        if let Some(display) = std::env::var("DISPLAY").ok().filter(|display| !display.trim().is_empty()) {
            return Ok(Display { display, xauth });
        }
        if let Some(number) = DISPLAYS.clone().find(|number| listening(*number)) {
            return Ok(Display { display: format!("127.0.0.1:{number}.0"), xauth });
        }
        let program = installed_server().ok_or("X11 转发需要本机 X 服务器：请安装 VcXsrv（或 Xming），omniTerm 会自动启动它；也可以先运行任意 X 服务器。")?;
        if xauth.is_none() {
            return Err("无法为 ssh 提供 X11 授权（omniTerm 的安装路径无法转成短路径）。请把 omniTerm 安装到不含空格的目录，或自行运行 X 服务器。".into());
        }
        let number = 0;
        let mut cookie = [0u8; 16];
        cookie.copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        let auth = paths::file(AUTH_FILE);
        paths::write_atomically(&auth, &xauthority(number, &cookie)).map_err(|error| format!("无法写入 X11 授权文件：{error}"))?;
        paths::write_atomically(&paths::file(COOKIE_FILE), format!("{number} {}\n", hex(&cookie)).as_bytes()).map_err(|error| format!("无法写入 X11 授权文件：{error}"))?;
        let child = Command::new(&program)
            .args([format!(":{number}"), "-multiwindow".into(), "-clipboard".into(), "-wgl".into(), "-auth".into(), auth.to_string_lossy().into_owned()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|error| format!("无法启动 X 服务器 {}：{error}", program.display()))?;
        if let Some(mut old) = server.replace(child) {
            let _ = old.kill();
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        while !listening(number) {
            if Instant::now() >= deadline || server.as_mut().is_some_and(|child| child.try_wait().ok().flatten().is_some()) {
                return Err(format!("X 服务器 {} 没有在 127.0.0.1:{} 上启动。", program.display(), 6000 + number));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(Display { display: format!("127.0.0.1:{number}.0"), xauth })
    }

    pub fn shutdown() {
        if let Some(mut child) = SERVER.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_file(paths::file(COOKIE_FILE));
        let _ = std::fs::remove_file(paths::file(AUTH_FILE));
    }

    /// `xauth list` output for the server omniTerm started, if `display` is that server.
    pub fn cookie_line(display: &str) -> Option<String> {
        let contents = std::fs::read_to_string(paths::file(COOKIE_FILE)).ok()?;
        let (number, cookie) = contents.trim().split_once(' ')?;
        let number: u16 = number.parse().ok()?;
        (display_number(display)? == number && cookie.len() == 32 && cookie.bytes().all(|byte| byte.is_ascii_hexdigit())).then(|| format!("127.0.0.1:{number}  MIT-MAGIC-COOKIE-1  {cookie}"))
    }

    fn installed_server() -> Option<PathBuf> {
        let roots = ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"].into_iter().filter_map(|name| std::env::var_os(name)).map(PathBuf::from);
        let candidates = roots.flat_map(|root| [root.join("VcXsrv").join("vcxsrv.exe"), root.join("Xming").join("Xming.exe")]);
        candidates.into_iter().find(|path| path.is_file()).or_else(|| paths::find_executable("vcxsrv.exe")).or_else(|| paths::find_executable("Xming.exe"))
    }

    /// omniTerm's own path for ssh's `XAuthLocation`. ssh runs it through a command line
    /// without quotes, so a path with spaces becomes its 8.3 short form.
    fn shim_path() -> Option<String> {
        let path = std::env::current_exe().ok()?;
        let text = path.to_string_lossy().into_owned();
        if !text.contains(' ') {
            return Some(text);
        }
        short_path(&path).filter(|short| !short.contains(' '))
    }

    fn short_path(path: &Path) -> Option<String> {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut buffer = vec![0u16; 1024];
        // SAFETY: `wide` is NUL-terminated and `buffer` holds the length passed.
        let length = unsafe { GetShortPathNameW(wide.as_ptr(), buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
        (length > 0 && length < buffer.len()).then(|| std::ffi::OsString::from_wide(&buffer[..length]).to_string_lossy().into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_numbers_come_from_every_display_form() {
        assert_eq!(display_number("127.0.0.1:0.0"), Some(0));
        assert_eq!(display_number("localhost:12"), Some(12));
        assert_eq!(display_number(":3"), Some(3));
        assert_eq!(display_number("unix:1.0"), Some(1));
        assert_eq!(display_number("no-display"), None);
    }

    #[test]
    fn the_authority_entry_is_what_xau_reads() {
        let entry = xauthority(7, &[0xab; 16]);
        let mut expected = vec![0xff, 0xff, 0, 0, 0, 1, b'7', 0, 18];
        expected.extend(b"MIT-MAGIC-COOKIE-1");
        expected.extend([0, 16]);
        expected.extend([0xab; 16]);
        assert_eq!(entry, expected);
        assert_eq!(hex(&[0x00, 0x0f, 0xff]), "000fff");
    }

    #[test]
    fn only_xauth_invocations_are_taken_over() {
        let args = |list: &[&str]| list.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
        assert_eq!(xauth_shim(&args(&["list", "127.0.0.1:0.0"])), Some(0));
        assert_eq!(xauth_shim(&args(&["-f", "C:\\t\\xauthfile", "list", ":0"])), Some(0));
        assert_eq!(xauth_shim(&args(&["-f", "C:\\t\\xauthfile", "generate", ":0", "MIT-MAGIC-COOKIE-1", "untrusted"])), Some(1));
        assert_eq!(xauth_shim(&args(&[])), None);
        assert_eq!(xauth_shim(&args(&["--bench", "32"])), None);
        assert_eq!(xauth_shim(&args(&["--smoke", "out.png"])), None);
        assert_eq!(xauth_shim(&args(&["list"])), None);
    }
}

// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! AccessClient's WinSCP export describes a separate SFTP endpoint. Its `pwd` is
//! passed unchanged, once, to that endpoint's first account-password prompt.
//! Neither the export nor its credential belongs in a saved terminal profile.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const MAX_IMPORT: u64 = 1024 * 1024;
const MODE: &str = "OMNITERM_QIZHI_ASKPASS";
const STATE: &str = "OMNITERM_QIZHI_AUTH_STATE";
const CREDENTIAL: &str = "credential";
const METADATA: &str = "scope.json";

/// Deliberately not Debug, Serialize or Clone: only the public endpoint fields
/// are suitable for display. A connection owns one imported credential attempt.
pub struct Connection {
    pub host: String,
    pub user: String,
    pub port: u16,
    pub initial_directory: String,
    pub title: String,
    secret: Option<String>,
}

fn paired<'a>(data: &'a Value, upper: &str, lower: &str) -> Result<Option<&'a Value>, String> {
    match (data.get(upper), data.get(lower)) {
        (Some(a), Some(b)) if a != b => {
            Err(format!("导出文件中的 {upper}/{lower} 不一致，请重新导出。"))
        }
        (Some(value), _) | (_, Some(value)) => Ok(Some(value)),
        _ => Ok(None),
    }
}

fn checked_text(value: Option<&Value>, label: &str, maximum: usize) -> Result<String, String> {
    let value = value
        .and_then(Value::as_str)
        .ok_or_else(|| format!("导出文件缺少有效的{label}。"))?;
    if value.len() > maximum || value.chars().any(char::is_control) {
        return Err(format!("导出文件中的{label}过长或含有控制字符。"));
    }
    Ok(value.to_string())
}

fn parse(bytes: &[u8]) -> Result<Connection, String> {
    if bytes.len() as u64 > MAX_IMPORT {
        return Err("连接导出文件过大。".into());
    }
    let document: Value =
        serde_json::from_slice(bytes).map_err(|_| "连接导出文件不是有效的 JSON。".to_string())?;
    let data = match document.get("parameters") {
        Some(value) if value.is_object() => value,
        Some(_) => return Err("连接导出文件中的 parameters 不是对象。".into()),
        None if document.is_object() => &document,
        None => return Err("连接导出文件缺少有效的参数对象。".into()),
    };
    if data.get("app").and_then(Value::as_str) != Some("winscp") {
        return Err("请选择 AccessClient 导出的 WinSCP 连接参数。".into());
    }
    if let Some(protocol) = data.get("proto").filter(|value| !value.is_null())
        && !protocol.as_str().is_some_and(|value| {
            value.trim().is_empty() || value.trim().eq_ignore_ascii_case("sftp")
        })
    {
        return Err("这个导出文件不是 SFTP 连接。".into());
    }
    let nested_proxy = data
        .get("proxy")
        .filter(|value| !value.is_null())
        .map(|proxy| {
            proxy
                .as_object()
                .map(|proxy| proxy.get("addr"))
                .ok_or("无法识别导出文件中的代理设置。")
        })
        .transpose()?
        .flatten();
    let flat_proxy = data.get("proxy.addr");
    if let (Some(flat), Some(nested)) = (flat_proxy, nested_proxy)
        && flat != nested
    {
        return Err("导出文件中的代理地址不一致，请重新导出。".into());
    }
    if let Some(address) = flat_proxy.or(nested_proxy).filter(|value| !value.is_null())
        && !address
            .as_str()
            .is_some_and(|value| value.trim().is_empty())
    {
        return Err(
            "这个导出文件需要代理入口，当前不能直接连接；请提供无需代理的 SFTP 参数。".into(),
        );
    }
    let host = checked_text(paired(data, "Host", "host")?, "SFTP 主机", 255)?;
    if host.is_empty()
        || host.starts_with('-')
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.:%[]-".contains(c))
    {
        return Err("导出文件中的 SFTP 主机格式无效。".into());
    }
    let user = checked_text(paired(data, "User", "user")?, "SFTP 用户名", 1024)?;
    if user.is_empty()
        || user.starts_with('-')
        || !user
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.-@/\\:#%+".contains(c))
    {
        return Err("导出文件中的 SFTP 用户名格式无效。".into());
    }
    let port = match paired(data, "Port", "port")? {
        Some(Value::String(value)) if value.bytes().all(|c| c.is_ascii_digit()) => {
            value.parse::<u16>().ok()
        }
        Some(Value::Number(value)) => value.as_u64().and_then(|value| u16::try_from(value).ok()),
        _ => None,
    }
    .filter(|value| *value > 0)
    .ok_or("导出文件中的 SFTP 端口必须是 1–65535。")?;
    let initial_directory = match paired(data, "Path", "path")? {
        None | Some(Value::Null) => ".".into(),
        value => {
            let path = checked_text(value, "初始目录", 4096)?;
            if path.is_empty() { ".".into() } else { path }
        }
    };
    // st is a display title, not a session token or an authentication credential.
    let title = match data.get("st").filter(|value| !value.is_null()) {
        Some(value) => checked_text(Some(value), "连接标题", 512)?,
        None => String::new(),
    };
    let title = if title.is_empty() {
        format!("SFTP · {host}:{port}")
    } else {
        title
    };
    // AccessClient's URL builder reads lowercase pwd. PWD and url are not used,
    // and this value must not be trimmed, decoded, or replaced by another field.
    let secret = data
        .get("pwd")
        .and_then(Value::as_str)
        .filter(|value| valid_answer(value))
        .ok_or("导出文件缺少有效的小写 pwd 凭据，请重新导出。")?
        .to_string();
    Ok(Connection {
        host,
        user,
        port,
        initial_directory,
        title,
        secret: Some(secret),
    })
}

pub fn load(path: &Path) -> Result<Connection, String> {
    let file = std::fs::File::open(path).map_err(|_| "无法读取连接导出文件。".to_string())?;
    if !file
        .metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() <= MAX_IMPORT)
    {
        return Err("连接导出文件不是普通文件，或文件过大。".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_IMPORT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取连接导出文件。".to_string())?;
    parse(&bytes)
}

impl Connection {
    /// Complete arguments for one SFTP subsystem, with no shared terminal or
    /// inherited SSH configuration, forwarding, agent, key, or command channel.
    pub fn ssh_args(&self) -> Vec<String> {
        let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
        let mut args = vec![
            "-F".into(),
            null.into(),
            "-T".into(),
            "-p".into(),
            self.port.to_string(),
        ];
        for option in [
            format!("User={}", self.user),
            "ControlMaster=no".into(),
            "ControlPath=none".into(),
            "ControlPersist=no".into(),
            "StrictHostKeyChecking=yes".into(),
            "NumberOfPasswordPrompts=1".into(),
            "PreferredAuthentications=password,keyboard-interactive".into(),
            "BatchMode=no".into(),
            "PubkeyAuthentication=no".into(),
            "IdentityAgent=none".into(),
            "IdentitiesOnly=yes".into(),
            "PasswordAuthentication=yes".into(),
            "KbdInteractiveAuthentication=yes".into(),
            "ClearAllForwardings=yes".into(),
            "ForwardAgent=no".into(),
            "ForwardX11=no".into(),
            "RemoteCommand=none".into(),
            "RequestTTY=no".into(),
            "ConnectTimeout=15".into(),
            "ConnectionAttempts=1".into(),
        ] {
            args.extend(["-o".into(), option]);
        }
        args.extend(["-s".into(), self.host.clone(), "sftp".into()]);
        args
    }

    /// Preparing another explicitly requested attempt does not reuse the export
    /// password. Its helper will ask interactively instead.
    pub fn prepare_auth(&mut self) -> Result<Auth, String> {
        let executable =
            std::env::current_exe().map_err(|_| "找不到 SFTP 认证助手。".to_string())?;
        let root =
            std::env::temp_dir().join(format!("omniterm-qizhi-{}", uuid::Uuid::new_v4().simple()));
        #[cfg(unix)]
        let builder = {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = std::fs::DirBuilder::new();
            builder.mode(0o700);
            builder
        };
        #[cfg(not(unix))]
        let builder = std::fs::DirBuilder::new();
        builder
            .create(&root)
            .map_err(|_| "无法创建私有的 SFTP 认证状态目录。".to_string())?;
        let auth = Auth {
            env: BTreeMap::from([
                (MODE.into(), "1".into()),
                (STATE.into(), root.to_string_lossy().into_owned()),
                (
                    "SSH_ASKPASS".into(),
                    executable.to_string_lossy().into_owned(),
                ),
                ("SSH_ASKPASS_REQUIRE".into(), "force".into()),
            ]),
            root,
        };
        let metadata = Metadata {
            version: 1,
            scope: uuid::Uuid::new_v4().to_string(),
            host: self.host.clone(),
            user: self.user.clone(),
            port: self.port,
        };
        let encoded =
            serde_json::to_vec(&metadata).map_err(|_| "无法准备 SFTP 认证。".to_string())?;
        write_private(&auth.root.join(METADATA), &encoded)
            .map_err(|_| "无法写入私有的 SFTP 认证状态。".to_string())?;
        if let Some(secret) = self.secret.take() {
            write_private(&auth.root.join(CREDENTIAL), secret.as_bytes())
                .map_err(|_| "无法准备一次性的 SFTP 凭据。".to_string())?;
        }
        Ok(auth)
    }
}

/// Contains paths and public helper settings only; no credential in the environment.
pub struct Auth {
    root: PathBuf,
    env: BTreeMap<String, String>,
}

impl Auth {
    pub fn env(&self) -> &BTreeMap<String, String> {
        &self.env
    }

    /// Active helpers observe removal and exit their own process, including its
    /// window. No process IDs from unrelated SSH sessions are used for cleanup.
    pub fn cancel(&self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Drop for Auth {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    version: u8,
    scope: String,
    host: String,
    user: String,
    port: u16,
}

fn write_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(contents)?;
    file.flush()
}

fn valid_answer(value: &str) -> bool {
    !value.is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control)
}

fn account_password(prompt: &str, metadata: &Metadata) -> bool {
    let prompt = prompt.trim();
    if prompt.eq_ignore_ascii_case("password:") {
        return true;
    }
    let Some(split) = prompt.len().checked_sub("password:".len()) else {
        return false;
    };
    if !prompt
        .get(split..)
        .is_some_and(|suffix| suffix.eq_ignore_ascii_case("password:"))
    {
        return false;
    }
    let Some(prefix) = prompt.get(..split) else {
        return false;
    };
    // Accounts can be case-sensitive; only the word "password" ignores case.
    let account = format!("{}@{}", metadata.user, metadata.host);
    prefix == format!("{account}'s ")
        || prefix == format!("({account}) ")
        || prefix == format!("{account} ")
}

/// The credential is consumed only for this account's first password prompt.
/// OTPs, host confirmation and different accounts cannot read that file.
fn automatic_answer(root: &Path, metadata: &Metadata, prompt: &str) -> Option<String> {
    if !account_password(prompt, metadata)
        || write_private(&root.join("password-attempted"), b"").is_err()
    {
        return None;
    }
    let path = root.join(CREDENTIAL);
    let result = (|| {
        let file = std::fs::File::open(&path).ok()?;
        if !file.metadata().is_ok_and(|metadata| metadata.len() <= 4096) {
            return None;
        }
        let mut value = String::new();
        file.take(4097).read_to_string(&mut value).ok()?;
        valid_answer(&value).then_some(value)
    })();
    // Do not submit an imported credential if its one-time file could not be
    // removed. The caller can still ask manually, and Auth will clean up on close.
    std::fs::remove_file(path).ok().and(result)
}

struct Prompt {
    marker: PathBuf,
    done: Arc<AtomicBool>,
    watcher: Option<std::thread::JoinHandle<()>>,
}

impl Prompt {
    fn start(root: &Path) -> Option<Self> {
        let marker = root.join("prompt-active");
        write_private(&marker, b"").ok()?;
        let done = Arc::new(AtomicBool::new(false));
        let stop = done.clone();
        let directory = root.to_path_buf();
        let watcher = std::thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                if !directory.is_dir() {
                    // This thread exists only in the dedicated askpass child.
                    std::process::exit(1);
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        Some(Self {
            marker,
            done,
            watcher: Some(watcher),
        })
    }
}

impl Drop for Prompt {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Release);
        if let Some(watcher) = self.watcher.take() {
            let _ = watcher.join();
        }
        let _ = std::fs::remove_file(&self.marker);
    }
}

fn answer(
    root: &Path,
    prompt: &str,
    confirm: bool,
    ask: impl FnOnce(&str, &str, bool, bool) -> Option<String>,
) -> Option<String> {
    let bytes = std::fs::read(root.join(METADATA))
        .ok()
        .filter(|bytes| bytes.len() <= 8192)?;
    let metadata: Metadata = serde_json::from_slice(&bytes).ok()?;
    if metadata.version != 1 || metadata.scope.is_empty() || prompt.len() > 8192 {
        return None;
    }
    if !confirm && let Some(value) = automatic_answer(root, &metadata, prompt) {
        return Some(value);
    }
    let _waiting = Prompt::start(root)?;
    // first=false is deliberate: even an account-password retry must never save
    // this one-time import or a subsequent OTP in the ordinary SSH keychain.
    // A repeated account prompt does not prove expiry: a gateway may require
    // another factor. Explain the export lifecycle without disabling that flow.
    let guidance = if !confirm && account_password(prompt, &metadata)
        && root.join("password-attempted").exists()
    {
        "已尝试导出文件中的凭据，但文件服务仍要求认证。凭据可能已失效；若不确定此处需要哪项认证，请取消，在堡垒机重新打开该资产的文件连接，更新导出的参数后重连。\n\n"
    } else {
        ""
    };
    let display = format!(
        "SFTP · {}@{}:{}\n\n{guidance}{prompt}",
        metadata.user, metadata.host, metadata.port
    );
    let value = ask(&metadata.scope, &display, false, confirm)?;
    (root.is_dir() && valid_answer(&value)).then_some(value)
}

pub fn helper(
    args: &[String],
    ask: impl FnOnce(&str, &str, bool, bool) -> Option<String>,
) -> Option<i32> {
    if std::env::var(MODE).as_deref() != Ok("1") {
        return None;
    }
    if args.len() != 1 {
        return Some(1);
    }
    let Some(root) = std::env::var_os(STATE).map(PathBuf::from) else {
        return Some(1);
    };
    let confirm = std::env::var("SSH_ASKPASS_PROMPT").as_deref() == Ok("confirm")
        || args[0].contains("yes/no");
    let result = answer(&root, &args[0], confirm, ask);
    Some(match result {
        Some(value)
            if std::io::stdout()
                .write_all(value.as_bytes())
                .and_then(|_| std::io::stdout().flush())
                .is_ok() =>
        {
            0
        }
        _ => 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn exported() -> Value {
        json!({"parameters": {"app": "winscp", "Host": "gateway.example", "host": "gateway.example", "Port": "60022", "port": "60022", "User": "alice/192.0.2.10/root", "user": "alice/192.0.2.10/root", "Path": "/目录", "path": "/目录", "pwd": "fixture%2F+unchanged", "PWD": "never-use-this", "st": "Fixture asset", "url": "https://unused.invalid/secret"}})
    }

    fn connection() -> Connection {
        parse(&serde_json::to_vec(&exported()).unwrap()).unwrap_or_else(|error| panic!("{error}"))
    }

    #[test]
    fn imports_endpoint_and_lowercase_credential_without_decoding_or_fallback() {
        let connection = connection();
        assert_eq!(connection.port, 60022);
        assert_eq!(connection.initial_directory, "/目录");
        assert_eq!(connection.title, "Fixture asset");
        assert_eq!(connection.secret.as_deref(), Some("fixture%2F+unchanged"));
        assert!(parse(&serde_json::to_vec(&exported()["parameters"]).unwrap()).is_ok());
        let mut missing = exported();
        missing["parameters"].as_object_mut().unwrap().remove("pwd");
        assert!(parse(&serde_json::to_vec(&missing).unwrap()).is_err());
    }

    #[test]
    fn rejects_conflicting_routes_proxy_non_sftp_and_control_characters() {
        for field in ["host", "user", "port", "path"] {
            let mut value = exported();
            value["parameters"][field] = json!("different");
            assert!(
                parse(&serde_json::to_vec(&value).unwrap()).is_err(),
                "{field}"
            );
        }
        for (field, value) in [
            ("proto", json!("scp")),
            ("app", json!("other")),
            ("proxy", json!({"addr":"proxy.example"})),
            ("proxy.addr", json!("proxy.example")),
            ("pwd", json!("bad\nsecret")),
            ("st", json!("bad\0title")),
        ] {
            let mut input = exported();
            input["parameters"][field] = value;
            assert!(
                parse(&serde_json::to_vec(&input).unwrap()).is_err(),
                "{field}"
            );
        }
        for (upper, lower, value) in [
            ("Host", "host", "-oProxyCommand=bad"),
            ("User", "user", "alice\nProxyCommand=bad"),
        ] {
            let mut input = exported();
            input["parameters"][upper] = json!(value);
            input["parameters"][lower] = json!(value);
            assert!(parse(&serde_json::to_vec(&input).unwrap()).is_err());
        }
        assert!(parse(&vec![b' '; MAX_IMPORT as usize + 1]).is_err());
    }

    #[test]
    fn independent_arguments_exclude_terminal_sharing_and_credentials() {
        let connection = connection();
        let args = connection.ssh_args();
        for option in [
            "ControlPath=none",
            "ControlMaster=no",
            "ControlPersist=no",
            "StrictHostKeyChecking=yes",
            "NumberOfPasswordPrompts=1",
            "ForwardAgent=no",
            "ForwardX11=no",
        ] {
            assert!(args.windows(2).any(|pair| pair == ["-o", option]));
        }
        assert!(args.ends_with(&["-s".into(), "gateway.example".into(), "sftp".into()]));
        assert!(!args.join(" ").contains("fixture%2F+unchanged"));
    }

    #[test]
    fn empty_protocol_defaults_to_sftp_and_proxy_formats_cannot_hide_a_route() {
        for protocol in [Value::Null, json!(""), json!("sftp")] {
            let mut input = exported();
            input["parameters"]["proto"] = protocol;
            input["parameters"]["proxy.addr"] = Value::Null;
            assert!(parse(&serde_json::to_vec(&input).unwrap()).is_ok());
        }
        for (flat, nested) in [
            (json!(""), json!("proxy.example")),
            (Value::Null, json!("proxy.example")),
            (json!("proxy.example"), json!("")),
        ] {
            let mut input = exported();
            input["parameters"]["proxy.addr"] = flat;
            input["parameters"]["proxy"] = json!({"addr":nested});
            assert!(parse(&serde_json::to_vec(&input).unwrap()).is_err());
        }
    }

    #[test]
    fn one_imported_password_is_scoped_and_otp_stays_manual() {
        let mut connection = connection();
        let auth = connection
            .prepare_auth()
            .unwrap_or_else(|error| panic!("{error}"));
        for prompt in [
            "2nd Password:",
            "OTP:",
            "(other@gateway.example) Password:",
            "(ALICE/192.0.2.10/root@gateway.example) Password:",
        ] {
            let result = answer(&auth.root, prompt, false, |_, display, first, confirm| {
                assert!(!first && !confirm);
                assert!(
                    display.starts_with("SFTP · alice/192.0.2.10/root@gateway.example:60022\n\n")
                );
                assert!(display.ends_with(prompt));
                assert!(auth.root.join("prompt-active").exists());
                Some("manual-fixture".into())
            });
            assert_eq!(result.as_deref(), Some("manual-fixture"));
            assert!(auth.root.join(CREDENTIAL).exists());
        }
        let prompt = "(alice/192.0.2.10/root@gateway.example) Password:";
        assert_eq!(
            answer(&auth.root, prompt, false, |_, _, _, _| panic!(
                "first password should use the import"
            ))
            .as_deref(),
            Some("fixture%2F+unchanged")
        );
        assert!(!auth.root.join(CREDENTIAL).exists());
        assert_eq!(
            answer(&auth.root, "2nd Password:", false, |_, display, first, _| {
                assert!(!first);
                assert!(!display.contains("凭据可能已失效"), "an explicit OTP must not be called a rejected export");
                Some("manual-otp".into())
            }).as_deref(),
            Some("manual-otp")
        );
        assert_eq!(
            answer(&auth.root, prompt, false, |_, display, first, _| {
                assert!(!first);
                assert!(display.contains("更新导出的参数后重连"));
                Some("manual-retry".into())
            })
            .as_deref(),
            Some("manual-retry")
        );
        assert!(
            !auth
                .env()
                .values()
                .any(|value| value.contains("fixture%2F+unchanged"))
        );
        let retry = connection
            .prepare_auth()
            .unwrap_or_else(|error| panic!("{error}"));
        assert_ne!(retry.root, auth.root);
        assert!(!retry.root.join(CREDENTIAL).exists());
        auth.cancel();
        assert!(
            answer(&auth.root, prompt, false, |_, _, _, _| panic!(
                "closed connection must not prompt"
            ))
            .is_none()
        );
    }

    #[test]
    fn simultaneous_connections_and_host_confirmation_do_not_share_credentials() {
        let mut first = connection();
        let mut input = exported();
        input["parameters"]["pwd"] = json!("second-fixture-password");
        let mut second =
            parse(&serde_json::to_vec(&input).unwrap()).unwrap_or_else(|error| panic!("{error}"));
        let a = first
            .prepare_auth()
            .unwrap_or_else(|error| panic!("{error}"));
        let b = second
            .prepare_auth()
            .unwrap_or_else(|error| panic!("{error}"));
        assert_ne!(a.root, b.root);
        assert_eq!(
            answer(
                &a.root,
                "Confirm host (yes/no)?",
                true,
                |_, _, first, confirm| {
                    assert!(!first && confirm);
                    Some("yes".into())
                }
            )
            .as_deref(),
            Some("yes")
        );
        assert!(a.root.join(CREDENTIAL).exists());
        assert_eq!(
            answer(&a.root, "Password:", false, |_, _, _, _| None).as_deref(),
            Some("fixture%2F+unchanged")
        );
        assert!(b.root.join(CREDENTIAL).exists());
        assert_eq!(
            answer(&b.root, "Password:", false, |_, _, _, _| None).as_deref(),
            Some("second-fixture-password")
        );
    }

    #[cfg(unix)]
    #[test]
    fn credentials_and_scope_are_private_and_removed_on_drop() {
        use std::os::unix::fs::PermissionsExt;
        let mut connection = connection();
        let auth = connection
            .prepare_auth()
            .unwrap_or_else(|error| panic!("{error}"));
        let root = auth.root.clone();
        assert_eq!(
            std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        for name in [CREDENTIAL, METADATA] {
            assert_eq!(
                std::fs::metadata(root.join(name))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        drop(auth);
        assert!(!root.exists());
    }
}

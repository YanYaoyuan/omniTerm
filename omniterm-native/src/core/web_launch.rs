// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! A single web-authorized SSH window. Opaque launch credentials never become
//! saved profiles, command arguments, environment values or terminal input.

use super::{accessclient::Envelope, plan, profile::{Kind, Profile}};
use crate::remote::qizhi::{Auth, Connection};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// No Debug/Serialize: an incoming file connection still owns its credential.
pub enum Incoming {
    Ssh(std::sync::Arc<Launch>),
    Sftp(Connection),
}

impl Incoming {
    pub fn from_wire(wire: &str) -> Result<Self, String> {
        let data = Envelope::decode(wire).map_err(String::from)?.into_document();
        match data.get("app").and_then(Value::as_str) {
            Some("putty") => Launch::from_document(data).map(|launch| Self::Ssh(std::sync::Arc::new(launch))),
            Some("winscp") => Connection::from_web(&data).map(Self::Sftp),
            _ => Err("当前网页接入支持 PuTTY 字符会话和 WinSCP 文件会话参数。".into()),
        }
    }
}

pub struct Launch {
    profile: Profile,
    authentication: Auth,
    config: PathBuf,
    used: AtomicBool,
}

impl std::fmt::Debug for Launch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WebAuthorizedSshLaunch")
    }
}

fn text<'a>(data: &'a Value, key: &str, maximum: usize) -> Result<&'a str, String> {
    data.get(key).and_then(Value::as_str)
        .filter(|v| !v.is_empty() && v.len() <= maximum && !v.chars().any(char::is_control))
        .ok_or_else(|| "网页 SSH 参数缺失或格式不受支持。".to_string())
}

fn nonempty(value: &Value) -> bool {
    !value.is_null() && !value.as_str().is_some_and(|s| s.trim().is_empty())
}

impl Launch {
    fn from_document(data: Value) -> Result<Self, String> {
        if !data.is_object() || data.get("app").and_then(Value::as_str) != Some("putty") {
            return Err("当前网页 SSH 接入支持 PuTTY 类型的字符会话参数。".into());
        }
        for key in ["env", "proxy.addr", "proxy_protocol", "guiservice_token", "wsurl", "url_path", "header", "batch_url", "scsession_url_path", "opc_host", "opc_port", "opc_header", "sessionData", "session_data", "control_mode"] {
            if data.get(key).is_some_and(nonempty) {
                return Err("该网页会话需要尚未支持的代理或扩展参数。".into());
            }
        }
        if data.get("proxy").is_some_and(|v| nonempty(v) && !v.as_object().is_some_and(|p| p.is_empty() || (p.len() == 1 && p.get("addr").is_some_and(|v| !nonempty(v))))) {
            return Err("该网页会话需要尚未支持的代理参数。".into());
        }
        if data.get("pr").is_some_and(|v| nonempty(v) && !v.as_str().is_some_and(|s| s.eq_ignore_ascii_case("ssh"))) {
            return Err("该网页字符会话不是受支持的 SSH 协议。".into());
        }
        let host = text(&data, "hn", 255)?;
        if host.contains('@') { return Err("网页 SSH 主机格式无效。".into()); }
        let port = text(&data, "pn", 5)?.parse::<u16>().ok().filter(|n| *n > 0)
            .ok_or("网页 SSH 端口无效。")?;
        let user = text(&data, "un", 128)?;
        let asset = text(&data, "sh", 255)?;
        let account = text(&data, "sa", 128)?;
        let title = data.get("st").and_then(Value::as_str).filter(|s| !s.is_empty())
            .map(String::from).unwrap_or_else(|| format!("{account}@{asset}"));
        let profile = Profile { kind: Some(Kind::Ssh), host: Some(host.into()), username: Some(user.into()), port: Some(port), name: Some(title), ..Profile::default() }.validate(true)?;
        let secret = text(&data, "pw", 4096)?.to_string();
        let session = text(&data, "ws", 4096)?;
        let mut connection = Connection::web_ssh(&profile, secret)?;
        let (authentication, config) = connection.prepare_terminal_auth(session)?;
        Ok(Self { profile, authentication, config, used: AtomicBool::new(false) })
    }

    pub fn profile(&self) -> &Profile { &self.profile }

    pub fn build(&self, request: &plan::Request<'_>) -> Result<plan::Plan, String> {
        if request.kind != Kind::Ssh || self.used.swap(true, Ordering::AcqRel) {
            return Err("网页授权已使用，请从堡垒机网页重新打开资产。".into());
        }
        let request = plan::Request { profile: Some(&self.profile), companion: false, ..*request };
        let mut plan = plan::build(&request, || None)?;
        let mut args = vec!["-F".to_string(), self.config.to_string_lossy().into_owned(), "-tt".into(), "-p".into(), self.profile.port.unwrap_or(22).to_string()];
        for option in [format!("User={}", self.profile.username.as_deref().unwrap_or_default()), "ControlMaster=no".into(), "ControlPath=none".into(), "ControlPersist=no".into(), "StrictHostKeyChecking=ask".into(), "NumberOfPasswordPrompts=1".into(), "PreferredAuthentications=password,keyboard-interactive".into(), "PubkeyAuthentication=no".into(), "IdentityAgent=none".into(), "PasswordAuthentication=yes".into(), "KbdInteractiveAuthentication=yes".into(), "ForwardAgent=no".into(), "ForwardX11=no".into(), "ClearAllForwardings=yes".into(), "RemoteCommand=none".into(), "ConnectTimeout=15".into(), "ConnectionAttempts=1".into()] {
            args.extend(["-o".into(), option]);
        }
        args.push(self.profile.host.clone().unwrap_or_default());
        plan.host["args"] = serde_json::json!(args);
        let env = plan.host["env"].as_object_mut().ok_or("无法准备网页 SSH 环境。")?;
        for key in ["OMNITERM_SSH_ASKPASS", "OMNITERM_SSH_AUTH_SCOPE", "OMNITERM_SSH_AUTH_STATE", "OMNITERM_QIZHI_ASKPASS", "OMNITERM_QIZHI_AUTH_STATE", "SHTERM_WEBSID"] { env.remove(key); }
        env.extend(self.authentication.env().iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))));
        env.insert("TERM".into(), Value::String("vt100".into()));
        plan.companion = None;
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::settings::LocalShell;
    use serde_json::json;

    fn packet() -> Value { json!({"app":"putty","hn":"gateway.example","pn":"60022","un":"alice","sh":"192.0.2.10","sa":"root","st":"Fixture asset","pw":"fixture-credential","ws":"fixture-session-reference","env":"","proxy.addr":null}) }

    #[test]
    fn web_authorization_is_private_single_use_and_does_not_create_companions() {
        let launch = Launch::from_document(packet()).unwrap();
        let config = launch.config.clone();
        assert!(std::fs::read_to_string(&config).unwrap().contains("SHTERM_WEBSID=fixture-session-reference"));
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(config.metadata().unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(config.parent().unwrap().metadata().unwrap().permissions().mode() & 0o777, 0o700);
        }
        let cwd = std::env::current_dir().unwrap();
        let shell = LocalShell::default();
        let request = plan::Request { kind: Kind::Ssh, cwd: cwd.to_str().unwrap(), profile: Some(launch.profile()), companion: true, local_shell: &shell };
        let plan = launch.build(&request).unwrap();
        assert!(plan.companion.is_none());
        let exposed = format!("{:?} {} {}", launch, plan.host, serde_json::to_string(launch.profile()).unwrap());
        for value in ["fixture-credential", "fixture-session-reference"] { assert!(!exposed.contains(value)); }
        assert!(launch.build(&request).is_err());
        drop(launch);
        assert!(!config.exists(), "closing the owning session cleans authorization state");
    }

    #[test]
    fn unsupported_launches_fail_without_using_external_programs_or_urls() {
        for (key, value) in [("app", json!("mstsc")), ("hn", json!("-oProxyCommand=bad")), ("pn", json!("0")), ("ws", json!("bad\nSetEnv EVIL=x")), ("pw", json!("bad\nsecret")), ("env", json!("run-untrusted")), ("proxy.addr", json!("proxy.example")), ("control_mode", json!("unsupported"))] {
            let mut data = packet(); data[key] = value;
            assert!(Launch::from_document(data).is_err(), "{key}");
        }
        assert!(Launch::from_document(json!([packet()])).is_err());
    }

    fn file_packet() -> Value {
        json!({"app":"winscp","host":"gateway.example","port":"60022","user":"alice","pwd":"fixture-credential","PWD":"never-use-uppercase","st":"Fixture files","path":"/","proxy.addr":null})
    }

    #[test]
    fn winscp_envelope_dispatches_to_the_file_connection() {
        use base64::Engine;
        use std::io::Write;
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&serde_json::to_vec(&file_packet()).unwrap()).unwrap();
        let wire = format!("accessclient://{}", base64::engine::general_purpose::STANDARD.encode(encoder.finish().unwrap()));
        assert!(matches!(Incoming::from_wire(&wire).unwrap(), Incoming::Sftp(_)));
    }

    #[test]
    #[ignore = "needs the isolated loopback web SFTP fixture and built app helper"]
    fn web_authorized_sftp_end_to_end() {
        use crate::remote::manager::{Manager, Phase, TransferStatus};
        use std::time::{Duration, Instant};
        let mut data = file_packet();
        data["host"] = json!("127.0.0.1"); data["port"] = json!(std::env::var("OMNITERM_WEB_TEST_PORT").unwrap());
        let connection = Connection::from_web(&data).unwrap();
        let manager = Manager::new(|| {});
        manager.attach_qizhi("web-files-fixture", connection).unwrap();
        let remote = manager.get("web-files-fixture").unwrap();
        let wait = |ready: &dyn Fn() -> bool| {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !ready() {
                assert!(Instant::now() < deadline, "web SFTP operation timed out");
                assert!(!matches!(remote.snapshot().phase, Phase::Unavailable { .. }), "web SFTP became unavailable");
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        wait(&|| remote.snapshot().phase == Phase::Ready);
        remote.list("/".into());
        wait(&|| remote.snapshot().listing.entries.iter().any(|entry| entry.name == "fixture.txt"));
        let local = std::env::temp_dir().join(format!("omniterm-web-sftp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&local).unwrap();
        let source = local.join("web-upload.txt");
        std::fs::write(&source, b"web-sftp-fixture-round-trip").unwrap();
        remote.upload(vec![source], "/".into());
        wait(&|| remote.snapshot().transfers.iter().any(|item| item.upload && item.status == TransferStatus::Done));
        let downloads = local.join("downloads"); std::fs::create_dir(&downloads).unwrap();
        remote.download(vec!["/web-upload.txt".into()], downloads.clone()).unwrap();
        wait(&|| remote.snapshot().transfers.iter().any(|item| !item.upload && item.status == TransferStatus::Done));
        assert_eq!(std::fs::read(downloads.join("web-upload.txt")).unwrap(), b"web-sftp-fixture-round-trip");
        assert!(!remote.snapshot().metrics_available);
        manager.shutdown();
        std::fs::remove_dir_all(local).unwrap();
    }

    #[test]
    #[ignore = "needs the isolated loopback web SSH fixture and built app helper"]
    fn web_authorized_ssh_end_to_end() {
        let port = std::env::var("OMNITERM_WEB_TEST_PORT").unwrap();
        let known = std::env::var("OMNITERM_WEB_TEST_KNOWN_HOSTS").unwrap();
        let helper = std::env::var("OMNITERM_WEB_TEST_APP").unwrap();
        let mut data = packet();
        data["hn"] = json!("127.0.0.1"); data["pn"] = json!(port);
        let launch = Launch::from_document(data).unwrap();
        let cwd = std::env::current_dir().unwrap(); let shell = LocalShell::default();
        let request = plan::Request { kind: Kind::Ssh, cwd: cwd.to_str().unwrap(), profile: None, companion: true, local_shell: &shell };
        let plan = launch.build(&request).unwrap();
        let mut args: Vec<String> = serde_json::from_value(plan.host["args"].clone()).unwrap();
        args.splice(args.len()-1..args.len()-1, ["-o".into(), format!("UserKnownHostsFile={known}")]);
        let env: std::collections::BTreeMap<String, String> = serde_json::from_value(plan.host["env"].clone()).unwrap();
        let result = std::process::Command::new(plan.host["executable"].as_str().unwrap()).args(args)
            .envs(env).env("SSH_ASKPASS", helper).stdin(std::process::Stdio::null()).output().unwrap();
        assert!(result.status.success(), "fixture SSH failed without replaying credentials");
        assert!(String::from_utf8_lossy(&result.stdout).contains("fixture-web-asset-connected"));
        assert!(!String::from_utf8_lossy(&result.stderr).contains("fixture-credential"));
        let root = launch.config.parent().unwrap().to_path_buf();
        assert!(!root.join("credential").exists(), "the helper must consume the credential file once");
        drop(launch);
        assert!(!root.exists(), "the web session owns all private authentication state");
    }
}

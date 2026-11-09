// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Saved connections, in the desktop app's `profiles.json`, validated the same way: every field
//! that later becomes a command-line argument is checked here, before it is stored or used.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const MAX_PROFILES: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Shell,
    Agent,
    Ssh,
    Sftp,
    Serial,
    Tunnel,
}

impl Kind {
    pub const CONNECTIONS: [Kind; 4] = [Kind::Ssh, Kind::Sftp, Kind::Serial, Kind::Tunnel];

    pub fn remote(self) -> bool {
        matches!(self, Kind::Ssh | Kind::Sftp | Kind::Tunnel)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Forward {
    L,
    R,
    D,
}

/// X11 forwarding for an SSH session, as ssh's `-X` and `-Y`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum X11 {
    #[default]
    Off,
    /// `-X`: the remote program gets an untrusted connection, kept out of other windows.
    Restricted,
    /// `-Y`: full access to the display, which some programs need.
    Trusted,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<Kind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_jump: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baud: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forward_type: Option<Forward>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_port: Option<u16>,
    /// Kept out of `profiles.json`, which the desktop app shares and rejects unknown fields
    /// in; the store keeps it beside, by profile id.
    #[serde(skip)]
    pub x11: X11,
    /// Native-only opt-in; passwords themselves live exclusively in the OS keychain.
    #[serde(skip)]
    pub remember_password: bool,
}

fn text(value: &Option<String>, label: &str, maximum: usize) -> Result<Option<String>, String> {
    match value.as_deref() {
        None | Some("") => Ok(None),
        Some(value) if value.chars().count() > maximum || value.chars().any(|c| c.is_control()) => {
            Err(format!("{label} 必须是不含控制字符的文本（最多 {maximum} 个字符）。"))
        }
        Some(value) => Ok(Some(value.trim().to_string()).filter(|value| !value.is_empty())),
    }
}

/// A hostname, SSH alias or IP address, optionally `user@` first; never an option to ssh.
fn host(value: &str, label: &str, allow_user: bool) -> Result<String, String> {
    let ok = !value.is_empty()
        && !value.starts_with('-')
        && value.chars().all(|c| c.is_ascii_alphanumeric() || "_.:@[]%-".contains(c))
        && (allow_user || !value.contains('@'))
        && value.matches('@').count() <= 1;
    if !ok {
        return Err(format!("{label} 必须是主机名、SSH 别名或 IP 地址，且不能以“-”开头。"));
    }
    if let Some((user, rest)) = value.split_once('@')
        && (user.is_empty() || rest.is_empty() || rest.starts_with('-'))
    {
        return Err(format!("{label} 不是有效的 SSH 目标。"));
    }
    Ok(value.to_string())
}

impl Profile {
    /// Checks a profile for `kind`. `complete` also requires what a connection needs to start.
    pub fn validate(&self, complete: bool) -> Result<Profile, String> {
        let kind = self.kind.ok_or("连接类型缺失。")?;
        let mut out = Profile { kind: Some(kind), ..Profile::default() };
        out.id = text(&self.id, "id", 80)?;
        if let Some(id) = &out.id
            && !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err("连接 id 无效。".into());
        }
        out.name = text(&self.name, "名称", 120)?;
        out.key_path = text(&self.key_path, "私钥路径", 4096)?;
        out.summary = text(&self.summary, "说明", 4096)?;
        out.device = text(&self.device, "设备", 4096)?;
        out.host = text(&self.host, "主机", 255)?.map(|value| host(&value, "主机", true)).transpose()?;
        out.username = text(&self.username, "用户名", 128)?;
        if let Some(user) = &out.username
            && !user.split('/').all(|part| part.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') && part.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c)))
        {
            return Err("SSH 用户名无效。".into());
        }
        if let Some(jump) = text(&self.proxy_jump, "ProxyJump", 1024)? {
            let hops: Vec<String> = jump.split(',').map(|hop| hop.trim().to_string()).collect();
            for hop in &hops {
                host(hop.strip_prefix("ssh://").unwrap_or(hop), "ProxyJump 主机", true)?;
            }
            out.proxy_jump = Some(hops.join(","));
        }
        if out.host.as_deref().is_some_and(|host| host.contains('@')) && out.username.is_some() {
            return Err("用户名请填在“用户名”或主机里，不要两处都填。".into());
        }
        for (value, label) in [(self.port, "端口"), (self.bind_port, "本地端口"), (self.target_port, "目标端口")] {
            if value == Some(0) {
                return Err(format!("{label} 必须是 1–65535。"));
            }
        }
        out.port = self.port;
        out.bind_port = self.bind_port;
        out.target_port = self.target_port;
        if let Some(baud) = self.baud
            && !(1..=4_000_000).contains(&baud)
        {
            return Err("波特率必须是 1–4000000。".into());
        }
        out.baud = self.baud;
        out.target_host = text(&self.target_host, "目标主机", 255)?.map(|value| host(&value, "目标主机", false)).transpose()?;
        out.forward_type = self.forward_type;
        out.x11 = if kind == Kind::Ssh { self.x11 } else { X11::Off };
        out.remember_password = kind == Kind::Ssh && self.remember_password;
        if complete {
            if matches!(kind, Kind::Ssh | Kind::Sftp | Kind::Tunnel) && out.host.is_none() {
                return Err("需要填写主机或 SSH 别名。".into());
            }
            if kind == Kind::Serial {
                let device = out.device.as_deref().ok_or("需要填写串口设备（例如 /dev/ttyUSB0 或 COM3）。")?;
                if device.starts_with('-') {
                    return Err("串口设备不能以“-”开头。".into());
                }
            }
            if kind == Kind::Tunnel {
                let forward = *out.forward_type.get_or_insert(Forward::L);
                if out.bind_port.is_none() {
                    return Err("需要填写隧道的本地监听端口。".into());
                }
                if forward != Forward::D && (out.target_host.is_none() || out.target_port.is_none()) {
                    return Err("本地或远程转发需要目标主机和端口。".into());
                }
            }
        }
        Ok(out)
    }

    pub fn display_name(&self) -> String {
        self.name.clone().or_else(|| self.host.clone()).or_else(|| self.device.clone()).unwrap_or_else(|| "终端".into())
    }
}

pub struct Store {
    path: PathBuf,
    /// X11 forwarding per profile id, which `profiles.json` cannot hold.
    extras: PathBuf,
    ssh_passwords: PathBuf,
}

impl Store {
    pub fn new(path: PathBuf) -> Self {
        let extras = path.with_file_name("native-profile-x11.json");
        let ssh_passwords = path.with_file_name("native-profile-ssh-auth.json");
        Self { path, extras, ssh_passwords }
    }

    fn x11_modes(&self) -> std::collections::HashMap<String, X11> {
        std::fs::read(&self.extras).ok().filter(|contents| contents.len() <= 1 << 20).and_then(|contents| serde_json::from_slice(&contents).ok()).unwrap_or_default()
    }

    pub fn list(&self) -> Result<Vec<Profile>, String> {
        let contents = match std::fs::read(&self.path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.to_string()),
        };
        if contents.len() > 1 << 20 {
            return Err("保存的连接文件过大。".into());
        }
        let profiles: Vec<Profile> = serde_json::from_slice(&contents).map_err(|_| "保存的连接文件无效。".to_string())?;
        if profiles.len() > MAX_PROFILES {
            return Err("保存的连接文件无效。".into());
        }
        let mut ids = std::collections::HashSet::new();
        let modes = self.x11_modes();
        let passwords: std::collections::HashMap<String, bool> = std::fs::read(&self.ssh_passwords).ok().filter(|bytes| bytes.len() <= 1 << 20).and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
        profiles
            .iter()
            .map(|profile| {
                let mut valid = profile.validate(true)?;
                if valid.kind == Some(Kind::Ssh) {
                    valid.x11 = valid.id.as_ref().and_then(|id| modes.get(id)).copied().unwrap_or_default();
                    valid.remember_password = valid.id.as_ref().and_then(|id| passwords.get(id)).copied().unwrap_or(false);
                }
                match &valid.id {
                    Some(id) if ids.insert(id.clone()) => Ok(valid),
                    _ => Err("保存的连接文件里有无效或重复的连接。".to_string()),
                }
            })
            .collect()
    }

    fn write(&self, profiles: &[Profile]) -> Result<(), String> {
        let mut json = serde_json::to_vec_pretty(profiles).map_err(|error| error.to_string())?;
        json.push(b'\n');
        super::paths::write_atomically(&self.path, &json).map_err(|error| format!("无法保存连接：{error}"))?;
        let modes: std::collections::BTreeMap<&str, X11> = profiles.iter().filter(|profile| profile.x11 != X11::Off).filter_map(|profile| Some((profile.id.as_deref()?, profile.x11))).collect();
        let mut json = serde_json::to_vec_pretty(&modes).map_err(|error| error.to_string())?;
        json.push(b'\n');
        super::paths::write_atomically(&self.extras, &json).map_err(|error| format!("无法保存连接：{error}"))?;
        let modes: std::collections::BTreeMap<&str, bool> = profiles.iter().filter(|profile| profile.remember_password).filter_map(|profile| Some((profile.id.as_deref()?, true))).collect();
        let json = serde_json::to_vec_pretty(&modes).map_err(|error| error.to_string())?;
        super::paths::write_atomically(&self.ssh_passwords, &json).map_err(|error| format!("无法保存 SSH 认证偏好：{error}"))
    }

    pub fn save(&self, input: &Profile) -> Result<Vec<Profile>, String> {
        let mut profile = input.validate(true)?;
        profile.id.get_or_insert_with(|| uuid::Uuid::new_v4().to_string());
        if profile.name.is_none() {
            profile.name = Some(profile.display_name());
        }
        let mut profiles = self.list()?;
        match profiles.iter().position(|item| item.id == profile.id) {
            Some(index) => profiles[index] = profile,
            None if profiles.len() >= MAX_PROFILES => return Err(format!("最多可以保存 {MAX_PROFILES} 个连接。")),
            None => profiles.push(profile),
        }
        self.write(&profiles)?;
        Ok(profiles)
    }

    /// All or nothing: a batch that cannot fit is refused before anything is written.
    pub fn import(&self, inputs: &[Profile]) -> Result<Vec<Profile>, String> {
        let mut profiles = self.list()?;
        let mut added = Vec::new();
        for input in inputs {
            if profiles.iter().chain(&added).any(|profile: &Profile| profile.kind == input.kind && profile.host == input.host) {
                continue;
            }
            let mut profile = input.validate(true)?;
            profile.id = Some(uuid::Uuid::new_v4().to_string());
            added.push(profile);
        }
        if profiles.len() + added.len() > MAX_PROFILES {
            return Err(format!("最多可以保存 {MAX_PROFILES} 个连接，还能再添加 {} 个。", MAX_PROFILES - profiles.len()));
        }
        if !added.is_empty() {
            profiles.extend(added);
            self.write(&profiles)?;
        }
        Ok(profiles)
    }

    pub fn delete(&self, id: &str) -> Result<Vec<Profile>, String> {
        let profiles: Vec<Profile> = self.list()?.into_iter().filter(|profile| profile.id.as_deref() != Some(id)).collect();
        self.write(&profiles)?;
        Ok(profiles)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssh(host: &str) -> Profile {
        Profile { kind: Some(Kind::Ssh), host: Some(host.into()), ..Profile::default() }
    }

    #[test]
    fn x11_forwarding_stays_out_of_the_shared_profiles_file() {
        let directory = std::env::temp_dir().join(format!("omniterm-profiles-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let store = Store::new(directory.join("profiles.json"));
        let saved = store.save(&Profile { x11: X11::Trusted, ..ssh("box") }).unwrap();
        let shared = std::fs::read_to_string(directory.join("profiles.json")).unwrap();
        assert!(!shared.contains("x11") && !shared.contains("trusted"), "{shared}");
        assert_eq!(store.list().unwrap()[0].x11, X11::Trusted);
        assert_eq!(Profile { kind: Some(Kind::Tunnel), bind_port: Some(1), forward_type: Some(Forward::D), x11: X11::Trusted, ..ssh("box") }.validate(true).unwrap().x11, X11::Off);
        let id = saved[0].id.clone().unwrap();
        store.save(&Profile { id: Some(id), x11: X11::Off, ..ssh("box") }).unwrap();
        assert_eq!(store.list().unwrap()[0].x11, X11::Off);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn hosts_can_never_become_ssh_options() {
        assert!(ssh("-oProxyCommand=x").validate(true).is_err());
        assert!(ssh("a b").validate(true).is_err());
        assert!(ssh("user@").validate(true).is_err());
        assert!(ssh("deploy@10.0.0.1").validate(true).is_ok());
        assert!(Profile { username: Some("me".into()), ..ssh("deploy@box") }.validate(true).is_err());
    }

    #[test]
    fn bastion_routing_usernames_are_allowed_without_options_or_control_characters() {
        for user in ["alice", "alice/192.0.2.10/root", "alice/asset-name/_ubuntu"] {
            assert_eq!(Profile { username: Some(user.into()), ..ssh("gateway") }.validate(true).unwrap().username.as_deref(), Some(user));
        }
        for user in ["/root", "alice//root", "alice/", "-oProxyCommand=x", "alice/-oProxyCommand", "alice/asset/root\n", "alice/asset/root;id", "alice/asset/root@host", "alice/asset/root x"] {
            assert!(Profile { username: Some(user.into()), ..ssh("gateway") }.validate(true).is_err());
        }
    }

    #[test]
    fn remembering_ssh_passwords_keeps_only_a_boolean_outside_shared_profiles() {
        let directory = std::env::temp_dir().join(format!("omniterm-auth-profiles-{}", uuid::Uuid::new_v4()));
        let store = Store::new(directory.join("profiles.json"));
        let saved = store.save(&Profile { remember_password: true, ..ssh("gateway") }).unwrap();
        assert!(store.list().unwrap()[0].remember_password);
        let shared = std::fs::read_to_string(directory.join("profiles.json")).unwrap();
        assert!(!shared.contains("rememberPassword"));
        assert!(!shared.contains("password"));
        let preferences: serde_json::Value = serde_json::from_slice(&std::fs::read(directory.join("native-profile-ssh-auth.json")).unwrap()).unwrap();
        assert_eq!(preferences[saved[0].id.as_ref().unwrap()], true);
        store.save(&Profile { id: saved[0].id.clone(), remember_password: false, ..ssh("gateway") }).unwrap();
        assert!(!store.list().unwrap()[0].remember_password);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn tunnels_need_their_ports_and_default_to_local_forwarding() {
        let tunnel = Profile { kind: Some(Kind::Tunnel), host: Some("box".into()), bind_port: Some(8080), target_host: Some("localhost".into()), target_port: Some(80), ..Profile::default() };
        assert_eq!(tunnel.validate(true).unwrap().forward_type, Some(Forward::L));
        assert!(Profile { bind_port: None, ..tunnel.clone() }.validate(true).is_err());
        assert!(Profile { target_port: None, forward_type: Some(Forward::D), ..tunnel }.validate(true).is_ok());
    }

    #[test]
    fn profiles_round_trip_through_the_desktop_file_format() {
        let json = r#"[{"id":"a1","kind":"ssh","name":"prod","host":"box","port":2222,"proxyJump":"jump1, jump2"}]"#;
        let profiles: Vec<Profile> = serde_json::from_str(json).unwrap();
        let valid = profiles[0].validate(true).unwrap();
        assert_eq!(valid.proxy_jump.as_deref(), Some("jump1,jump2"));
        assert!(serde_json::to_string(&valid).unwrap().contains(r#""proxyJump":"jump1,jump2""#));
        assert!(serde_json::from_str::<Vec<Profile>>(r#"[{"kind":"ssh","password":"x"}]"#).is_err(), "secrets are never stored");
    }

    #[test]
    fn the_store_saves_imports_and_deletes() {
        let directory = std::env::temp_dir().join(format!("omniterm-profiles-{}", uuid::Uuid::new_v4()));
        let store = Store::new(directory.join("profiles.json"));
        let saved = store.save(&ssh("box")).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].name.as_deref(), Some("box"));
        assert_eq!(store.import(&[ssh("box"), ssh("other")]).unwrap().len(), 2);
        let id = saved[0].id.clone().unwrap();
        assert_eq!(store.delete(&id).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(directory);
    }
}

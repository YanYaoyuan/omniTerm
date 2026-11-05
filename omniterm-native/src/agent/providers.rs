// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Model providers, in the desktop app's `model-providers.json`, and their API keys, which
//! live in the operating system's keychain and never in a file anyone with the file could read.

use crate::core::paths;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

pub const MAX_PROFILES: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireApi {
    Responses,
    Chat,
    AnthropicMessages,
}

impl WireApi {
    pub fn label(self) -> &'static str {
        match self {
            WireApi::Responses => "Responses",
            WireApi::Chat => "Chat Completions",
            WireApi::AnthropicMessages => "Anthropic Messages",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Auth {
    ApiKey,
    EngineLogin,
}

pub struct Preset {
    pub id: &'static str,
    pub label: &'static str,
    pub wire: WireApi,
    pub base_url: &'static str,
    pub protocols: &'static [WireApi],
    pub auth: Auth,
}

/// Base URLs are starting points; regions and gateways differ, so each stays editable.
pub const PRESETS: &[Preset] = &[
    Preset { id: "openai", label: "OpenAI", wire: WireApi::Responses, base_url: "https://api.openai.com/v1", protocols: &[WireApi::Responses], auth: Auth::ApiKey },
    Preset { id: "openai-login", label: "OpenAI（现有登录）", wire: WireApi::Responses, base_url: "", protocols: &[WireApi::Responses], auth: Auth::EngineLogin },
    Preset { id: "qwen", label: "Qwen / 阿里云百炼", wire: WireApi::Chat, base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1", protocols: &[WireApi::Chat, WireApi::Responses], auth: Auth::ApiKey },
    Preset { id: "anthropic", label: "Claude / Anthropic", wire: WireApi::AnthropicMessages, base_url: "https://api.anthropic.com/v1", protocols: &[WireApi::AnthropicMessages], auth: Auth::ApiKey },
    Preset { id: "kimi", label: "Kimi / Moonshot", wire: WireApi::Chat, base_url: "https://api.moonshot.ai/v1", protocols: &[WireApi::Chat], auth: Auth::ApiKey },
    Preset { id: "openrouter", label: "OpenRouter", wire: WireApi::Chat, base_url: "https://openrouter.ai/api/v1", protocols: &[WireApi::Chat, WireApi::Responses], auth: Auth::ApiKey },
    Preset { id: "custom", label: "自定义端点", wire: WireApi::Chat, base_url: "", protocols: &[WireApi::Chat, WireApi::Responses, WireApi::AnthropicMessages], auth: Auth::ApiKey },
];

pub fn preset(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|preset| preset.id == id)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Verified {
    #[serde(default)]
    pub connection: bool,
    #[serde(default)]
    pub tools: bool,
    #[serde(default)]
    pub at: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    #[serde(default)]
    pub id: String,
    pub preset_id: String,
    pub name: String,
    #[serde(default = "api_key")]
    pub auth: Auth,
    pub wire_api: WireApi,
    #[serde(default)]
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub chat_only: bool,
    #[serde(default = "default_budget")]
    pub context_budget_bytes: u32,
    #[serde(default)]
    pub verified: Verified,
}

fn api_key() -> Auth {
    Auth::ApiKey
}

fn default_budget() -> u32 {
    64 * 1024
}

fn text(value: &str, label: &str, maximum: usize) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("请输入{label}。"));
    }
    if value.chars().count() > maximum {
        return Err(format!("{label}不能超过 {maximum} 个字符。"));
    }
    if value.chars().any(char::is_control) {
        return Err(format!("{label}不能包含控制字符。"));
    }
    Ok(value.to_string())
}

/// https only, except plain http to this machine; no credentials or query in the URL.
pub fn validate_base_url(value: &str) -> Result<String, String> {
    let input = text(value, "Base URL", 2048)?;
    let invalid = || "Base URL 格式不正确，请填写完整地址，例如 https://example.com/v1。".to_string();
    let (scheme, rest) = input.split_once("://").ok_or_else(invalid)?;
    let scheme = scheme.to_ascii_lowercase();
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains(' ') {
        return Err(invalid());
    }
    let host = if authority.starts_with('[') { authority.split(']').next().map(|h| format!("{h}]")).unwrap_or_default() } else { authority.rsplit_once(':').map(|(host, _)| host).unwrap_or(authority).to_ascii_lowercase() };
    let loopback = matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]");
    if !(scheme == "https" || (scheme == "http" && loopback)) {
        return Err("Base URL 必须使用 https（仅 localhost 可以使用 http）。".into());
    }
    if authority.contains('@') || rest.contains('?') || rest.contains('#') {
        return Err("Base URL 不能包含账号、密码或查询参数，API Key 请填写在单独的字段中。".into());
    }
    Ok(format!("{scheme}://{rest}").trim_end_matches('/').to_string())
}

impl Profile {
    pub fn validate(mut self) -> Result<Profile, String> {
        let preset = preset(&self.preset_id).ok_or("未知的模型服务预设。")?;
        if !preset.protocols.contains(&self.wire_api) {
            return Err(format!("{} 不支持这个协议。", preset.label));
        }
        if self.id.is_empty() {
            self.id = uuid::Uuid::new_v4().to_string();
        } else if !self.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') || self.id.len() > 80 {
            return Err("配置 id 无效。".into());
        }
        if !(4096..=512 * 1024).contains(&self.context_budget_bytes) {
            return Err("上下文预算必须是 4–512 KiB。".into());
        }
        self.auth = preset.auth;
        self.name = text(&self.name, "配置名称", 120)?;
        self.base_url = if self.auth == Auth::EngineLogin { String::new() } else { validate_base_url(&self.base_url)? };
        self.model = text(&self.model, "模型 ID", 200)?;
        Ok(self)
    }
}

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    profiles: Vec<serde_json::Value>,
}

pub struct Store {
    path: std::path::PathBuf,
}

impl Store {
    pub fn new() -> Self {
        Self { path: paths::file("model-providers.json") }
    }

    pub fn list(&self) -> Result<Vec<Profile>, String> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err("模型设置文件无法读取。".into()),
        };
        let file: File = serde_json::from_slice(&bytes).map_err(|_| "模型设置文件无法读取。".to_string())?;
        Ok(file.profiles.into_iter().filter_map(|value| serde_json::from_value::<Profile>(value).ok()?.validate().ok()).collect())
    }

    fn write(&self, profiles: &[Profile]) -> Result<(), String> {
        let profiles = profiles.iter().map(|profile| serde_json::to_value(profile).unwrap_or_default()).collect();
        let mut json = serde_json::to_vec_pretty(&File { version: 1, profiles }).map_err(|error| error.to_string())?;
        json.push(b'\n');
        paths::write_atomically(&self.path, &json).map_err(|error| format!("无法保存模型设置：{error}"))
    }

    pub fn get(&self, id: &str) -> Result<Profile, String> {
        self.list()?.into_iter().find(|profile| profile.id == id).ok_or_else(|| "请先选择一个模型配置。".into())
    }

    /// Editing what a profile connects to voids what was verified about it.
    pub fn save(&self, input: Profile) -> Result<Profile, String> {
        let mut profile = input.validate()?;
        let mut profiles = self.list()?;
        match profiles.iter().position(|item| item.id == profile.id) {
            Some(index) => {
                let old = &profiles[index];
                if old.wire_api != profile.wire_api || old.base_url != profile.base_url || old.model != profile.model {
                    profile.verified = Verified::default();
                }
                profiles[index] = profile.clone();
            }
            None if profiles.len() >= MAX_PROFILES => return Err(format!("最多可以保存 {MAX_PROFILES} 个模型配置。")),
            None => {
                profile.verified = Verified::default();
                profiles.push(profile.clone());
            }
        }
        self.write(&profiles)?;
        Ok(profile)
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        let profiles: Vec<Profile> = self.list()?.into_iter().filter(|profile| profile.id != id).collect();
        self.write(&profiles)
    }

    pub fn record_verification(&self, id: &str, connection: bool, tools: bool) -> Result<(), String> {
        let mut profiles = self.list()?;
        if let Some(profile) = profiles.iter_mut().find(|profile| profile.id == id) {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0);
            profile.verified = Verified { connection, tools, at: Some(now) };
            self.write(&profiles)?;
        }
        Ok(())
    }
}

/// API keys: the system keychain when one is running, otherwise memory for this run only.
pub struct Vault {
    session: Mutex<HashMap<String, String>>,
}

const SERVICE: &str = "omniTerm model provider";

impl Vault {
    pub fn new() -> Self {
        Self { session: Mutex::default() }
    }

    fn entry(id: &str) -> Option<keyring::Entry> {
        keyring::Entry::new(SERVICE, id).ok()
    }

    /// Returns whether the key will survive a restart.
    pub fn set(&self, id: &str, secret: &str) -> Result<bool, String> {
        let secret = text(secret, "API Key", 8192)?;
        if let Some(entry) = Self::entry(id)
            && entry.set_password(&secret).is_ok()
        {
            self.session.lock().unwrap_or_else(|p| p.into_inner()).remove(id);
            return Ok(true);
        }
        self.session.lock().unwrap_or_else(|p| p.into_inner()).insert(id.to_string(), secret);
        Ok(false)
    }

    pub fn get(&self, id: &str) -> Option<String> {
        if let Some(secret) = self.session.lock().unwrap_or_else(|p| p.into_inner()).get(id) {
            return Some(secret.clone());
        }
        Self::entry(id)?.get_password().ok()
    }

    pub fn delete(&self, id: &str) {
        self.session.lock().unwrap_or_else(|p| p.into_inner()).remove(id);
        if let Some(entry) = Self::entry(id) {
            let _ = entry.delete_credential();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_urls_must_be_https_or_loopback_and_carry_no_secrets() {
        assert_eq!(validate_base_url("https://api.example.com/v1/").unwrap(), "https://api.example.com/v1");
        assert_eq!(validate_base_url("http://localhost:11434/v1").unwrap(), "http://localhost:11434/v1");
        assert!(validate_base_url("http://example.com/v1").is_err());
        assert!(validate_base_url("https://user:pass@example.com").is_err());
        assert!(validate_base_url("https://example.com/v1?key=1").is_err());
        assert!(validate_base_url("example.com").is_err());
    }

    #[test]
    fn profiles_follow_their_preset() {
        let profile = Profile { id: String::new(), preset_id: "anthropic".into(), name: "Claude".into(), auth: Auth::ApiKey, wire_api: WireApi::Chat, base_url: "https://api.anthropic.com/v1".into(), model: "claude".into(), chat_only: false, context_budget_bytes: 65536, verified: Verified::default() };
        assert!(profile.clone().validate().is_err(), "Anthropic speaks only its own protocol");
        let valid = Profile { wire_api: WireApi::AnthropicMessages, ..profile }.validate().unwrap();
        assert!(!valid.id.is_empty());
        let login = Profile { preset_id: "openai-login".into(), wire_api: WireApi::Responses, base_url: "ignored".into(), ..valid }.validate().unwrap();
        assert_eq!((login.auth, login.base_url.as_str()), (Auth::EngineLogin, ""));
    }
}

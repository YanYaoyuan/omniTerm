// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Opt-in OpenSSH askpass. Only the first recognisable account-password prompt
//! may use the OS keychain; OTPs, key passphrases and later prompts stay manual.
//! Secrets travel on askpass's private stdout pipe, never terminal input/history,
//! command arguments, environment variables or connection JSON.

use super::profile::Profile;
use serde_json::json;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const MODE: &str = "OMNITERM_SSH_ASKPASS";
const SCOPE: &str = "OMNITERM_SSH_AUTH_SCOPE";
const STATE: &str = "OMNITERM_SSH_AUTH_STATE";
const SERVICE: &str = "omniTerm SSH first password";

fn directories() -> &'static Mutex<Vec<PathBuf>> {
    static DIRECTORIES: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();
    DIRECTORIES.get_or_init(Mutex::default)
}

pub fn prepare(profile: &Profile) -> Result<BTreeMap<String, String>, String> {
    let root = std::env::temp_dir().join(format!("omniterm-auth-{}", uuid::Uuid::new_v4().simple()));
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700);
        builder
    };
    #[cfg(not(unix))]
    let builder = std::fs::DirBuilder::new();
    builder.create(&root).map_err(|_| "无法创建 SSH 认证状态目录。".to_string())?;
    directories().lock().unwrap_or_else(|p| p.into_inner()).push(root.clone());
    let executable = std::env::current_exe().map_err(|_| "找不到 SSH 密码助手。".to_string())?;
    // Include every configured hop/identity setting; passwords stay scoped to the target.
    let scope = scope(profile);
    Ok(BTreeMap::from([
        (MODE.into(), "1".into()),
        (SCOPE.into(), scope),
        (STATE.into(), root.join("first-password-used").to_string_lossy().into_owned()),
        ("SSH_ASKPASS".into(), executable.to_string_lossy().into_owned()),
        ("SSH_ASKPASS_REQUIRE".into(), "force".into()),
    ]))
}

pub fn cleanup() {
    for directory in directories().lock().unwrap_or_else(|p| p.into_inner()).drain(..) {
        let _ = std::fs::remove_dir_all(directory);
    }
}

/// Deliberately narrow: `2nd Password:`, OTPs and passphrases do not match.
fn account_password(prompt: &str) -> bool {
    let prompt = prompt.trim().to_ascii_lowercase();
    prompt == "password:" || prompt.ends_with(") password:") || prompt.ends_with("'s password:")
}

fn first_password(prompt: &str, state: &Path) -> bool {
    if !account_password(prompt) {
        return false;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(state).is_ok()
}

pub(crate) fn scope(profile: &Profile) -> String {
    json!([profile.host, profile.username, profile.port, profile.proxy_jump, profile.key_path]).to_string()
}

fn entry(scope: &str) -> Result<keyring::Entry, keyring::Error> {
    keyring::Entry::new(SERVICE, scope)
}

fn decode_saved(encoded: &str, prompt: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(encoded).ok()?;
    (value["prompt"].as_str()? == prompt.trim())
        .then(|| value["password"].as_str().map(String::from))
        .flatten()
}

pub(crate) fn saved(scope: &str, prompt: &str) -> Option<String> {
    decode_saved(&entry(scope).ok()?.get_password().ok()?, prompt).filter(|value| valid_secret(value))
}

pub(crate) fn save(scope: &str, prompt: &str, secret: &str) -> Result<(), keyring::Error> {
    entry(scope)?.set_password(&json!({ "prompt": prompt.trim(), "password": secret }).to_string())
}

pub fn forget(profile: &Profile) -> Result<(), String> {
    match entry(&scope(profile)).and_then(|entry| entry.delete_credential()) {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err("无法删除钥匙串里的 SSH 密码，请先解锁系统钥匙串。".into()),
    }
}

pub(crate) fn valid_secret(value: &str) -> bool {
    !value.is_empty() && value.len() <= 4096 && !value.contains(['\0', '\n', '\r'])
}

/// `read` is never called for an OTP or a second password attempt. Keeping this
/// decision separate makes that boundary testable without touching a real vault.
fn automatic_answer(prompt: &str, state: &Path, read: impl FnOnce() -> Option<String>) -> (bool, Option<String>) {
    let first = first_password(prompt, state);
    let answer = first.then(read).flatten().filter(|value| valid_secret(value));
    (first, answer)
}

/// OpenSSH invokes the current executable with one prompt argument. The marker
/// is scoped to opt-in SSH processes; normal application launches are unaffected.
pub fn helper(args: &[String], ask: impl FnOnce(&str, &str, bool, bool) -> Option<String>) -> Option<i32> {
    if std::env::var(MODE).as_deref() != Ok("1") || args.len() != 1 {
        return None;
    }
    let prompt = &args[0];
    if prompt.len() > 8192 {
        return Some(1);
    }
    let (Ok(scope), Ok(state)) = (std::env::var(SCOPE), std::env::var(STATE)) else {
        return Some(1);
    };
    let confirm = std::env::var("SSH_ASKPASS_PROMPT").as_deref() == Ok("confirm") || prompt.contains("yes/no");
    let (first, saved) = if confirm {
        (false, None)
    } else {
        automatic_answer(prompt, Path::new(&state), || decode_saved(&entry(&scope).ok()?.get_password().ok()?, prompt))
    };
    let answer = match saved {
        Some(value) => Some(value),
        None => ask(&scope, prompt, first, confirm),
    };
    Some(match answer {
        Some(value) if std::io::stdout().write_all(value.as_bytes()).is_ok() => 0,
        _ => 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_first_fixed_password_can_read_saved_credentials() {
        let root = std::env::temp_dir().join(format!("omniterm-auth-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let state = root.join("used");
        for prompt in [
            "2nd Password:",
            "Verification code:",
            "OTP:",
            "Enter passphrase for key 'id':",
            "Second password:",
        ] {
            assert_eq!(automatic_answer(prompt, &state, || panic!("OTP must not access vault")), (false, None));
        }
        assert!(!state.exists(), "OTP prompts must not consume the fixed-password slot");
        assert_eq!(
            automatic_answer("(alice@gateway) Password: ", &state, || Some("fixture-only".into())),
            (true, Some("fixture-only".into()))
        );
        assert_eq!(
            automatic_answer("(alice@gateway) Password: ", &state, || panic!("do not retry saved passwords")),
            (false, None)
        );
        assert_eq!(automatic_answer("2nd Password:", &state, || panic!("OTP must remain manual")), (false, None));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn credential_identity_includes_connection_and_exact_authentication_prompt() {
        let saved = json!({ "prompt": "(alice@a) Password:", "password": "fixture" }).to_string();
        assert_eq!(decode_saved(&saved, "(alice@a) Password: "), Some("fixture".into()));
        assert_eq!(decode_saved(&saved, "(bob@a) Password:"), None);
        let a = Profile {
            host: Some("a".into()),
            ..Profile::default()
        };
        let b = Profile {
            host: Some("b".into()),
            ..Profile::default()
        };
        assert_ne!(scope(&a), scope(&b));
        for prompt in ["Password:", "alice@host's password:", "(alice@host) Password:"] {
            assert!(account_password(prompt));
        }
        for value in ["", "bad\nvalue", "bad\0value", "bad\rvalue"] {
            assert!(!valid_secret(value));
        }
    }
}

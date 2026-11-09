// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Import and bind file connection references to stable SSH panes, never current focus.

use super::{App, Dialog};
use crate::core::profile::{FileConnection, Kind, Profile};
use crate::remote::qizhi::{self, Connection};

pub struct Import {
    pub pane: String,
    pub pane_title: String,
    pub profile: Profile,
    pub binding: FileConnection,
    pub error: Option<String>,
}

fn reference(source: std::path::PathBuf, connection: &Connection) -> FileConnection {
    FileConnection { source, host: connection.host.clone(), user: connection.user.clone(), port: connection.port, title: connection.title.clone() }
}

pub fn choose() -> Result<Option<FileConnection>, String> {
    let Some(source) = rfd::FileDialog::new().set_title("导入文件连接参数").add_filter("AccessClient JSON", &["json"]).pick_file() else { return Ok(None) };
    let connection = qizhi::load(&source)?;
    Ok(Some(reference(source, &connection)))
}

fn load(binding: &FileConnection) -> Result<Connection, String> {
    let connection = qizhi::load(&binding.source)?;
    if reference(binding.source.clone(), &connection) != *binding {
        return Err("文件连接参数的目标、账号或资产标题已变化。请重新导入并确认绑定的终端。".into());
    }
    Ok(connection)
}

pub fn same_target(a: &Profile, b: &Profile) -> bool {
    a.kind == b.kind && a.host == b.host && a.username == b.username && a.port == b.port && a.proxy_jump == b.proxy_jump && a.key_path == b.key_path
}

pub fn target_label(profile: &Profile) -> String {
    format!("{} · {}{}{}", profile.display_name(), profile.username.as_deref().map(|user| format!("{user}@")).unwrap_or_default(), profile.host.as_deref().unwrap_or("SSH"), profile.port.map(|port| format!(":{port}")).unwrap_or_default())
}

impl App {
    pub(super) fn import_file_connection(&mut self, id: &str) {
        // Capture the destination before opening the native picker.
        let Some(pane) = self.panes.get(id).filter(|pane| pane.session.kind == Kind::Ssh) else { return };
        let Some(profile) = pane.session.profile.clone() else { return };
        let pane_title = pane.label();
        match choose() {
            Ok(Some(binding)) => self.dialog = Some(Dialog::FileConnection(Import { pane: id.into(), pane_title, profile, binding, error: None })),
            Ok(None) => {}
            Err(error) => self.notify(error, true),
        }
    }

    pub(super) fn confirm_file_connection(&mut self) {
        let Some(Dialog::FileConnection(import)) = self.dialog.take() else { return };
        let result = self.bind_file_connection(&import);
        if let Err(error) = result {
            self.dialog = Some(Dialog::FileConnection(Import { error: Some(error), ..import }));
        }
    }

    fn bind_file_connection(&mut self, import: &Import) -> Result<(), String> {
        let pane = self.panes.get(&import.pane).ok_or("目标终端已关闭。请在要绑定的终端重新导入。")?;
        let mut profile = pane.session.profile.clone().ok_or("目标终端不再是 SSH 连接。")?;
        if !same_target(&profile, &import.profile) || profile.id != import.profile.id {
            return Err("目标终端已变化，请重新导入。".into());
        }
        let connection = load(&import.binding)?;
        profile.file_connection = Some(import.binding.clone());
        profile.file_connection_stale = false;
        if let Some(id) = &profile.id {
            let mut saved = self.store.list()?.into_iter().find(|saved| saved.id.as_ref() == Some(id)).ok_or("已保存的连接已删除，请重新导入到新的连接。")?;
            if !same_target(&saved, &profile) {
                return Err("已保存连接的 SSH 目标已变化。请重新打开该连接后导入。".into());
            }
            saved.file_connection = profile.file_connection.clone();
            saved.file_connection_stale = false;
            self.profiles = self.store.save(&saved)?;
        }
        self.panes.get_mut(&import.pane).expect("pane checked above").session.profile = Some(profile);
        self.file_panels.remove(&import.pane);
        self.file_connection_errors.remove(&import.pane);
        if let Err(error) = self.remotes.attach_qizhi(&import.pane, connection) {
            self.remotes.detach_file_only(&import.pane);
            self.file_connection_errors.insert(import.pane.clone(), error);
        }
        self.sidebar_files = true;
        self.sidebar_visible = true;
        self.notify(format!("文件连接已绑定到 {}。", import.pane_title), false);
        Ok(())
    }

    pub(super) fn reconnect_file_connection(&mut self, id: &str) {
        if self.panes.get(id).and_then(|pane| pane.session.profile.as_ref()).is_some_and(|profile| profile.file_connection_stale) {
            self.remotes.detach_file_only(id);
            self.file_connection_errors.insert(id.into(), "SSH 连接目标已更改。请重新导入文件连接参数，确认它属于当前资产。".into());
            return;
        }
        let binding = self.panes.get(id).and_then(|pane| pane.session.profile.as_ref()).and_then(|profile| profile.file_connection.clone());
        let Some(binding) = binding else { return };
        self.file_panels.remove(id);
        self.file_connection_errors.remove(id);
        let result = load(&binding).and_then(|connection| self.remotes.attach_qizhi(id, connection));
        if let Err(error) = result {
            // An old connection must not remain usable after a reference changes identity.
            self.remotes.detach_file_only(id);
            self.file_connection_errors.insert(id.into(), error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refreshed_export_cannot_silently_change_the_bound_asset() {
        let directory = std::env::temp_dir().join(format!("omniterm-file-binding-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let source = directory.join("connection.json");
        let export = |host: &str, user: &str, title: &str, pwd: &str| serde_json::json!({"app": "winscp", "host": host, "user": user, "port": 2222, "st": title, "pwd": pwd});
        std::fs::write(&source, export("192.0.2.1", "fixture", "asset one", "first").to_string()).unwrap();
        let binding = reference(source.clone(), &qizhi::load(&source).unwrap());
        std::fs::write(&source, export("192.0.2.1", "fixture", "asset one", "refreshed").to_string()).unwrap();
        assert!(load(&binding).is_ok(), "credential refresh preserves the selected asset");
        for changed in [export("192.0.2.2", "fixture", "asset one", "other"), export("192.0.2.1", "other-user", "asset one", "other"), export("192.0.2.1", "fixture", "asset two", "other")] {
            std::fs::write(&source, changed.to_string()).unwrap();
            assert!(load(&binding).is_err());
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}

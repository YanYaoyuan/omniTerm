// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! A web-authorized SFTP window reuses the existing file and transfer interface.

use super::{App, Intent, remote_panel};
use eframe::egui::{self, RichText};

pub struct Window {
    pub id: String,
    pub title: String,
}

pub fn show(app: &mut App, ui: &mut egui::Ui, intents: &mut Vec<Intent>) {
    let Some(window) = &app.web_files else { return };
    let id = window.id.clone();
    let title = window.title.clone();
    ui.add_space(12.0);
    ui.heading(title);
    ui.label(RichText::new("SFTP 文件会话 · 断线后请从堡垒机网页重新打开").size(11.0));
    ui.add_space(8.0);
    ui.separator();
    if let Some(remote) = app.remotes.get(&id) {
        remote_panel::file_panel(app, ui, &remote, intents);
    } else if let Some(error) = app.file_connection_errors.get(&id) {
        ui.label(error);
    }
}

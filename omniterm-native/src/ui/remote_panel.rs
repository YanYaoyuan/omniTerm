// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! The SFTP file panel and the host metrics bar of an SSH tab.

use super::style::{self, alpha, icon, icon_button};
use super::{App, Intent};
use crate::icons;
use crate::remote::manager::{DragFile, FileEntry, Phase, Remote, Snapshot, Transfer, TransferStatus, join, parent};
use crate::remote::metrics::Sample;
use eframe::egui::{self, Align, Key, Layout, RichText, Sense, Stroke, Ui, Vec2};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

/// What a panel remembers while the tab is open: path, selection and the pending question.
#[derive(Default)]
pub struct PanelState {
    draft: String,
    shown_path: Option<String>,
    selection: BTreeSet<String>,
    anchor: Option<String>,
    show_hidden: bool,
    editing: Option<Editing>,
    confirm: Option<Confirm>,
    uploads_seen: u64,
    started: bool,
    /// Dragged items already announced as ready, so each is announced once.
    drags_announced: BTreeSet<String>,
}

enum Editing {
    NewFolder(String),
    Rename { from: String, value: String },
}

enum Confirm {
    Delete(Vec<String>),
    OverwriteDownload { paths: Vec<String>, directory: PathBuf, conflicts: Vec<String> },
    OverwriteUpload { paths: Vec<PathBuf>, directory: String, conflicts: Vec<String> },
}

pub fn format_bytes(value: f64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    if !value.is_finite() || value < 0.0 {
        return "—".into();
    }
    let (mut size, mut unit) = (value, 0);
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 || size >= 100.0 { format!("{} {}", size.round(), UNITS[unit]) } else { format!("{size:.1} {}", UNITS[unit]) }
}

pub fn format_uptime(seconds: f64) -> String {
    let (days, hours, minutes) = ((seconds / 86400.0) as u64, (seconds / 3600.0) as u64 % 24, (seconds / 60.0) as u64 % 60);
    if days > 0 {
        format!("{days}天 {hours}时")
    } else if hours > 0 {
        format!("{hours}时 {minutes}分")
    } else {
        format!("{minutes}分")
    }
}

fn format_time(seconds: u32) -> String {
    if seconds == 0 {
        return String::new();
    }
    // Civil date from days since 1970 (Howard Hinnant's algorithm), in UTC.
    let days = seconds as i64 / 86400;
    let rest = seconds as i64 % 86400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year}-{month:02}-{day:02} {:02}:{:02}", rest / 3600, rest / 60 % 60)
}

/// Folders first, then names in a natural order, so file10 follows file9.
fn sorted(entries: &[FileEntry], show_hidden: bool) -> Vec<FileEntry> {
    let mut list: Vec<FileEntry> = entries.iter().filter(|entry| show_hidden || !entry.name.starts_with('.')).cloned().collect();
    let natural = |a: &str, b: &str| {
        let key = |text: &str| {
            let mut parts: Vec<(String, u128)> = Vec::new();
            let mut digits = String::new();
            let mut letters = String::new();
            for character in text.to_lowercase().chars() {
                if character.is_ascii_digit() {
                    if !letters.is_empty() {
                        parts.push((std::mem::take(&mut letters), u128::MAX));
                    }
                    digits.push(character);
                } else {
                    if !digits.is_empty() {
                        parts.push((String::new(), digits.parse().unwrap_or(0)));
                        digits.clear();
                    }
                    letters.push(character);
                }
            }
            if !digits.is_empty() {
                parts.push((String::new(), digits.parse().unwrap_or(0)));
            }
            if !letters.is_empty() {
                parts.push((letters, u128::MAX));
            }
            parts
        };
        key(a).cmp(&key(b))
    };
    list.sort_by(|a, b| b.directory.cmp(&a.directory).then_with(|| natural(&a.name, &b.name)));
    list
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/') && !name.contains('\0')
}

pub fn file_panel(app: &mut App, ui: &mut Ui, remote: &Arc<Remote>, intents: &mut Vec<Intent>) {
    let palette = style::palette(ui);
    let snapshot = remote.snapshot();
    let state = app.file_panels.entry(remote.id().to_string()).or_default();
    if snapshot.phase != Phase::Ready {
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            match &snapshot.phase {
                Phase::Unavailable { reason, detail } => {
                    ui.label(icon(icons::WARNING_CIRCLE, 22.0).color(palette.danger()));
                    ui.add(egui::Label::new(RichText::new(reason.message()).size(11.5)).wrap());
                    if !detail.is_empty() && *reason != crate::remote::manager::Reason::NoSftp {
                        ui.add(egui::Label::new(RichText::new(detail).monospace().size(9.5).color(palette.faint())).wrap());
                    }
                    if style::secondary_button(ui, "重试").clicked() {
                        remote.retry();
                    }
                }
                phase => {
                    ui.add(egui::Spinner::new().size(20.0));
                    ui.label(RichText::new(if *phase == Phase::Waiting { "等待 SSH 登录完成…" } else { "正在打开 SFTP…" }).size(11.5).color(palette.muted()));
                }
            }
        });
        state.started = false;
        return;
    }
    let current = snapshot.listing.path.clone().or_else(|| snapshot.home.clone()).unwrap_or_else(|| "/".into());
    if !state.started {
        state.started = true;
        remote.list(state.shown_path.clone().unwrap_or_else(|| current.clone()));
    }
    // Tell the user once when a dragged item can be dragged out.
    state.drags_announced.retain(|path| snapshot.drags.contains_key(path));
    for (path, drag) in &snapshot.drags {
        if matches!(drag, DragFile::Ready(_)) && state.drags_announced.insert(path.clone()) {
            intents.push(Intent::Notify(format!("{} 已准备好，再次拖动即可拖到本地文件夹。", path.rsplit('/').next().unwrap_or(path)), false));
        }
    }
    if snapshot.uploads_done != state.uploads_seen {
        state.uploads_seen = snapshot.uploads_done;
        remote.list(current.clone());
    }
    if state.shown_path.as_deref() != snapshot.listing.path.as_deref() {
        state.shown_path = snapshot.listing.path.clone();
        state.draft = current.clone();
        state.selection.clear();
    }

    // Toolbar: up, home, refresh | upload, new folder, hidden files.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        if icon_button(ui, icons::ARROW_LEFT, "上一级", current != "/").clicked() {
            remote.list(parent(&current));
        }
        if icon_button(ui, icons::HOUSE, "主目录", true).clicked()
            && let Some(home) = &snapshot.home
        {
            remote.list(home.clone());
        }
        if icon_button(ui, icons::ARROW_CLOCKWISE, "刷新", true).clicked() {
            remote.list(current.clone());
        }
        ui.add_space(6.0);
        if icon_button(ui, icons::UPLOAD_SIMPLE, "上传文件", true).clicked()
            && let Some(paths) = rfd::FileDialog::new().set_title("选择要上传的文件").pick_files()
        {
            upload(remote, state, paths, current.clone(), intents);
        }
        if icon_button(ui, icons::FOLDER_PLUS, "新建文件夹", true).clicked() {
            state.editing = Some(Editing::NewFolder(String::new()));
        }
        let hidden = if state.show_hidden { icons::EYE } else { icons::EYE_SLASH };
        if icon_button(ui, hidden, if state.show_hidden { "隐藏点文件" } else { "显示点文件" }, true).clicked() {
            state.show_hidden = !state.show_hidden;
        }
        if snapshot.listing.loading {
            ui.add(egui::Spinner::new().size(12.0));
        }
    });
    let path = ui.add(egui::TextEdit::singleline(&mut state.draft).desired_width(f32::INFINITY).font(egui::FontId::monospace(11.0)));
    if path.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) && !state.draft.trim().is_empty() {
        remote.list(state.draft.trim().to_string());
    }
    if let Some(error) = &snapshot.listing.error {
        ui.horizontal_wrapped(|ui| {
            ui.label(icon(icons::WARNING_CIRCLE, 14.0).color(palette.danger()));
            ui.label(RichText::new(error).size(10.5).color(palette.danger()));
        });
    }

    let entries = sorted(&snapshot.listing.entries, state.show_hidden);
    let names: Vec<String> = entries.iter().map(|entry| entry.name.clone()).collect();
    let list_height = (ui.available_height() - transfers_height(&snapshot, ui.available_height()) - ui.spacing().item_spacing.y).max(0.0);
    egui::ScrollArea::vertical().id_salt("remote-files").max_height(list_height).min_scrolled_height(0.0).auto_shrink([false, false]).show(ui, |ui| {
        if let Some(Editing::NewFolder(value)) = &mut state.editing {
            let (commit, cancel) = name_input(ui, value, "文件夹名称");
            if commit {
                let name = value.trim().to_string();
                state.editing = None;
                if valid_name(&name) {
                    let target = join(&current, &name);
                    let reload = (remote.clone(), current.clone());
                    remote.operate(move |client| client.mkdir(&target), move |result| finish(result, &reload));
                } else {
                    intents.push(Intent::Notify("名称不能为空，也不能包含 /。".into(), true));
                }
            } else if cancel {
                state.editing = None;
            }
        }
        for entry in &entries {
            if let Some(Editing::Rename { from, value }) = &mut state.editing
                && *from == entry.name
            {
                let (commit, cancel) = name_input(ui, value, "名称");
                if commit {
                    let name = value.trim().to_string();
                    let from = join(&current, from);
                    state.editing = None;
                    if valid_name(&name) {
                        let to = join(&current, &name);
                        let reload = (remote.clone(), current.clone());
                        remote.operate(move |client| client.rename(&from, &to), move |result| finish(result, &reload));
                    } else {
                        intents.push(Intent::Notify("名称不能为空，也不能包含 /。".into(), true));
                    }
                } else if cancel {
                    state.editing = None;
                }
                continue;
            }
            let selected = state.selection.contains(&entry.name);
            let sense = if crate::drag_out::SUPPORTED { Sense::click_and_drag() } else { Sense::click() };
            let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), sense);
            let path = join(&current, &entry.name);
            let drag = snapshot.drags.get(&path);
            if selected {
                ui.painter().rect_filled(rect, 4.0, alpha(palette.tokens.accent, 0.16));
            } else if response.hovered() {
                ui.painter().rect_filled(rect, 4.0, alpha(palette.tokens.ink, 0.04));
            }
            let glyph = if entry.directory { icons::FOLDER } else { icons::FILE };
            let tint = if entry.directory { palette.accent() } else { palette.muted() };
            ui.painter().text(rect.left_center() + Vec2::new(8.0, 0.0), egui::Align2::LEFT_CENTER, glyph, egui::FontId::new(15.0, egui::FontFamily::Name(style::ICONS.into())), tint);
            let size = if entry.directory { String::new() } else { format_bytes(entry.size as f64) };
            let size_width = if size.is_empty() { 0.0 } else { 60.0 };
            let mut job = egui::text::LayoutJob::simple_singleline(entry.name.clone(), egui::FontId::proportional(11.5), palette.text_soft());
            job.wrap = egui::text::TextWrapping::truncate_at_width((rect.width() - 36.0 - size_width).max(20.0));
            let galley = ui.painter().layout_job(job);
            ui.painter().galley(rect.left_center() + Vec2::new(30.0, -galley.size().y / 2.0), galley, palette.text_soft());
            if entry.symlink {
                ui.painter().text(rect.left_center() + Vec2::new(18.0, 5.0), egui::Align2::LEFT_CENTER, icons::LINK, egui::FontId::new(9.0, egui::FontFamily::Name(style::ICONS.into())), palette.faint());
            }
            match drag {
                Some(DragFile::Preparing) => {
                    ui.painter().text(rect.right_center() - Vec2::new(8.0, 0.0), egui::Align2::RIGHT_CENTER, icons::CIRCLE_NOTCH, egui::FontId::new(12.0, egui::FontFamily::Name(style::ICONS.into())), palette.muted());
                }
                Some(DragFile::Ready(_)) => {
                    ui.painter().text(rect.right_center() - Vec2::new(6.0, 0.0), egui::Align2::RIGHT_CENTER, "可拖出", egui::FontId::proportional(10.0), palette.accent());
                }
                None => {
                    ui.painter().text(rect.right_center() - Vec2::new(6.0, 0.0), egui::Align2::RIGHT_CENTER, size, egui::FontId::proportional(10.0), palette.faint());
                }
            }
            // The OS drags only finished local files: the first drag fetches the item, the
            // next one from the same row hands it to the system.
            if response.drag_started() {
                match drag {
                    Some(DragFile::Ready(file)) if file.exists() => intents.push(Intent::DragOut(file.clone())),
                    Some(DragFile::Preparing) => {}
                    _ => match remote.prepare_drag(path.clone()) {
                        Ok(()) => intents.push(Intent::Notify(format!("正在准备 {}，完成后再拖动一次即可拖出。", entry.name), false)),
                        Err(error) => intents.push(Intent::Notify(error, true)),
                    },
                }
            }
            let response = response.on_hover_text(format!("{}\n{} · {} · {:03o}", entry.name, if entry.directory { "文件夹".into() } else { format_bytes(entry.size as f64) }, format_time(entry.mtime), entry.mode & 0o777));
            if response.double_clicked() {
                if entry.directory {
                    remote.list(join(&current, &entry.name));
                } else {
                    download(app_downloads(), remote, state, vec![join(&current, &entry.name)], intents);
                }
            } else if response.clicked() {
                let modifiers = ui.input(|input| input.modifiers);
                select(state, &names, &entry.name, modifiers.command, modifiers.shift);
            }
            response.context_menu(|ui| {
                if !state.selection.contains(&entry.name) {
                    state.selection = [entry.name.clone()].into();
                    state.anchor = Some(entry.name.clone());
                }
                let chosen: Vec<String> = names.iter().filter(|name| state.selection.contains(*name)).map(|name| join(&current, name)).collect();
                if entry.directory && ui.button(format!("{}  打开", icons::FOLDER_OPEN)).clicked() {
                    remote.list(join(&current, &entry.name));
                }
                if ui.button(format!("{}  下载…", icons::DOWNLOAD_SIMPLE)).clicked() {
                    download(app_downloads(), remote, state, chosen.clone(), intents);
                }
                if ui.button(format!("{}  重命名", icons::PENCIL_SIMPLE)).clicked() {
                    state.editing = Some(Editing::Rename { from: entry.name.clone(), value: entry.name.clone() });
                }
                if ui.button(format!("{}  复制路径", icons::COPY)).clicked() {
                    ui.ctx().copy_text(chosen.join("\n"));
                }
                ui.separator();
                if ui.button(format!("{}  删除", icons::TRASH)).clicked() {
                    state.confirm = Some(Confirm::Delete(chosen));
                }
            });
        }
        if entries.is_empty() && !snapshot.listing.loading {
            ui.vertical_centered(|ui| {
                ui.add_space(20.0);
                ui.label(RichText::new("空文件夹").size(11.0).color(palette.faint()));
            });
        }
    });

    // Files dropped anywhere on the panel go up into the folder it shows.
    let panel = ui.min_rect();
    let dropped: Vec<PathBuf> = ui.input(|input| input.raw.dropped_files.iter().map(|file| file.path().to_path_buf()).filter(|path| !path.as_os_str().is_empty()).collect());
    if !dropped.is_empty() && ui.input(|input| input.pointer.hover_pos()).is_some_and(|pointer| panel.contains(pointer)) {
        upload(remote, state, dropped, current.clone(), intents);
    }
    transfers(ui, remote, &snapshot);
    confirm(ui, remote, state, &current);
}

const TRANSFER_ROW_HEIGHT: f32 = 58.0;

fn transfers_height(snapshot: &Snapshot, available: f32) -> f32 {
    if snapshot.transfers.is_empty() {
        return 0.0;
    }
    // In a short dock, give one progress row space before allocating the file list.
    let desired = 34.0 + TRANSFER_ROW_HEIGHT * snapshot.transfers.len().min(3) as f32;
    desired.min((available * 0.6).max(34.0 + TRANSFER_ROW_HEIGHT)).min(available)
}

fn select(state: &mut PanelState, names: &[String], name: &str, toggle: bool, range: bool) {
    if range && let Some(anchor) = state.anchor.as_ref().and_then(|anchor| names.iter().position(|n| n == anchor)) {
        let at = names.iter().position(|n| n == name).unwrap_or(anchor);
        let (from, to) = (anchor.min(at), anchor.max(at));
        state.selection = names[from..=to].iter().cloned().collect();
        return;
    }
    if toggle {
        if !state.selection.remove(name) {
            state.selection.insert(name.to_string());
        }
    } else {
        state.selection = [name.to_string()].into();
    }
    state.anchor = Some(name.to_string());
}

fn name_input(ui: &mut Ui, value: &mut String, hint: &str) -> (bool, bool) {
    let response = ui.add(egui::TextEdit::singleline(value).hint_text(hint).char_limit(255).desired_width(f32::INFINITY));
    response.request_focus();
    let commit = response.lost_focus() && !ui.input(|input| input.key_pressed(Key::Escape));
    let cancel = ui.input(|input| input.key_pressed(Key::Escape));
    (commit, cancel)
}

fn finish(result: Result<(), String>, (remote, directory): &(Arc<Remote>, String)) {
    if result.is_ok() {
        remote.list(directory.clone());
    }
}

/// Where downloads go: the folder the user chose last, asked again only when it is gone.
fn app_downloads() -> Option<PathBuf> {
    rfd::FileDialog::new().set_title("选择下载位置").pick_folder()
}

/// Conflicts are asked about before anything is queued, so the user decides about replacing data.
fn download(destination: Option<PathBuf>, remote: &Arc<Remote>, state: &mut PanelState, paths: Vec<String>, intents: &mut Vec<Intent>) {
    let Some(directory) = destination else { return };
    let conflicts: Vec<String> = paths.iter().filter_map(|path| path.rsplit('/').next()).filter(|name| directory.join(name).symlink_metadata().is_ok()).map(String::from).collect();
    if !conflicts.is_empty() {
        state.confirm = Some(Confirm::OverwriteDownload { paths, directory, conflicts });
        return;
    }
    if let Err(error) = remote.download(paths, directory) {
        intents.push(Intent::Notify(error, true));
    }
}

fn upload(remote: &Arc<Remote>, state: &mut PanelState, paths: Vec<PathBuf>, directory: String, intents: &mut Vec<Intent>) {
    match remote.upload_conflicts(&paths, &directory) {
        Ok(conflicts) if conflicts.is_empty() => remote.upload(paths, directory),
        Ok(conflicts) => state.confirm = Some(Confirm::OverwriteUpload { paths, directory, conflicts }),
        Err(error) => intents.push(Intent::Notify(error, true)),
    }
}

fn confirm(ui: &mut Ui, remote: &Arc<Remote>, state: &mut PanelState, current: &str) {
    let palette = style::palette(ui);
    let Some(question) = &state.confirm else { return };
    let (title, detail, action) = match question {
        Confirm::Delete(paths) => (
            format!("删除 {} 个远程项目？", paths.len()),
            format!("{}\n此操作无法撤销，文件夹会连同其内容一起删除。", paths.iter().map(|path| path.rsplit('/').next().unwrap_or(path)).collect::<Vec<_>>().join("、")),
            "删除",
        ),
        Confirm::OverwriteUpload { directory, conflicts, .. } => ("覆盖远程文件？".into(), format!("{directory} 中已存在：{}", conflicts.join("、")), "覆盖"),
        Confirm::OverwriteDownload { directory, conflicts, .. } => ("覆盖本地文件？".into(), format!("{} 中已存在：{}", directory.display(), conflicts.join("、")), "覆盖"),
    };
    let mut decision = None;
    egui::Modal::new(egui::Id::unique("sftp-confirm")).frame(egui::Frame::new().fill(palette.surface()).corner_radius(12).inner_margin(20).stroke(Stroke::new(1.0, palette.border_strong))).show(ui.ctx(), |ui| {
        ui.set_width(380.0);
        ui.horizontal(|ui| {
            ui.label(icon(icons::WARNING_CIRCLE, 22.0).color(palette.danger()));
            ui.label(RichText::new(&title).size(15.0).strong().color(palette.text()));
        });
        ui.add_space(8.0);
        ui.add(egui::Label::new(RichText::new(&detail).size(11.5)).wrap());
        ui.add_space(14.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if style::danger_button(ui, action).clicked() {
                decision = Some(true);
            }
            if style::ghost_button(ui, "取消").clicked() {
                decision = Some(false);
            }
        });
    });
    match decision {
        Some(true) => match state.confirm.take() {
            Some(Confirm::Delete(paths)) => {
                let reload = (remote.clone(), current.to_string());
                remote.operate(move |client| paths.iter().try_for_each(|path| remove_tree(client, path, 0)), move |result| finish(result, &reload));
            }
            Some(Confirm::OverwriteUpload { paths, directory, .. }) => remote.upload(paths, directory),
            Some(Confirm::OverwriteDownload { paths, directory, .. }) => {
                let _ = remote.download(paths, directory);
            }
            None => {}
        },
        Some(false) => state.confirm = None,
        None => {}
    }
}

/// `lstat` first, so a link to a directory removes the link and never what it points at.
fn remove_tree(client: &crate::remote::sftp::Client, path: &str, depth: usize) -> crate::remote::sftp::Result<()> {
    if path.trim_end_matches('/').is_empty() {
        return Err(crate::remote::sftp::Error::Protocol("不能删除远程根目录。".into()));
    }
    let attributes = client.lstat(path)?;
    if !attributes.directory() {
        return client.remove(path);
    }
    if depth >= 64 {
        return Err(crate::remote::sftp::Error::Protocol("远程目录嵌套太深，无法删除。".into()));
    }
    for entry in client.readdir(path)? {
        remove_tree(client, &join(path, &entry.name), depth + 1)?;
    }
    client.rmdir(path)
}

fn transfers(ui: &mut Ui, remote: &Arc<Remote>, snapshot: &Snapshot) {
    if snapshot.transfers.is_empty() {
        return;
    }
    let palette = style::palette(ui);
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(RichText::new("传输").size(10.5).color(palette.muted()));
        if snapshot.transfers.iter().any(|transfer| transfer.finished()) {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(egui::Button::new(RichText::new("清除已完成").size(10.0)).frame(false)).clicked() {
                    remote.clear_finished();
                }
            });
        }
    });
    if let Some(id) = transfer_rows(ui, &snapshot.transfers) {
        remote.cancel(id);
    }
}

/// Keep active work above history, and reveal newly added transfers even if the user
/// previously scrolled through completed items. Ordinary progress updates keep the scroll.
fn transfer_rows(ui: &mut Ui, transfers: &[Transfer]) -> Option<u64> {
    let latest = transfers.last().map(|transfer| transfer.id);
    let memory = ui.scope_id().with("latest-transfer");
    let previous = ui.ctx().data_mut(|data| data.get_temp::<Option<u64>>(memory));
    ui.ctx().data_mut(|data| data.insert_temp(memory, latest));
    let mut scroll = egui::ScrollArea::vertical().id_salt("transfers").max_height(ui.available_height().max(0.0)).min_scrolled_height(0.0).auto_shrink([false, false]);
    if previous != Some(latest) {
        scroll = scroll.vertical_scroll_offset(0.0);
    }
    let mut cancel = None;
    scroll.show(ui, |ui| {
        let running = transfers.iter().filter(|transfer| transfer.status == TransferStatus::Running);
        let queued = transfers.iter().filter(|transfer| transfer.status == TransferStatus::Queued);
        let history = transfers.iter().rev().filter(|transfer| transfer.finished());
        for transfer in running.chain(queued).chain(history) {
            if transfer_row(ui, transfer) {
                cancel = Some(transfer.id);
            }
        }
    });
    cancel
}

fn transfer_row(ui: &mut Ui, transfer: &Transfer) -> bool {
    let palette = style::palette(ui);
    let fraction = if transfer.status == TransferStatus::Done { 1.0 } else if transfer.total > 0 { (transfer.bytes as f64 / transfer.total as f64).clamp(0.0, 1.0) as f32 } else { 0.0 };
    let status = match transfer.status {
        TransferStatus::Queued => "排队中".to_string(),
        TransferStatus::Done if transfer.skipped > 0 => format!("完成，跳过 {} 项", transfer.skipped),
        TransferStatus::Done => "完成 100%".into(),
        TransferStatus::Canceled => "已取消".into(),
        TransferStatus::Failed => "失败".into(),
        TransferStatus::Running if transfer.total > 0 => format!("{:.0}%", fraction * 100.0),
        TransferStatus::Running => "准备中".into(),
    };
    let tint = if transfer.status == TransferStatus::Failed { palette.danger() } else if transfer.status == TransferStatus::Canceled { palette.muted() } else { palette.accent() };
    let width = ui.available_width().max(0.0);
    let (row, response) = ui.allocate_exact_size(Vec2::new(width, TRANSFER_ROW_HEIGHT), Sense::hover());
    response.on_hover_text(if transfer.error.is_empty() { &transfer.name } else { &transfer.error });
    let mut canceled = false;
    ui.scope_builder(egui::UiBuilder::new().max_rect(row), |ui| {
        ui.set_clip_rect(ui.clip_rect().intersect(row));
        let mut right = row.right();
        if !transfer.finished() {
            let cancel = egui::Rect::from_min_size(egui::pos2(right - 22.0, row.top()), Vec2::splat(22.0));
            canceled = ui.put(cancel, egui::Button::new(icon(icons::X, 12.0)).frame(false)).on_hover_text("取消传输").clicked();
            right = cancel.left() - 4.0;
        }
        let mut job = egui::text::LayoutJob::simple_singleline(status, egui::FontId::proportional(10.0), tint);
        job.wrap = egui::text::TextWrapping::truncate_at_width((right - row.left()).max(0.0) * 0.55);
        let status = ui.painter().layout_job(job);
        right -= status.size().x;
        ui.painter().galley(egui::pos2(right, row.top() + 5.0), status, tint);
        ui.painter().text(row.min + Vec2::new(0.0, 11.0), egui::Align2::LEFT_CENTER, if transfer.upload { icons::ARROW_UP } else { icons::ARROW_DOWN }, egui::FontId::new(13.0, egui::FontFamily::Name(style::ICONS.into())), palette.muted());
        let name_left = row.left() + 19.0;
        let mut job = egui::text::LayoutJob::simple_singleline(transfer.name.clone(), egui::FontId::proportional(10.5), palette.text_soft());
        job.wrap = egui::text::TextWrapping::truncate_at_width((right - 8.0 - name_left).max(0.0));
        let name = ui.painter().layout_job(job);
        ui.painter().galley(egui::pos2(name_left, row.top() + 5.0), name, palette.text_soft());
        let bar = egui::Rect::from_min_size(row.min + Vec2::new(0.0, 25.0), Vec2::new(width, 8.0));
        ui.put(bar, egui::ProgressBar::new(fraction).desired_width(width).desired_height(8.0).fill(tint).animate(transfer.status == TransferStatus::Running && transfer.total == 0));
        let bytes = if transfer.total > 0 { format!("{} / {}", format_bytes(transfer.bytes as f64), format_bytes(transfer.total as f64)) } else { format_bytes(transfer.bytes as f64) };
        ui.put(egui::Rect::from_min_size(row.min + Vec2::new(0.0, 37.0), Vec2::new(width, 16.0)), egui::Label::new(RichText::new(bytes).size(10.0).color(palette.muted())).truncate());
    });
    canceled
}

/// The host bar under an SSH terminal: CPU, memory, swap, disk, load, network, uptime, GPUs.
pub fn metrics_bar(ui: &mut Ui, rect: egui::Rect, snapshot: &Snapshot) {
    let palette = style::palette(ui);
    ui.painter().rect_filled(rect, 0.0, palette.panel);
    ui.painter().hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, palette.border));
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink2(Vec2::new(14.0, 0.0))).layout(Layout::left_to_right(Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        let Some(sample) = snapshot.metrics.as_ref().filter(|_| snapshot.metrics_available) else {
            ui.label(RichText::new(if snapshot.phase == Phase::Waiting { "等待 SSH 登录完成…" } else { "主机指标不可用" }).size(11.0).color(palette.muted()));
            return;
        };
        metric_items(ui, sample);
    });
}

fn metric_items(ui: &mut Ui, sample: &Sample) {
    let palette = style::palette(ui);
    let item = |ui: &mut Ui, label: &str, value: String, fraction: Option<f64>| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            ui.label(RichText::new(label).size(10.5).color(palette.faint()));
            if let Some(fraction) = fraction {
                let (bar, _) = ui.allocate_exact_size(Vec2::new(34.0, 4.0), Sense::hover());
                ui.painter().rect_filled(bar, 2.0, alpha(palette.tokens.ink, 0.1));
                let fill = egui::Rect::from_min_size(bar.min, Vec2::new(bar.width() * fraction.clamp(0.0, 1.0) as f32, bar.height()));
                let color = if fraction > 0.9 { palette.danger() } else if fraction > 0.75 { style::color(palette.tokens.warning) } else { palette.accent() };
                ui.painter().rect_filled(fill, 2.0, color);
            }
            ui.label(RichText::new(value).size(10.5).monospace().color(palette.text_soft()));
        });
    };
    if let Some(cpu) = sample.cpu {
        let cores = sample.cores.map(|cores| format!(" · {cores} 核")).unwrap_or_default();
        item(ui, "CPU", format!("{cpu:.0}%{cores}"), Some(cpu / 100.0));
    }
    let usage = |usage: crate::remote::metrics::Usage| (format!("{} / {}", format_bytes(usage.used), format_bytes(usage.total)), Some(usage.used / usage.total.max(1.0)));
    if let Some(mem) = sample.mem {
        let (text, fraction) = usage(mem);
        item(ui, "内存", text, fraction);
    }
    if let Some(swap) = sample.swap {
        let (text, fraction) = usage(swap);
        item(ui, "交换", text, fraction);
    }
    if let Some(disk) = sample.disk {
        let (text, fraction) = usage(disk);
        item(ui, "磁盘", text, fraction);
    }
    if let Some([one, five, fifteen]) = sample.load {
        item(ui, "负载", format!("{one:.2} {five:.2} {fifteen:.2}"), None);
    }
    if let Some((rx, tx)) = sample.net {
        item(ui, "网络", format!("↓{}/s ↑{}/s", format_bytes(rx), format_bytes(tx)), None);
    }
    if let Some(uptime) = sample.uptime {
        item(ui, "运行", format_uptime(uptime), None);
    }
    for gpu in &sample.gpus {
        let memory = gpu.mem.map(|mem| format!(" · {}", format_bytes(mem.used))).unwrap_or_default();
        let temp = gpu.temp.map(|temp| format!(" · {temp:.0}°C")).unwrap_or_default();
        item(ui, &gpu.name, format!("{:.0}%{memory}{temp}", gpu.util), Some(gpu.util / 100.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transfer(id: u64, upload: bool, status: TransferStatus) -> Transfer {
        let mut transfer = Transfer::queued(id, upload, format!("{}-{id}.bin", if upload { "upload" } else { "download" }));
        transfer.status = status;
        transfer.total = 4 * 1024 * 1024;
        transfer.bytes = 1024 * 1024;
        transfer
    }

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        style::install_fonts(&ctx);
        ctx
    }

    fn visible_bars(output: &egui::FullOutput) -> Vec<&egui::epaint::RectShape> {
        output.shapes.iter().filter_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect) if (rect.rect.height() - 8.0).abs() < 0.01 && shape.clip_rect.intersects(rect.rect) => Some(rect),
            _ => None,
        }).collect()
    }

    #[test]
    fn upload_and_download_bars_remain_visible_above_completed_history() {
        let mut transfers: Vec<Transfer> = (1..=20).map(|id| transfer(id, true, TransferStatus::Done)).collect();
        transfers.push(transfer(21, true, TransferStatus::Running));
        transfers.push(transfer(22, false, TransferStatus::Running));
        let ctx = context();
        for _ in 0..2 {
            let output = ctx.run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(248.0, 135.0))), ..Default::default() }, |ui| {
                transfer_rows(ui, &transfers);
            });
            let names: Vec<&str> = output.shapes.iter().filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if shape.clip_rect.intersects(text.visual_bounding_rect()) && text.galley.job.text.ends_with(".bin") => Some(text.galley.job.text.as_str()),
                _ => None,
            }).collect();
            assert!(names.starts_with(&["upload-21.bin", "download-22.bin"]), "active transfers are hidden below history: {names:?}");
            let bars = visible_bars(&output);
            assert_eq!(bars.len(), 4, "each visible task has a track and a filled part");
            for pair in bars.chunks_exact(2) {
                assert!((pair[1].rect.width() / pair[0].rect.width() - 0.25).abs() < 0.01);
            }
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn completed_empty_files_keep_a_full_bar_and_long_names_fit_narrow_panels() {
        for width in [160.0, 248.0, 400.0] {
            let ctx = context();
            let mut item = transfer(1, false, TransferStatus::Done);
            item.name = "long-file-name-".repeat(30);
            item.bytes = 0;
            item.total = 0;
            let output = ctx.run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(width, 100.0))), ..Default::default() }, |ui| {
                transfer_rows(ui, &[item.clone()]);
            });
            let bars = visible_bars(&output);
            assert_eq!(bars.len(), 2);
            assert_eq!(bars[0].rect, bars[1].rect);
            for shape in &output.shapes {
                if let egui::Shape::Text(text) = &shape.shape {
                    assert!(text.pos.x + text.galley.size().x <= width, "text extends past the panel");
                    if text.galley.job.text == item.name {
                        assert!(text.pos.x + text.galley.size().x < width - 45.0, "filename obscures completion status");
                    }
                }
            }
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn cancel_button_targets_the_visible_active_transfer() {
        let ctx = context();
        let transfers = [transfer(1, false, TransferStatus::Done), transfer(2, true, TransferStatus::Running)];
        let mut position = egui::Pos2::ZERO;
        let mut canceled = None;
        for step in 0..3 {
            let events = if step == 0 { Vec::new() } else { vec![egui::Event::PointerMoved(position), egui::Event::PointerButton { pos: position, button: egui::PointerButton::Primary, pressed: step == 1, modifiers: egui::Modifiers::default() }] };
            ctx.run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(248.0, 150.0))), events, ..Default::default() }, |ui| {
                position = ui.available_rect_before_wrap().right_top() + Vec2::new(-11.0, 11.0);
                canceled = transfer_rows(ui, &transfers).or(canceled);
            }).drop_without_applying_deltas();
        }
        assert_eq!(canceled, Some(2));
    }

    #[test]
    fn declining_download_overwrite_preserves_file_and_enqueues_nothing() {
        let directory = std::env::temp_dir().join(format!("omniterm-overwrite-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir(&directory).unwrap();
        let file = directory.join("报告 #1%.txt");
        std::fs::write(&file, b"original local content").unwrap();
        let manager = crate::remote::manager::Manager::new(|| {});
        manager.attach("confirm", crate::core::plan::Companion { executable: "ssh".into(), destination: "unused".into(), control_path: Some(directory.join("not-connected").to_string_lossy().into_owned()), args: Vec::new() });
        let remote = manager.get("confirm").unwrap();
        let mut state = PanelState::default();
        download(Some(directory.clone()), &remote, &mut state, vec!["/remote/报告 #1%.txt".into()], &mut Vec::new());
        assert!(matches!(state.confirm, Some(Confirm::OverwriteDownload { .. })));
        assert!(remote.snapshot().transfers.is_empty());
        let ctx = context();
        let mut position = egui::Pos2::ZERO;
        for step in 0..4 {
            let events = if step < 2 { Vec::new() } else { vec![egui::Event::PointerMoved(position), egui::Event::PointerButton { pos: position, button: egui::PointerButton::Primary, pressed: step == 2, modifiers: egui::Modifiers::default() }] };
            let output = ctx.run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 600.0))), events, ..Default::default() }, |ui| confirm(ui, &remote, &mut state, "/remote"));
            if step == 1 {
                position = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text == "取消" => Some(text.visual_bounding_rect().center()),
                    _ => None,
                }).expect("confirmation must offer a Cancel button");
            }
            output.drop_without_applying_deltas();
        }
        assert!(state.confirm.is_none());
        assert!(remote.snapshot().transfers.is_empty());
        assert_eq!(std::fs::read(&file).unwrap(), b"original local content");
        manager.shutdown();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn sizes_times_and_order_read_like_the_desktop_app() {
        assert_eq!(format_bytes(512.0), "512 B");
        assert_eq!(format_bytes(1536.0), "1.5 KiB");
        assert_eq!(format_bytes(200.0 * 1024.0 * 1024.0), "200 MiB");
        assert_eq!(format_uptime(90_000.0), "1天 1时");
        assert_eq!(format_time(1_700_000_000), "2023-11-14 22:13");
        let entry = |name: &str, directory| FileEntry { name: name.into(), directory, symlink: false, size: 0, mtime: 0, mode: 0 };
        let order: Vec<String> = sorted(&[entry("file10", false), entry("file9", false), entry("zdir", true), entry(".hidden", false)], false).into_iter().map(|e| e.name).collect();
        assert_eq!(order, ["zdir", "file9", "file10"]);
    }

    #[test]
    fn clicks_select_toggle_and_extend() {
        let names: Vec<String> = ["a", "b", "c", "d"].map(String::from).to_vec();
        let mut state = PanelState::default();
        select(&mut state, &names, "b", false, false);
        select(&mut state, &names, "d", false, true);
        assert_eq!(state.selection.len(), 3);
        select(&mut state, &names, "c", true, false);
        assert!(!state.selection.contains("c"));
    }
}

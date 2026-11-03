// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! The workspace sidebar: the working folder, local terminals, saved connections and the
//! connection tools. Its width is kept in pixels, so the terminals take every window resize.

use super::dialogs::ConnectionForm;
use super::style::{self, alpha, icon, icon_button};
use super::{App, Dialog, Intent, shortcuts::Action};
use crate::core::profile::{Kind, Profile};
use crate::core::text::{basename, path_label};
use crate::icons;
use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Stroke, Ui, Vec2};

pub fn kind_icon(kind: Kind) -> &'static str {
    match kind {
        Kind::Shell => icons::TERMINAL_WINDOW,
        Kind::Agent => icons::ROBOT,
        Kind::Ssh => icons::HARD_DRIVES,
        Kind::Sftp => icons::FOLDER_OPEN,
        Kind::Serial => icons::PLUGS_CONNECTED,
        Kind::Tunnel => icons::ARROWS_LEFT_RIGHT,
    }
}

pub fn connection_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Ssh => "SSH",
        Kind::Sftp => "SFTP",
        Kind::Serial => "串口",
        Kind::Tunnel => "端口转发",
        Kind::Shell => "本地终端",
        Kind::Agent => "Agent",
    }
}

pub fn show(app: &mut App, ui: &mut Ui, intents: &mut Vec<Intent>) {
    let palette = style::palette(ui);
    let response = egui::Panel::left("sidebar")
        .resizable(true)
        .default_size(app.sidebar_width)
        .size_range(180.0..=520.0)
        .frame(egui::Frame::new().fill(palette.panel).inner_margin(egui::Margin { left: 12, right: 12, top: 17, bottom: 0 }))
        .show(ui, |ui| {
            let rect = ui.max_rect().expand2(Vec2::new(12.0, 0.0));
            ui.painter().vline(rect.right() - 0.5, rect.y_range(), Stroke::new(1.0, palette.border_strong));
            // An SSH tab adds its files beside the workspace view.
            if let Some(remote) = app.focused_remote() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    let width = (ui.available_width() - 3.0) / 2.0;
                    for (files, glyph, label) in [(true, icons::FOLDER_OPEN, "文件"), (false, icons::HARD_DRIVES, "工作区")] {
                        let active = app.sidebar_files == files;
                        let button = egui::Button::new((icon(glyph, 14.0), RichText::new(label).size(11.0)))
                            .min_size(Vec2::new(width, 27.0))
                            .fill(if active { palette.hover() } else { Color32::TRANSPARENT })
                            .stroke(Stroke::NONE)
                            .corner_radius(5);
                        if ui.add(button).clicked() {
                            app.sidebar_files = files;
                        }
                    }
                });
                ui.add_space(10.0);
                if app.sidebar_files {
                    super::remote_panel::file_panel(app, ui, &remote, intents);
                    return;
                }
            }
            ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                bottom(ui, intents);
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        workspace(app, ui, intents);
                        connections(app, ui, intents);
                    });
                });
            });
        });
    app.sidebar_width = response.response.rect.width();
}

fn heading(ui: &mut Ui, title: &str, action: Option<(&str, &str)>) -> bool {
    let palette = style::palette(ui);
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.set_height(28.0);
        ui.add_space(10.0);
        ui.label(RichText::new(title).size(11.0).color(palette.muted()));
        if let Some((glyph, label)) = action {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| clicked = icon_button(ui, glyph, label, true).clicked());
        }
    });
    clicked
}

/// A full-width row button with an icon, a title and an optional second line.
fn row(ui: &mut Ui, glyph: &str, title: &str, detail: Option<&str>, active: bool, trailing: Option<&str>) -> egui::Response {
    let palette = style::palette(ui);
    let height = if detail.is_some() { 42.0 } else { 32.0 };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    let fill = if active { palette.hover() } else if response.hovered() { alpha(palette.tokens.ink, 0.04) } else { Color32::TRANSPARENT };
    ui.painter().rect_filled(rect, 6.0, fill);
    let mut text_left = rect.left() + 10.0;
    ui.painter().text(egui::pos2(text_left, rect.center().y), egui::Align2::LEFT_CENTER, glyph, egui::FontId::new(17.0, egui::FontFamily::Name(style::ICONS.into())), if active { palette.accent() } else { palette.muted() });
    text_left += 26.0;
    let width = rect.right() - text_left - if trailing.is_some() { 30.0 } else { 8.0 };
    let line = |text: &str, size: f32, color: Color32| {
        let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), egui::FontId::proportional(size), color);
        job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(10.0));
        ui.painter().layout_job(job)
    };
    let title = line(title, 12.0, if active { palette.text() } else { palette.text_soft() });
    match detail {
        Some(detail) => {
            let detail = line(detail, 10.0, palette.faint());
            ui.painter().galley(egui::pos2(text_left, rect.top() + 6.0), title, palette.text_soft());
            ui.painter().galley(egui::pos2(text_left, rect.top() + 23.0), detail, palette.faint());
        }
        None => {
            let y = rect.center().y - title.size().y / 2.0;
            ui.painter().galley(egui::pos2(text_left, y), title, palette.text_soft());
        }
    }
    if let Some(trailing) = trailing {
        ui.painter().text(egui::pos2(rect.right() - 10.0, rect.center().y), egui::Align2::RIGHT_CENTER, trailing, egui::FontId::proportional(10.0), palette.faint());
    }
    response
}

fn workspace(app: &mut App, ui: &mut Ui, intents: &mut Vec<Intent>) {
    let palette = style::palette(ui);
    if heading(ui, "工作区", Some((icons::FOLDER_OPEN, "打开工作文件夹"))) {
        intents.push(Intent::ChooseDirectory);
    }
    // The working folder: new local terminals start here.
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 52.0), Sense::click());
    ui.painter().rect(rect, 8.0, if response.hovered() { palette.hover() } else { alpha(palette.tokens.ink, 0.03) }, Stroke::new(1.0, palette.border), egui::StrokeKind::Inside);
    let avatar = egui::Rect::from_min_size(rect.left_top() + Vec2::new(9.0, 10.0), Vec2::splat(32.0));
    ui.painter().rect_filled(avatar, 7.0, alpha(palette.tokens.accent, 0.12));
    ui.painter().text(avatar.center(), egui::Align2::CENTER_CENTER, icons::FOLDER_SIMPLE, egui::FontId::new(20.0, egui::FontFamily::Name(style::ICONS.into())), palette.accent());
    let left = avatar.right() + 10.0;
    let width = rect.right() - left - 24.0;
    for (text, size, y, color) in [(basename(&app.cwd).to_string(), 12.5, 9.0, palette.text()), (path_label(&app.cwd, &app.home), 10.0, 28.0, palette.faint())] {
        let mut job = egui::text::LayoutJob::simple_singleline(text, egui::FontId::proportional(size), color);
        job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(10.0));
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(egui::pos2(left, rect.top() + y), galley, color);
    }
    ui.painter().text(egui::pos2(rect.right() - 12.0, rect.center().y), egui::Align2::RIGHT_CENTER, icons::CARET_RIGHT, egui::FontId::new(13.0, egui::FontFamily::Name(style::ICONS.into())), palette.faint());
    if response.on_hover_text(&app.cwd).clicked() {
        intents.push(Intent::ChooseDirectory);
    }
    ui.add_space(10.0);
    let locals = app.panes.values().filter(|pane| pane.session.kind == Kind::Shell).count();
    let focused_local = app.focused().is_some_and(|pane| pane.session.kind == Kind::Shell);
    if row(ui, icons::TERMINAL_WINDOW, "本地终端", None, focused_local, Some(&locals.to_string())).clicked() {
        let tab = app.tabs.iter().find(|tab| tab.root.panes().iter().any(|id| app.panes.get(id).is_some_and(|pane| pane.session.kind == Kind::Shell)));
        intents.push(match tab {
            Some(tab) => Intent::SelectTab(tab.id),
            None => Intent::Action(Action::NewTab),
        });
    }
    if row(ui, icons::ROBOT, "omniTerm Agent", None, false, None).clicked() && !app.agent_visible {
        intents.push(Intent::Action(Action::ToggleAgent));
    }
    ui.add_space(8.0);
}

fn connections(app: &mut App, ui: &mut Ui, intents: &mut Vec<Intent>) {
    let palette = style::palette(ui);
    if heading(ui, "已保存的连接", Some((icons::PLUS, "添加连接"))) {
        intents.push(Intent::Dialog(Dialog::Connection(ConnectionForm::new(Kind::Ssh, None))));
    }
    if app.profiles.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(14.0);
            ui.label(icon(icons::HARD_DRIVES, 23.0).color(palette.faint()));
            ui.label(RichText::new("你的连接，会在这里").size(12.0).color(palette.text_soft()));
            ui.label(RichText::new("保存常用主机，一键连接。").size(10.5).color(palette.faint()));
            ui.add_space(4.0);
            if ui.add(egui::Button::new((icon(icons::PLUS, 13.0).color(palette.accent()), RichText::new("添加第一个连接").size(11.0).color(palette.accent()))).frame(false)).clicked() {
                intents.push(Intent::Dialog(Dialog::Connection(ConnectionForm::new(Kind::Ssh, None))));
            }
            if ui.add(egui::Button::new((icon(icons::HARD_DRIVES, 13.0).color(palette.accent()), RichText::new("从 ~/.ssh/config 导入").size(11.0).color(palette.accent()))).frame(false)).clicked() {
                intents.push(Intent::OpenSshImport);
            }
        });
        return;
    }
    for profile in &app.profiles {
        let kind = profile.kind.unwrap_or(Kind::Ssh);
        let detail = profile_detail(profile);
        let response = row(ui, kind_icon(kind), &profile.display_name(), Some(&detail), false, None);
        let menu_rect = egui::Rect::from_center_size(egui::pos2(response.rect.right() - 16.0, response.rect.center().y), Vec2::splat(24.0));
        let menu = ui.put(menu_rect, egui::Button::new(icon(icons::DOTS_THREE, 18.0).color(palette.muted())).frame(false)).on_hover_text(format!("{}的选项", profile.display_name()));
        egui::Popup::menu(&menu).show(|ui| {
            if ui.button(format!("{}  编辑连接", icons::PENCIL_SIMPLE)).clicked() {
                intents.push(Intent::Dialog(Dialog::Connection(ConnectionForm::new(kind, Some(profile.clone())))));
            }
            if ui.button(format!("{}  删除连接", icons::TRASH)).clicked() {
                intents.push(Intent::Dialog(Dialog::Delete { profile: profile.clone(), error: None }));
            }
        });
        if response.on_hover_text(format!("连接 {}", profile.display_name())).clicked() {
            intents.push(Intent::NewTab { kind, profile: Some(profile.clone()), cwd: None });
        }
    }
}

pub fn profile_detail(profile: &Profile) -> String {
    if profile.kind == Some(Kind::Serial) {
        return profile.device.clone().unwrap_or_default();
    }
    if let Some(summary) = &profile.summary {
        return summary.clone();
    }
    let user = profile.username.as_deref().map(|user| format!("{user}@")).unwrap_or_default();
    let jump = profile.proxy_jump.as_deref().map(|jump| format!(" · 经 {jump}")).unwrap_or_default();
    format!("{user}{}{jump}", profile.host.as_deref().unwrap_or_default())
}

fn bottom(ui: &mut Ui, intents: &mut Vec<Intent>) {
    let palette = style::palette(ui);
    ui.add_space(8.0);
    // Settings, with the version on the right.
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 6.0, palette.hover());
    }
    ui.painter().text(rect.left_center() + Vec2::new(10.0, 0.0), egui::Align2::LEFT_CENTER, icons::GEAR_SIX, egui::FontId::new(18.0, egui::FontFamily::Name(style::ICONS.into())), palette.muted());
    ui.painter().text(rect.left_center() + Vec2::new(36.0, 0.0), egui::Align2::LEFT_CENTER, "设置", egui::FontId::proportional(12.0), palette.text_soft());
    ui.painter().text(rect.right_center() - Vec2::new(10.0, 0.0), egui::Align2::RIGHT_CENTER, format!("v{}", env!("CARGO_PKG_VERSION")), egui::FontId::monospace(9.5), palette.faint());
    if response.clicked() {
        intents.push(Intent::Action(Action::Settings));
    }
    ui.add_space(4.0);
    // Connection tools: a 2×2 grid, as in the desktop app.
    let width = (ui.available_width() - 6.0) / 2.0;
    for kinds in Kind::CONNECTIONS.chunks(2).rev() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for &kind in kinds {
                let button = egui::Button::new((icon(kind_icon(kind), 16.0).color(palette.muted()), RichText::new(connection_label(kind)).size(11.0).color(palette.text_soft())))
                    .min_size(Vec2::new(width, 32.0))
                    .fill(alpha(palette.tokens.ink, 0.03))
                    .stroke(Stroke::new(1.0, palette.border))
                    .corner_radius(6);
                if ui.add(button).clicked() {
                    intents.push(Intent::Dialog(Dialog::Connection(ConnectionForm::new(kind, None))));
                }
            }
        });
    }
    ui.horizontal(|ui| {
        ui.set_height(28.0);
        ui.add_space(10.0);
        ui.label(RichText::new("连接工具").size(11.0).color(palette.muted()));
    });
    let line = ui.min_rect();
    ui.painter().hline(line.x_range(), line.top() - 2.0, Stroke::new(1.0, palette.border));
}

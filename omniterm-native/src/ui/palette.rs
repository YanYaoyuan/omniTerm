// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! The command palette: every command, saved connection and tab, fuzzy-matched.

use super::sidebar::{connection_label, kind_icon};
use super::style::{self, icon};
use super::{App, Intent, shortcuts::{self, Action}};
use crate::core::profile::Kind;
use crate::core::text::fuzzy_score;
use crate::icons;
use crate::theme::THEMES;
use eframe::egui::{self, Color32, Id, Key, RichText, Sense, Stroke, Vec2};

#[derive(Default)]
pub struct State {
    query: String,
    index: usize,
}

struct Command {
    group: &'static str,
    label: String,
    keywords: &'static str,
    icon: &'static str,
    shortcut: Option<String>,
    disabled: bool,
    run: Intent,
}

fn commands(app: &App) -> Vec<Command> {
    let mac = app.mac;
    let key = |action| Some(shortcuts::label(action, mac));
    let tab = app.active_tab();
    let panes = tab.map(|tab| tab.root.panes().len()).unwrap_or(0);
    let has = app.focused().is_some();
    let command = |group, label: &str, icon, shortcut, disabled, run| Command { group, label: label.to_string(), keywords: "", icon, shortcut, disabled, run };
    let mut list = vec![
        command("终端", "新建本地终端标签", icons::PLUS, key(Action::NewTab), false, Intent::Action(Action::NewTab)),
        command("窗格", "向右分屏", icons::SQUARE_SPLIT_HORIZONTAL, key(Action::SplitRight), false, Intent::Action(Action::SplitRight)),
        command("窗格", "向下分屏", icons::SQUARE_SPLIT_VERTICAL, key(Action::SplitDown), false, Intent::Action(Action::SplitDown)),
        command("窗格", if tab.is_some_and(|tab| tab.zoomed) { "还原窗格" } else { "最大化当前窗格" }, icons::ARROWS_OUT, key(Action::MaximizePane), panes < 2, Intent::Action(Action::MaximizePane)),
        Command { keywords: "fullscreen focus zoom 全屏 专注", ..command("窗格", if app.focus_mode { "退出全屏终端" } else { "全屏终端（隐藏其他面板，所有窗格一起全屏）" }, icons::CORNERS_OUT, key(Action::ToggleFullscreen), tab.is_none(), Intent::Action(Action::ToggleFullscreen)) },
        command("窗格", if tab.is_some_and(|tab| tab.broadcast) { "关闭广播输入" } else { "开启广播输入（同时输入到所有窗格）" }, icons::BROADCAST, key(Action::Broadcast), tab.is_none(), Intent::Action(Action::Broadcast)),
        command("终端", "关闭当前窗格", icons::X, key(Action::ClosePane), !has, Intent::Action(Action::ClosePane)),
        command("终端", "搜索输出", icons::MAGNIFYING_GLASS, key(Action::Find), !has, Intent::Action(Action::Find)),
        command("终端", "清空屏幕", icons::TRASH, key(Action::Clear), !has, Intent::Action(Action::Clear)),
        command("终端", "全选输出", icons::COPY, None, !has, Intent::SelectAll),
        command("工作区", "在文件夹中打开终端…", icons::FOLDER_OPEN, None, false, Intent::ChooseDirectory),
        command("工作区", if app.sidebar_visible { "隐藏工作区侧栏" } else { "显示工作区侧栏" }, icons::SIDEBAR_SIMPLE, key(Action::ToggleSidebar), false, Intent::Action(Action::ToggleSidebar)),
        command("omniTerm Agent", "附加选中内容到 omniTerm Agent", icons::PAPER_PLANE_TILT, key(Action::SendToAgent), false, Intent::Action(Action::SendToAgent)),
        command("omniTerm Agent", if app.agent_visible { "隐藏 omniTerm Agent 面板" } else { "显示 omniTerm Agent 面板" }, icons::SIDEBAR_SIMPLE, key(Action::ToggleAgent), false, Intent::Action(Action::ToggleAgent)),
        command("外观", "放大字体", icons::TEXT_AA, key(Action::FontBigger), false, Intent::Action(Action::FontBigger)),
        command("外观", "缩小字体", icons::TEXT_AA, key(Action::FontSmaller), false, Intent::Action(Action::FontSmaller)),
        command("外观", "重置字体大小", icons::TEXT_AA, key(Action::FontReset), false, Intent::Action(Action::FontReset)),
    ];
    for theme in &THEMES {
        list.push(Command { keywords: "theme color 配色", ..command("主题", theme.name, icons::PALETTE, None, false, Intent::SetTheme(theme.id)) });
    }
    list.push(command("设置", "打开终端设置", icons::GEAR_SIX, key(Action::Settings), false, Intent::Action(Action::Settings)));
    list.push(Command { keywords: "ssh config import 导入", ..command("连接", "从 ~/.ssh/config 导入主机…", icons::HARD_DRIVES, None, false, Intent::OpenSshImport) });
    for kind in Kind::CONNECTIONS {
        list.push(command("新建连接", connection_label(kind), kind_icon(kind), None, false, Intent::Dialog(super::Dialog::Connection(super::dialogs::ConnectionForm::new(kind, None)))));
    }
    for profile in &app.profiles {
        let kind = profile.kind.unwrap_or(Kind::Ssh);
        list.push(Command { label: profile.display_name(), keywords: "", ..command("连接", "", kind_icon(kind), None, false, Intent::NewTab { kind, profile: Some(profile.clone()), cwd: None }) });
    }
    for (index, tab) in app.tabs.iter().enumerate() {
        let kind = app.panes.get(&tab.focus).map(|pane| pane.session.kind).unwrap_or(Kind::Shell);
        list.push(Command { label: format!("{}. {}", index + 1, super::terminals::tab_label(app, tab)), ..command("标签", "", kind_icon(kind), None, false, Intent::SelectTab(tab.id)) });
    }
    list
}

pub fn show(app: &mut App, ctx: &egui::Context, intents: &mut Vec<Intent>) {
    let Some(state) = app.palette.as_mut() else { return };
    let query = state.query.clone();
    let mut list = commands(app);
    let mut scored: Vec<(f32, usize)> = list
        .iter()
        .enumerate()
        .map(|(index, command)| (fuzzy_score(&query, &format!("{} {} {}", command.group, command.label, command.keywords)), index))
        .filter(|(score, _)| *score > 0.0)
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored.truncate(60);
    let palette = style::palette_of(ctx);
    let Some(state) = app.palette.as_mut() else { return };
    state.index = state.index.min(scored.len().saturating_sub(1));
    let (down, up, enter) = ctx.input(|input| (input.key_pressed(Key::ArrowDown), input.key_pressed(Key::ArrowUp), input.key_pressed(Key::Enter)));
    if down {
        state.index = (state.index + 1).min(scored.len().saturating_sub(1));
    }
    if up {
        state.index = state.index.saturating_sub(1);
    }
    let mut chosen = enter.then_some(state.index);
    let response = egui::Modal::new(Id::unique("palette"))
        .backdrop_color(Color32::from_black_alpha(110))
        .frame(egui::Frame::new().fill(palette.surface()).stroke(Stroke::new(1.0, palette.border_strong)).corner_radius(10).inner_margin(6))
        .show(ctx, |ui| {
            ui.set_width(560.0);
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.label(icon(icons::MAGNIFYING_GLASS, 16.0).color(palette.muted()));
                let edit = ui.add(egui::TextEdit::singleline(&mut state.query).hint_text("输入命令、连接或标签名称…").frame(egui::Frame::NONE).desired_width(f32::INFINITY).font(egui::FontId::proportional(14.0)).margin(Vec2::new(4.0, 10.0)));
                edit.request_focus();
                if edit.changed() {
                    state.index = 0;
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                if scored.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(18.0);
                        ui.label(RichText::new("没有匹配的命令").color(palette.muted()));
                        ui.add_space(18.0);
                    });
                }
                for (position, (_, index)) in scored.iter().enumerate() {
                    let command = &list[*index];
                    let selected = position == state.index;
                    let (rect, row) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), if command.disabled { Sense::hover() } else { Sense::click() });
                    if selected {
                        ui.painter().rect_filled(rect, 6.0, palette.hover());
                        if down || up {
                            ui.scroll_to_rect(rect, None);
                        }
                    }
                    if row.hovered() && ui.input(|input| input.pointer.delta() != Vec2::ZERO) {
                        state.index = position;
                    }
                    let tint = if command.disabled { palette.faint() } else { palette.text_soft() };
                    let font = |size| egui::FontId::proportional(size);
                    ui.painter().text(rect.left_center() + Vec2::new(12.0, 0.0), egui::Align2::LEFT_CENTER, command.icon, egui::FontId::new(16.0, egui::FontFamily::Name(style::ICONS.into())), tint);
                    ui.painter().text(rect.left_center() + Vec2::new(38.0, 0.0), egui::Align2::LEFT_CENTER, command.group, font(10.5), palette.faint());
                    ui.painter().text(rect.left_center() + Vec2::new(130.0, 0.0), egui::Align2::LEFT_CENTER, &command.label, font(12.5), tint);
                    if let Some(shortcut) = &command.shortcut {
                        ui.painter().text(rect.right_center() - Vec2::new(12.0, 0.0), egui::Align2::RIGHT_CENTER, shortcut, egui::FontId::monospace(10.0), palette.muted());
                    }
                    if row.clicked() {
                        chosen = Some(position);
                    }
                }
            });
        });
    if let Some(position) = chosen
        && let Some((_, index)) = scored.get(position)
        && !list[*index].disabled
    {
        let command = list.swap_remove(*index);
        app.palette = None;
        intents.push(command.run);
        return;
    }
    if response.should_close() {
        app.palette = None;
    }
}

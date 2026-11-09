// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! The terminal area: tabs, the terminal toolbar, search, split panes drawn on the GPU, and
//! everything typed, clicked or dropped on a terminal.

use super::sidebar::{connection_label, kind_icon};
use super::style::{self, alpha, icon, icon_button, kbd};
use super::{App, Intent, Pane, Tab, shortcuts::{self, Action}};
use crate::core::profile::Kind;
use crate::core::text::{path_label, quote_path};
use crate::glyphs::Upload;
use crate::gpu::GridPaint;
use crate::grid::{self, PADDING};
use crate::icons;
use crate::input::key_bytes;
use crate::layout::{Divider, Rect as Fraction};
use crate::terminal::{Size, mouse_report};
use alacritty_terminal::index::Side;
use alacritty_terminal::selection::SelectionType;
use alacritty_terminal::term::TermMode;
use eframe::egui::{self, Align, Align2, Color32, CursorIcon, Event, FontId, Id, Key, Layout, Modifiers, PointerButton, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use eframe::egui_wgpu;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const TOOLBAR: f32 = 55.0;
const TABSTRIP: f32 = 41.0;
const TERMINAL_TOOLBAR: f32 = 34.0;
const FOOTNOTE: f32 = 28.0;
const METRICS: f32 = 32.0;
const GAP: f32 = 6.0;
const BLINK: Duration = Duration::from_millis(600);

pub fn tab_label(app: &App, tab: &Tab) -> String {
    if !tab.title.is_empty() {
        return tab.title.clone();
    }
    app.panes.get(&tab.focus).map(Pane::label).unwrap_or_else(|| "终端".into())
}

pub fn show(app: &mut App, ui: &mut Ui, intents: &mut Vec<Intent>, modal: bool) {
    let palette = style::palette(ui);
    let full = ui.max_rect();
    ui.painter().rect_filled(full, 0.0, palette.panel_soft);
    let mut area = full;
    if !app.focus_mode {
        let toolbar = Rect::from_min_size(area.min, Vec2::new(area.width(), TOOLBAR));
        workspace_toolbar(app, ui, toolbar, intents);
        area.min.y += TOOLBAR;
    }
    let strip = Rect::from_min_size(area.min, Vec2::new(area.width(), TABSTRIP));
    tab_strip(app, ui, strip, intents);
    area.min.y += TABSTRIP;
    let bar = Rect::from_min_size(area.min, Vec2::new(area.width(), TERMINAL_TOOLBAR));
    terminal_toolbar(app, ui, bar, intents);
    area.min.y += TERMINAL_TOOLBAR;
    if app.search.open {
        let search = Rect::from_min_size(area.min + Vec2::new(12.0, 2.0), Vec2::new(area.width() - 24.0, 39.0));
        search_bar(app, ui, search, intents);
        area.min.y += 43.0;
    }
    if !app.focus_mode {
        let note = Rect::from_min_max(Pos2::new(area.left(), area.bottom() - FOOTNOTE), area.max);
        footnote(app, ui, note);
        area.max.y -= FOOTNOTE;
        if let Some(remote) = app.focused_remote().filter(|remote| !remote.is_independent()) {
            let bar = Rect::from_min_max(Pos2::new(area.left(), area.bottom() - METRICS), area.max);
            super::remote_panel::metrics_bar(ui, bar, &remote.snapshot());
            area.max.y -= METRICS;
        }
    }
    area.min.y += 4.0;
    ui.painter().rect_filled(area, 0.0, if palette.panel_bg == Color32::TRANSPARENT { Color32::TRANSPARENT } else { style::color(app.theme().background) });
    stack(app, ui, area, intents, modal);
}

fn workspace_toolbar(app: &App, ui: &mut Ui, rect: Rect, intents: &mut Vec<Intent>) {
    let palette = style::palette(ui);
    ui.painter().rect_filled(rect, 0.0, palette.panel_soft);
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, palette.border));
    let directory = app.focused().map(|pane| pane.cwd().to_string()).unwrap_or_else(|| app.cwd.clone());
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink2(Vec2::new(20.0, 0.0))).layout(Layout::left_to_right(Align::Center)), |ui| {
        ui.label(icon(icons::FOLDER_SIMPLE, 16.0).color(palette.muted()));
        ui.label(RichText::new("工作区").size(12.0).color(palette.muted()));
        ui.label(icon(icons::CARET_RIGHT, 12.0).color(palette.faint()));
        ui.add(egui::Label::new(RichText::new(path_label(&directory, &app.home)).size(12.0).strong().color(palette.text())).truncate()).on_hover_text(&directory);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if style::secondary_button(ui, &format!("{}  打开文件夹", icons::FOLDER_OPEN)).clicked() {
                intents.push(Intent::ChooseDirectory);
            }
        });
    });
}

fn tab_strip(app: &mut App, ui: &mut Ui, rect: Rect, intents: &mut Vec<Intent>) {
    let palette = style::palette(ui);
    ui.painter().rect_filled(rect, 0.0, palette.panel_soft);
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, palette.border));
    let right_room = if app.focus_mode { 76.0 } else { 44.0 };
    let tabs_rect = Rect::from_min_max(rect.min, Pos2::new(rect.right() - right_room, rect.bottom()));
    let count = app.tabs.len().max(1) as f32;
    let width = (tabs_rect.width() / count).clamp(90.0, 220.0);
    let dragged: Option<u64> = ui.ctx().data(|data| data.get_temp(Id::unique("dragged-tab")));
    for (index, tab) in app.tabs.iter().enumerate() {
        let cell = Rect::from_min_size(Pos2::new(tabs_rect.left() + width * index as f32, rect.top()), Vec2::new(width, rect.height()));
        if cell.left() > tabs_rect.right() {
            break;
        }
        let active = Some(tab.id) == app.active;
        let id = Id::unique(("tab", tab.id));
        let response = ui.interact(cell, id, Sense::click_and_drag());
        if active {
            ui.painter().rect_filled(cell, 0.0, if palette.panel_bg == Color32::TRANSPARENT { alpha(palette.tokens.ink, 0.07) } else { palette.panel_bg });
            ui.painter().hline(cell.x_range(), cell.bottom() - 1.0, Stroke::new(2.0, palette.accent()));
        } else if response.hovered() {
            ui.painter().rect_filled(cell, 0.0, alpha(palette.tokens.ink, 0.03));
        }
        if dragged == Some(tab.id) {
            ui.painter().rect_filled(cell, 0.0, alpha(palette.tokens.bg, 0.5));
        }
        ui.painter().vline(cell.right() - 0.5, cell.y_range().shrink(10.0), Stroke::new(1.0, palette.border));
        let focus = app.panes.get(&tab.focus);
        let kind = focus.map(|pane| pane.session.kind).unwrap_or(Kind::Shell);
        let color = if active { palette.text_soft() } else { palette.muted() };
        let mut x = cell.left() + 14.0;
        ui.painter().text(Pos2::new(x, cell.center().y), Align2::LEFT_CENTER, kind_icon(kind), FontId::new(15.0, egui::FontFamily::Name(style::ICONS.into())), color);
        x += 22.0;
        let ids = tab.root.panes();
        let finished = ids.iter().all(|id| app.panes.get(id).is_none_or(|pane| !pane.running()));
        let mut trailing = 30.0;
        if ids.len() > 1 {
            trailing += 20.0;
        }
        if tab.broadcast {
            trailing += 16.0;
        }
        let renaming = ui.ctx().data(|data| data.get_temp::<(u64, String)>(Id::unique("renaming"))).filter(|(id, _)| *id == tab.id);
        if let Some((_, mut value)) = renaming {
            let edit = Rect::from_min_max(Pos2::new(x, cell.top() + 8.0), Pos2::new(cell.right() - trailing, cell.bottom() - 8.0));
            let response = ui.put(edit, egui::TextEdit::singleline(&mut value).char_limit(60).font(FontId::proportional(11.5)));
            response.request_focus();
            let done = response.lost_focus() || ui.input(|input| input.key_pressed(Key::Enter));
            let cancel = ui.input(|input| input.key_pressed(Key::Escape));
            ui.ctx().data_mut(|data| {
                if done || cancel {
                    data.remove::<(u64, String)>(Id::unique("renaming"));
                } else {
                    data.insert_temp(Id::unique("renaming"), (tab.id, value.clone()));
                }
            });
            if done && !cancel {
                intents.push(Intent::RenameTab(tab.id, value));
            }
        } else {
            let mut job = egui::text::LayoutJob::simple_singleline(tab_label(app, tab), FontId::proportional(11.5), color);
            job.wrap = egui::text::TextWrapping::truncate_at_width((cell.right() - trailing - x).max(12.0));
            let galley = ui.painter().layout_job(job);
            ui.painter().galley(Pos2::new(x, cell.center().y - galley.size().y / 2.0), galley, color);
        }
        let mut right = cell.right() - 30.0;
        if finished {
            ui.painter().circle_filled(Pos2::new(right - 6.0, cell.center().y), 3.0, palette.faint());
            right -= 14.0;
        }
        if tab.broadcast {
            ui.painter().text(Pos2::new(right - 6.0, cell.center().y), Align2::CENTER_CENTER, icons::BROADCAST, FontId::new(12.0, egui::FontFamily::Name(style::ICONS.into())), palette.accent());
            right -= 16.0;
        }
        if ids.len() > 1 {
            let badge = Rect::from_center_size(Pos2::new(right - 8.0, cell.center().y), Vec2::new(18.0, 16.0));
            ui.painter().rect_filled(badge, 8.0, alpha(palette.tokens.ink, 0.08));
            ui.painter().text(badge.center(), Align2::CENTER_CENTER, ids.len().to_string(), FontId::proportional(9.5), palette.muted());
        }
        let close = Rect::from_center_size(Pos2::new(cell.right() - 16.0, cell.center().y), Vec2::splat(22.0));
        let close_response = ui.interact(close, id.with("close"), Sense::click()).on_hover_text(format!("关闭 {}", tab_label(app, tab)));
        if close_response.hovered() {
            ui.painter().rect_filled(close, 4.0, palette.hover());
        }
        if response.hovered() || active || close_response.hovered() {
            ui.painter().text(close.center(), Align2::CENTER_CENTER, icons::X, FontId::new(12.0, egui::FontFamily::Name(style::ICONS.into())), palette.muted());
        }
        if close_response.clicked() {
            intents.push(Intent::CloseTab(tab.id));
            continue;
        }
        let number = if index < 9 { format!("（{}{}）", if app.mac { "⌘" } else { "Alt+" }, index + 1) } else { String::new() };
        let response = response.on_hover_text(format!("{}{number}\n双击重命名，拖动排序", tab_label(app, tab)));
        if response.double_clicked() {
            ui.ctx().data_mut(|data| data.insert_temp(Id::unique("renaming"), (tab.id, tab_label(app, tab))));
        } else if response.clicked() {
            intents.push(Intent::SelectTab(tab.id));
        }
        // Dragging a tab over another moves it there.
        if response.drag_started() {
            ui.ctx().data_mut(|data| data.insert_temp(Id::unique("dragged-tab"), tab.id));
        }
        if let Some(from) = dragged
            && from != tab.id
            && ui.input(|input| input.pointer.hover_pos()).is_some_and(|pointer| cell.contains(pointer))
            && ui.input(|input| input.pointer.any_released())
        {
            intents.push(Intent::MoveTab { from, to: tab.id });
        }
    }
    if ui.input(|input| input.pointer.any_released()) {
        ui.ctx().data_mut(|data| data.remove::<u64>(Id::unique("dragged-tab")));
    }
    // A pane dropped on the tab strip becomes a tab of its own.
    if let Some(dragged) = app.dragging_pane.clone()
        && ui.input(|input| input.pointer.hover_pos()).is_some_and(|pointer| rect.contains(pointer))
    {
        ui.painter().rect(rect.shrink(2.0), 4.0, alpha(palette.tokens.accent, 0.1), Stroke::new(1.5, alpha(palette.tokens.accent, 0.6)), egui::StrokeKind::Inside);
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, "放到这里成为新标签", FontId::proportional(11.0), palette.text_soft());
        if ui.input(|input| input.pointer.any_released()) {
            intents.push(Intent::MovePane { id: dragged, target: None });
            app.dragging_pane = None;
        }
    }
    let new_tab = Rect::from_center_size(Pos2::new(tabs_rect.right() + 22.0, rect.center().y), Vec2::splat(28.0));
    let response = ui.put(new_tab, egui::Button::new(icon(icons::PLUS, 17.0).color(palette.muted())).frame(false)).on_hover_text(format!("新建终端（{}）", shortcuts::label(Action::NewTab, app.mac)));
    if response.clicked() {
        intents.push(Intent::Action(Action::NewTab));
    }
    if app.focus_mode {
        let exit = Rect::from_center_size(Pos2::new(rect.right() - 20.0, rect.center().y), Vec2::splat(28.0));
        if ui.put(exit, egui::Button::new(icon(icons::ARROWS_IN, 16.0).color(palette.accent())).frame(false)).on_hover_text(format!("退出全屏（{}）", shortcuts::label(Action::ToggleFullscreen, app.mac))).clicked() {
            intents.push(Intent::Action(Action::ToggleFullscreen));
        }
    }
}

fn terminal_toolbar(app: &App, ui: &mut Ui, rect: Rect, intents: &mut Vec<Intent>) {
    let palette = style::palette(ui);
    ui.painter().rect_filled(rect, 0.0, palette.panel_bg);
    let pane = app.focused();
    let tab = app.active_tab();
    let panes = tab.map(|tab| tab.root.panes().len()).unwrap_or(0);
    ui.scope_builder(egui::UiBuilder::new().max_rect(Rect::from_min_max(rect.min + Vec2::new(17.0, 0.0), rect.max - Vec2::new(10.0, 0.0))).layout(Layout::left_to_right(Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let small = |text: String| RichText::new(text).size(11.0).color(palette.muted());
        match pane {
            Some(pane) => {
                super::chrome::status_dot(ui, pane.running());
                ui.label(small(if pane.session.kind == Kind::Shell { "本地 Shell".into() } else { connection_label(pane.session.kind).into() }));
                ui.label(small("·".into()));
                ui.label(small(if pane.running() { "已连接".into() } else { "会话已结束".into() }));
                if panes > 1 {
                    ui.label(small("·".into()));
                    ui.label(small(format!("{panes} 个窗格{}", if tab.is_some_and(|tab| tab.zoomed) { "（已最大化）" } else { "" })));
                }
                if tab.is_some_and(|tab| tab.broadcast) {
                    egui::Frame::new().fill(alpha(palette.tokens.accent, 0.12)).corner_radius(9).inner_margin(egui::Margin::symmetric(7, 1)).show(ui, |ui| {
                        ui.label(RichText::new(format!("{} 广播输入", icons::BROADCAST)).size(10.0).color(palette.accent()));
                    });
                }
            }
            None => {
                ui.label(small("终端".into()));
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let has = pane.is_some();
            let key = |action| shortcuts::label(action, app.mac);
            let button = |ui: &mut Ui, intents: &mut Vec<Intent>, glyph: &str, label: String, enabled: bool, intent: Intent| {
                if icon_button(ui, glyph, &label, enabled).clicked() && enabled {
                    intents.push(intent);
                }
            };
            button(ui, intents, icons::TRASH, format!("清空终端屏幕（{}）", key(Action::Clear)), has, Intent::Action(Action::Clear));
            button(ui, intents, icons::PAPER_PLANE_TILT, format!("附加选中内容到 omniTerm Agent（{}）", key(Action::SendToAgent)), has, Intent::Action(Action::SendToAgent));
            button(ui, intents, icons::CLIPBOARD_TEXT, "粘贴到终端".into(), has, Intent::Paste);
            button(ui, intents, icons::COPY, "复制选中文本".into(), has, Intent::Copy);
            button(ui, intents, icons::MAGNIFYING_GLASS, format!("搜索终端（{}）", key(Action::Find)), has, Intent::Action(Action::Find));
            let (separator, _) = ui.allocate_exact_size(Vec2::new(9.0, 16.0), Sense::hover());
            ui.painter().vline(separator.center().x, separator.y_range(), Stroke::new(1.0, palette.border_strong));
            button(ui, intents, icons::BROADCAST, format!("广播输入到所有窗格（{}）", key(Action::Broadcast)), tab.is_some(), Intent::Action(Action::Broadcast));
            button(ui, intents, icons::CORNERS_OUT, format!("全屏终端：隐藏其他面板，当前标签的所有窗格一起全屏（{}）", key(Action::ToggleFullscreen)), tab.is_some(), Intent::Action(Action::ToggleFullscreen));
            let zoomed = tab.is_some_and(|tab| tab.zoomed);
            button(ui, intents, if zoomed { icons::ARROWS_IN } else { icons::ARROWS_OUT }, format!("{}窗格（{}）", if zoomed { "还原" } else { "最大化" }, key(Action::MaximizePane)), panes > 1, Intent::Action(Action::MaximizePane));
            button(ui, intents, icons::SQUARE_SPLIT_VERTICAL, format!("向下分屏（{}）", key(Action::SplitDown)), tab.is_some(), Intent::Action(Action::SplitDown));
            button(ui, intents, icons::SQUARE_SPLIT_HORIZONTAL, format!("向右分屏（{}）", key(Action::SplitRight)), tab.is_some(), Intent::Action(Action::SplitRight));
        });
    });
}

fn search_bar(app: &mut App, ui: &mut Ui, rect: Rect, intents: &mut Vec<Intent>) {
    let palette = style::palette(ui);
    ui.painter().rect(rect, 5.0, palette.hover(), Stroke::new(1.0, alpha(palette.tokens.accent, 0.19)), egui::StrokeKind::Inside);
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink2(Vec2::new(10.0, 0.0))).layout(Layout::left_to_right(Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(icon(icons::MAGNIFYING_GLASS, 15.0).color(palette.muted()));
        let response = ui.add(egui::TextEdit::singleline(&mut app.search.value).hint_text("搜索终端输出").frame(egui::Frame::NONE).desired_width(ui.available_width() - 190.0).font(FontId::proportional(11.5)));
        if app.search.focus {
            response.request_focus();
            app.search.focus = false;
        }
        if response.changed() {
            intents.push(Intent::Find { backwards: false });
        }
        if response.has_focus() || response.lost_focus() {
            if ui.input(|input| input.key_pressed(Key::Enter)) {
                let backwards = ui.input(|input| input.modifiers.shift);
                intents.push(Intent::Find { backwards });
                response.request_focus();
            }
            if ui.input(|input| input.key_pressed(Key::Escape)) {
                intents.push(Intent::CloseSearch);
            }
        }
        if !app.search.value.is_empty() && !app.search.found {
            ui.label(RichText::new("未找到").size(10.0).color(palette.danger()));
        }
        let option = |ui: &mut Ui, on: &mut bool, label: &str, tooltip: &str| {
            let text = RichText::new(label).monospace().size(11.0).color(if *on { palette.accent() } else { palette.muted() });
            if ui.add(egui::Button::new(text).fill(if *on { alpha(palette.tokens.accent, 0.12) } else { Color32::TRANSPARENT }).stroke(Stroke::NONE)).on_hover_text(tooltip).clicked() {
                *on = !*on;
                true
            } else {
                false
            }
        };
        if option(ui, &mut app.search.options.case_sensitive, "Aa", "区分大小写") | option(ui, &mut app.search.options.regex, ".*", "正则表达式") {
            intents.push(Intent::Find { backwards: false });
        }
        if icon_button(ui, icons::ARROW_UP, "上一个结果", true).clicked() {
            intents.push(Intent::Find { backwards: true });
        }
        if icon_button(ui, icons::ARROW_DOWN, "下一个结果", true).clicked() {
            intents.push(Intent::Find { backwards: false });
        }
        if icon_button(ui, icons::X, "关闭搜索", true).clicked() {
            intents.push(Intent::CloseSearch);
        }
    });
}

fn footnote(app: &App, ui: &mut Ui, rect: Rect) {
    let palette = style::palette(ui);
    ui.painter().rect_filled(rect, 0.0, palette.panel_bg);
    let directory = app.focused().map(|pane| pane.cwd().to_string()).unwrap_or_else(|| app.cwd.clone());
    let font = FontId::monospace(9.0);
    ui.painter().text(rect.left_center() + Vec2::new(16.0, 0.0), Align2::LEFT_CENTER, path_label(&directory, &app.home), font.clone(), palette.faint());
    ui.painter().text(rect.right_center() - Vec2::new(16.0, 0.0), Align2::RIGHT_CENTER, format!("{} · GPU · UTF-8", app.theme().name), font, palette.faint());
}

/// Splits, panes and dividers of the active tab.
fn stack(app: &mut App, ui: &mut Ui, area: Rect, intents: &mut Vec<Intent>, modal: bool) {
    let palette = style::palette(ui);
    let Some(tab) = app.active_tab() else {
        ui.scope_builder(egui::UiBuilder::new().max_rect(area).layout(Layout::top_down(Align::Center)), |ui| {
            ui.add_space(area.height() * 0.32);
            ui.label(icon(icons::TERMINAL_WINDOW, 34.0).color(palette.faint()));
            ui.add_space(14.0);
            ui.label(RichText::new("开始一个新的会话").size(17.0).color(palette.text_soft()));
            ui.label(RichText::new("打开本地终端，或连接到你的远程主机。").size(11.0).color(palette.muted()));
            ui.add_space(18.0);
            if style::primary_button(ui, &format!("{}  新建终端  {}", icons::PLUS, shortcuts::label(Action::NewTab, app.mac))).clicked() {
                intents.push(Intent::Action(Action::NewTab));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("按").size(10.0).color(palette.faint()));
                kbd(ui, &shortcuts::label(Action::Palette, app.mac));
                ui.label(RichText::new("打开命令面板").size(10.0).color(palette.faint()));
            });
        });
        return;
    };
    let tab_id = tab.id;
    let focus = tab.focus.clone();
    let (mut panes, mut dividers) = (Vec::new(), Vec::new());
    let whole = Fraction { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
    if tab.zoomed {
        panes.push((focus.clone(), whole));
    } else {
        tab.root.layout(whole, &mut Vec::new(), &mut panes, &mut dividers);
    }
    let many = panes.len() > 1;
    let uploads = app.frame_uploads.clone();
    let tab_has_keyboard = app.docked_focus.is_none();
    for (id, fraction) in &panes {
        let rect = pane_rect(area, *fraction);
        let focused = *id == focus && tab_has_keyboard;
        pane(app, ui, id, rect, focused, many, &uploads, intents, modal);
    }
    for divider in dividers {
        drag_divider(ui, area, tab_id, divider, intents);
    }
    // A dragged pane lands on the nearest edge of the pane under the pointer.
    if let Some(dragged) = app.dragging_pane.clone() {
        let pointer = ui.input(|input| input.pointer.hover_pos());
        let released = ui.input(|input| input.pointer.any_released());
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        let docking = super::dock::edge_band(ui.ctx(), pointer).is_some();
        for (id, fraction) in &panes {
            let rect = pane_rect(area, *fraction);
            if docking || *id == dragged || !pointer.is_some_and(|pointer| rect.contains(pointer)) {
                continue;
            }
            let pointer = pointer.expect("checked");
            let side = crate::layout::drop_side((pointer.x - rect.left()) / rect.width(), (pointer.y - rect.top()) / rect.height());
            let preview = match side {
                crate::layout::Direction::Left => Rect::from_min_max(rect.min, Pos2::new(rect.center().x, rect.bottom())),
                crate::layout::Direction::Right => Rect::from_min_max(Pos2::new(rect.center().x, rect.top()), rect.max),
                crate::layout::Direction::Up => Rect::from_min_max(rect.min, Pos2::new(rect.right(), rect.center().y)),
                crate::layout::Direction::Down => Rect::from_min_max(Pos2::new(rect.left(), rect.center().y), rect.max),
            };
            ui.painter().rect(preview.shrink(4.0), 6.0, alpha(palette.tokens.accent, 0.14), Stroke::new(1.5, alpha(palette.tokens.accent, 0.7)), egui::StrokeKind::Inside);
            if released {
                intents.push(Intent::MovePane { id: dragged.clone(), target: Some((id.clone(), side)) });
            }
        }
        if released {
            app.dragging_pane = None;
        }
    }
}

/// A terminal docked on a window edge, drawn and driven like any pane.
pub fn docked(app: &mut App, ui: &mut Ui, id: &str, rect: Rect, intents: &mut Vec<Intent>) {
    let focused = app.docked_focus.as_deref() == Some(id);
    let modal = app.dialog.is_some() || app.palette.is_some();
    let uploads = app.frame_uploads.clone();
    pane(app, ui, id, rect.shrink(3.0), focused, true, &uploads, intents, modal);
}

/// A pane's place, with a gutter between neighbors and none at the edges.
fn pane_rect(area: Rect, fraction: Fraction) -> Rect {
    let edge = |start: f32, size: f32| (if start > 0.0 { GAP / 2.0 } else { 0.0 }, if start + size < 1.0 - 1e-6 { GAP / 2.0 } else { 0.0 });
    let (left, right) = edge(fraction.x, fraction.w);
    let (top, bottom) = edge(fraction.y, fraction.h);
    Rect::from_min_max(
        Pos2::new(area.left() + fraction.x * area.width() + left, area.top() + fraction.y * area.height() + top),
        Pos2::new(area.left() + (fraction.x + fraction.w) * area.width() - right, area.top() + (fraction.y + fraction.h) * area.height() - bottom),
    )
}

fn drag_divider(ui: &mut Ui, area: Rect, tab: u64, divider: Divider, intents: &mut Vec<Intent>) {
    let palette = style::palette(ui);
    let parent = divider.parent;
    let rect = if divider.row {
        let x = area.left() + divider.position * area.width();
        Rect::from_min_max(Pos2::new(x - GAP / 2.0, area.top() + parent.y * area.height()), Pos2::new(x + GAP / 2.0, area.top() + (parent.y + parent.h) * area.height()))
    } else {
        let y = area.top() + divider.position * area.height();
        Rect::from_min_max(Pos2::new(area.left() + parent.x * area.width(), y - GAP / 2.0), Pos2::new(area.left() + (parent.x + parent.w) * area.width(), y + GAP / 2.0))
    };
    let response = ui.interact(rect, Id::unique(("divider", tab, &divider.path)), Sense::click_and_drag());
    let response = response.on_hover_cursor(if divider.row { CursorIcon::ResizeHorizontal } else { CursorIcon::ResizeVertical }).on_hover_text("拖动调整大小，双击平均分配");
    if response.dragged()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let ratio = if divider.row { ((pointer.x - area.left()) / area.width() - parent.x) / parent.w } else { ((pointer.y - area.top()) / area.height() - parent.y) / parent.h };
        intents.push(Intent::SetRatio { tab, path: divider.path.clone(), ratio });
    }
    if response.double_clicked() {
        intents.push(Intent::SetRatio { tab, path: divider.path.clone(), ratio: 0.5 });
    }
    if response.hovered() || response.dragged() {
        ui.painter().rect_filled(rect.shrink(1.0), 1.0, alpha(palette.tokens.accent, 0.5));
    }
}

#[allow(clippy::too_many_arguments)]
fn pane(app: &mut App, ui: &mut Ui, id: &str, rect: Rect, focused: bool, many: bool, uploads: &Arc<Mutex<Vec<Upload>>>, intents: &mut Vec<Intent>, modal: bool) {
    let palette = style::palette(ui);
    let scale = app.scale;
    let cell = app.fonts.cell;
    let size_px = [(rect.width() * scale).round(), (rect.height() * scale).round()];
    let (cols, rows) = grid::grid_size(size_px, cell);
    let Some(pane) = app.panes.get(id) else { return };
    let terminal = pane.session.terminal.clone();
    let key = pane.key;
    let running = pane.running();
    let powershell = pane.session.powershell;
    app.backend.resize(&terminal, Size { cols, rows });

    let widget = Id::unique(("pane", key));
    let response = ui.interact(rect, widget, Sense::click_and_drag());
    let cell_at = |position: Pos2| {
        let x = ((position.x - rect.left()) * scale - PADDING) / cell.width;
        let y = ((position.y - rect.top()) * scale - PADDING) / cell.height;
        let side = if x.fract() < 0.5 { Side::Left } else { Side::Right };
        (x.max(0.0) as usize, (y.max(0.0) as usize).min(rows.saturating_sub(1)), side)
    };
    let mode = terminal.mode();
    let modifiers = ui.input(|input| input.modifiers);
    // Programs that track the mouse get it, unless Shift is held to select instead.
    let reporting = mode.intersects(TermMode::MOUSE_MODE) && !modifiers.shift && running;
    if response.clicked() || response.drag_started() || response.secondary_clicked() {
        if !focused {
            intents.push(Intent::Focus(id.to_string()));
        }
        response.request_focus();
    }
    if reporting {
        mouse(app, ui, &response, id, mode, cell_at, modifiers);
    } else {
        let command = if app.mac { modifiers.mac_cmd } else { modifiers.ctrl };
        if response.clicked() && command
            && let Some(position) = response.interact_pointer_pos()
        {
            let (column, row, _) = cell_at(position);
            if let Some(url) = terminal.link_at(column, row) {
                open_link(&url, intents);
            }
        } else if response.triple_clicked()
            && let Some(position) = response.interact_pointer_pos()
        {
            let (column, row, side) = cell_at(position);
            terminal.start_selection(column, row, side, SelectionType::Lines);
            copy_on_select(app, ui, &terminal);
        } else if response.double_clicked()
            && let Some(position) = response.interact_pointer_pos()
        {
            let (column, row, side) = cell_at(position);
            terminal.start_selection(column, row, side, SelectionType::Semantic);
            copy_on_select(app, ui, &terminal);
        } else if response.drag_started_by(PointerButton::Primary)
            && let Some(position) = response.interact_pointer_pos()
        {
            let (column, row, side) = cell_at(position);
            terminal.start_selection(column, row, side, if modifiers.alt { SelectionType::Block } else { SelectionType::Simple });
            app.selecting = Some(id.to_string());
        } else if response.dragged()
            && app.selecting.as_deref() == Some(id)
            && let Some(position) = response.interact_pointer_pos()
        {
            let (column, row, side) = cell_at(position);
            terminal.extend_selection(column, row, side);
            // Dragging past the edge scrolls the history.
            if position.y < rect.top() {
                terminal.scroll(1);
            } else if position.y > rect.bottom() {
                terminal.scroll(-1);
            }
        } else if response.clicked() {
            terminal.clear_selection();
        }
        if response.drag_stopped() && app.selecting.as_deref() == Some(id) {
            app.selecting = None;
            copy_on_select(app, ui, &terminal);
        }
        if response.hovered() {
            let command = if app.mac { modifiers.mac_cmd } else { modifiers.ctrl };
            ui.ctx().set_cursor_icon(if command { CursorIcon::PointingHand } else { CursorIcon::Text });
            let delta = ui.input(|input| input.smooth_scroll_delta.y);
            let lines = (delta * scale / cell.height).round() as i32;
            if lines != 0 {
                if mode.contains(TermMode::ALT_SCREEN) && mode.contains(TermMode::ALTERNATE_SCROLL) {
                    let key = if lines > 0 { "\x1bOA" } else { "\x1bOB" };
                    app.backend.write(id, key.repeat(lines.unsigned_abs() as usize));
                } else {
                    terminal.scroll(lines);
                }
            }
        }
    }
    response.context_menu(|ui| {
        if ui.button(format!("{}  复制", icons::COPY)).clicked() {
            intents.push(Intent::Copy);
        }
        if ui.button(format!("{}  粘贴", icons::CLIPBOARD_TEXT)).clicked() {
            intents.push(Intent::Paste);
        }
        if ui.button("全选").clicked() {
            intents.push(Intent::SelectAll);
        }
        ui.separator();
        if ui.button(format!("{}  搜索", icons::MAGNIFYING_GLASS)).clicked() {
            intents.push(Intent::Action(Action::Find));
        }
        if ui.button(format!("{}  清空屏幕", icons::TRASH)).clicked() {
            intents.push(Intent::Action(Action::Clear));
        }
    });

    // Files dropped on a terminal are typed as quoted paths, as in the desktop app.
    let dropped = if running && !modal { app.file_drop.take(ui, rect) } else { Vec::new() };
    if !dropped.is_empty() {
        let text = dropped.iter().map(|path| quote_path(&path.to_string_lossy(), app.windows, powershell)).collect::<Vec<_>>().join(" ") + " ";
        app.backend.write(id, text);
        intents.push(Intent::Focus(id.to_string()));
    }
    if running && !modal && app.file_drop.hovered(ui, rect) {
        ui.painter().rect(rect.shrink(6.0), 6.0, alpha(palette.tokens.accent, 0.07), Stroke::new(1.5, alpha(palette.tokens.accent, 0.56)), egui::StrokeKind::Inside);
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, "拖放以插入文件路径", FontId::proportional(12.0), palette.text_soft());
    }

    // The focused pane owns the keyboard while nothing else does.
    let keyboard = focused && !modal && (!app.search.open || response.has_focus());
    if keyboard {
        if !response.has_focus() && ui.ctx().memory(|memory| memory.focused().is_none()) {
            response.request_focus();
        }
        if response.has_focus() {
            ui.memory_mut(|memory| memory.set_focus_lock_filter(widget, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true }));
            type_keys(app, ui);
        }
    }

    // Quads are rebuilt only when something they depend on changed.
    let blink = app.settings.cursor_blink && focused && response.has_focus();
    let cursor_on = !blink || (app.blink_epoch.elapsed().as_millis() / BLINK.as_millis()).is_multiple_of(2);
    if blink {
        let phase = BLINK.as_millis() - app.blink_epoch.elapsed().as_millis() % BLINK.as_millis();
        ui.ctx().request_repaint_after(Duration::from_millis(phase as u64));
    }
    let dim = many && app.settings.dim_inactive && !focused;
    let scene = app.scene();
    let alpha_bits = if scene.clear { 0.0f32 } else { 1.0 };
    let theme = app.theme();
    let pane = app.panes.get_mut(id).expect("pane exists");
    let built = super::BuildKey {
        generation: terminal.generation.load(std::sync::atomic::Ordering::Acquire),
        size: [size_px[0] as u32, size_px[1] as u32],
        focused: focused && response.has_focus(),
        dim,
        cursor_on,
        theme,
        cell: (cell.width.to_bits(), cell.height.to_bits()),
        atlas: app.fonts.epoch,
        alpha: alpha_bits.to_bits(),
    };
    if pane.built != built {
        let frame = grid::Frame { theme, size: size_px, focused: built.focused, background_alpha: alpha_bits, dim, cursor_on };
        pane.quads = Arc::new(grid::build(&terminal.lock(), &mut app.fonts, &frame));
        pane.built = built;
        pane.version += 1;
    }
    let paint = GridPaint { pane: pane.key, size: size_px, instances: pane.quads.clone(), version: pane.version, uploads: uploads.clone() };
    ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, paint));
    if many && focused {
        ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.0, alpha(palette.tokens.accent, 0.33)), egui::StrokeKind::Inside);
    }
    // The grip that moves this pane, shown while the pointer is over it.
    let grip = Rect::from_min_size(Pos2::new(rect.right() - 26.0, rect.top() + 4.0), Vec2::splat(20.0));
    let grip_response = ui.interact(grip, widget.with("grip"), Sense::drag());
    if response.hovered() || grip_response.hovered() || grip_response.dragged() {
        ui.painter().rect(grip, 4.0, palette.surface(), Stroke::new(1.0, palette.border), egui::StrokeKind::Inside);
        ui.painter().text(grip.center(), Align2::CENTER_CENTER, icons::DOTS_SIX_VERTICAL, FontId::new(13.0, egui::FontFamily::Name(style::ICONS.into())), palette.muted());
    }
    let grip_response = grip_response.on_hover_cursor(CursorIcon::Grab).on_hover_text("拖动以移动这个窗格：放到另一个窗格的边缘，或拖到标签栏成为新标签");
    if grip_response.drag_started() {
        app.dragging_pane = Some(id.to_string());
    }

    // The input method composes at the cursor; what it composes shows there until committed.
    if focused && response.has_focus() {
        let (column, row) = {
            let term = terminal.lock();
            let point = term.grid().cursor.point;
            (point.column.0 as f32, (point.line.0 + term.grid().display_offset() as i32) as f32)
        };
        let cursor = rect.min + Vec2::new((PADDING + column * cell.width) / scale, (PADDING + row * cell.height) / scale);
        let cursor_rect = Rect::from_min_size(cursor, Vec2::new(1.0, cell.height / scale));
        ui.ctx().output_mut(|output| output.ime = Some(egui::output::IMEOutput { purpose: egui::IMEPurpose::Terminal, rect, cursor_rect, should_interrupt_composition: false }));
        if !app.preedit.is_empty() {
            let galley = ui.painter().layout_no_wrap(app.preedit.clone(), FontId::monospace(app.settings.font_size), style::color(theme.foreground));
            let backdrop = Rect::from_min_size(cursor, galley.size()).expand(2.0);
            ui.painter().rect_filled(backdrop, 2.0, style::color(theme.background));
            ui.painter().galley(cursor, galley, style::color(theme.foreground));
            ui.painter().hline(backdrop.x_range(), backdrop.bottom(), Stroke::new(1.0, style::color(theme.cursor)));
        }
    }
}

fn copy_on_select(app: &App, ui: &Ui, terminal: &crate::terminal::Terminal) {
    if app.settings.copy_on_select
        && let Some(text) = terminal.selection_text()
    {
        ui.ctx().copy_text(text);
    }
}

fn open_link(url: &str, intents: &mut Vec<Intent>) {
    if !(url.starts_with("http://") || url.starts_with("https://")) || url.len() > 8192 {
        intents.push(Intent::Notify("只能打开 http 和 https 链接。".into(), true));
        return;
    }
    // Arguments, never a shell: the URL cannot become a command.
    let result = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else if cfg!(windows) {
        std::process::Command::new("rundll32.exe").args(["url.dll,FileProtocolHandler", url]).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    if let Err(error) = result {
        intents.push(Intent::Notify(format!("无法打开链接：{error}"), true));
    }
}

/// Mouse events for a program that asked for them.
fn mouse(app: &App, ui: &Ui, response: &egui::Response, id: &str, mode: TermMode, cell_at: impl Fn(Pos2) -> (usize, usize, Side), modifiers: Modifiers) {
    let held = (modifiers.shift, modifiers.alt, modifiers.ctrl);
    let send = |text: Option<String>| {
        if let Some(text) = text {
            app.backend.write_reply(id, text);
        }
    };
    let pointer = ui.input(|input| input.pointer.hover_pos());
    let Some(position) = pointer.filter(|position| response.rect.contains(*position)).or_else(|| response.interact_pointer_pos()) else { return };
    let (column, row, _) = cell_at(position);
    ui.input(|input| {
        for event in &input.events {
            match event {
                Event::PointerButton { button, pressed, .. } if response.hovered() || !*pressed => {
                    let code = match button {
                        PointerButton::Primary => 0,
                        PointerButton::Middle => 1,
                        PointerButton::Secondary => 2,
                        _ => continue,
                    };
                    send(mouse_report(mode, code, column, row, *pressed, false, held));
                }
                Event::PointerMoved(_) if response.dragged() => {
                    let code = if input.pointer.primary_down() { 0 } else if input.pointer.middle_down() { 1 } else { 2 };
                    send(mouse_report(mode, code, column, row, true, true, held));
                }
                Event::MouseWheel { delta, .. } if response.hovered() && delta.y != 0.0 => {
                    send(mouse_report(mode, if delta.y > 0.0 { 64 } else { 65 }, column, row, true, false, held));
                }
                _ => {}
            }
        }
    });
}

/// Keys, text, clipboard and the input method, for the focused terminal.
fn type_keys(app: &mut App, ui: &Ui) {
    let (events, modifiers) = ui.input(|input| (input.events.clone(), input.modifiers));
    let mac = app.mac;
    let meta = mac && app.settings.option_as_meta;
    // Alt alone is Meta for the terminal; Ctrl+Alt is AltGr on Windows and types text. On
    // macOS Option types characters, unless it is set to act as Meta.
    let swallow_text = |m: Modifiers| if mac { m.ctrl || (meta && m.alt) } else { m.ctrl != m.alt };
    let mut handled = false;
    let focused_mode = app.focused().map(|pane| pane.session.terminal.mode()).unwrap_or_default();
    for event in events {
        match event {
            Event::Key { key, pressed: true, modifiers, .. } => {
                if mac && modifiers.mac_cmd {
                    continue;
                }
                let terminal_modifiers = if mac && !meta { Modifiers { alt: false, ..modifiers } } else { modifiers };
                if let Some(bytes) = key_bytes(key, terminal_modifiers, focused_mode.contains(TermMode::APP_CURSOR)) {
                    handled = true;
                    app.type_text(bytes);
                }
            }
            Event::Text(text) if !(handled && swallow_text(modifiers)) && app.preedit.is_empty() => app.type_text(text),
            // Ctrl+C copies a selection and otherwise interrupts, like Windows Terminal;
            // Ctrl+Shift+C and Command+C only ever copy.
            Event::Copy => {
                let copied = app.focused().and_then(|pane| pane.session.terminal.selection_text()).map(|text| ui.ctx().copy_text(text)).is_some();
                if !copied && !mac && !modifiers.shift {
                    app.type_text("\x03".into());
                }
            }
            Event::Cut if !mac => app.type_text("\x18".into()),
            Event::Paste(text) => app.paste(&text),
            Event::Ime(egui::ImeEvent::Preedit { text, .. }) => app.preedit = text,
            Event::Ime(egui::ImeEvent::Commit(text)) => {
                app.preedit.clear();
                app.type_text(text);
            }
            _ => {}
        }
    }
}

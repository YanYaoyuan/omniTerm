// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! The window around the workspace: backdrop, title bar with its own window controls on
//! Windows and Linux (macOS keeps its traffic lights), status bar and toasts.

use super::style::{self, alpha, color, icon, icon_button};
use super::{App, Intent, shortcuts::Action};
use crate::core::settings::{ImageFit, Scene};
use crate::core::text::basename;
use crate::icons;
use crate::theme::THEMES;
use eframe::egui::{self, Align, Align2, Color32, CursorIcon, FontId, Layout, Pos2, Rect, ResizeDirection, RichText, Sense, Stroke, Ui, Vec2, ViewportCommand};

pub const TITLE_HEIGHT: f32 = 36.0;
pub const STATUS_HEIGHT: f32 = 22.0;
const RESIZE_EDGE: f32 = 5.0;
const MAC_TRAFFIC_LIGHTS: f32 = 83.0;

/// The theme color at the scene's base opacity, then the background image at its strength.
pub fn backdrop(app: &mut App, ui: &mut Ui, scene: Scene) {
    let rect = ui.max_rect();
    let theme = app.theme();
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, alpha(theme.background, scene.base));
    let Some(image) = app.settings.background_image.clone() else { return };
    if app.background.as_ref().is_none_or(|(path, _)| *path != image.path) {
        let texture = std::fs::read(&image.path).ok().and_then(|bytes| image::load_from_memory(&bytes).ok()).map(|decoded| {
            let rgba = decoded.to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            ui.ctx().load_texture("omniterm-background", egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()), egui::TextureOptions::LINEAR)
        });
        match texture {
            Some(texture) => app.background = Some((image.path.clone(), texture)),
            None => {
                app.settings.background_image = None;
                return;
            }
        }
    }
    let Some((_, texture)) = &app.background else { return };
    let tint = Color32::from_white_alpha((app.settings.background_image_opacity * 255.0) as u8);
    let size = texture.size_vec2();
    let painter = ui.painter_at(rect);
    match app.settings.background_image_fit {
        ImageFit::Cover | ImageFit::Contain => {
            let scale = if app.settings.background_image_fit == ImageFit::Cover { (rect.width() / size.x).max(rect.height() / size.y) } else { (rect.width() / size.x).min(rect.height() / size.y) };
            let target = Rect::from_center_size(rect.center(), size * scale);
            painter.image(texture.id(), target, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), tint);
        }
        ImageFit::Tile => {
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(rect.width() / size.x, rect.height() / size.y));
            painter.add(egui::Shape::Mesh(std::sync::Arc::new({
                let mut mesh = egui::Mesh::with_texture(texture.id());
                mesh.add_rect_with_uv(rect, uv, tint);
                mesh
            })));
        }
    }
}

pub fn title_bar(app: &mut App, ui: &mut Ui, intents: &mut Vec<Intent>) {
    let palette = style::palette(ui);
    egui::Panel::top("titlebar").exact_size(TITLE_HEIGHT).frame(egui::Frame::new().fill(palette.panel)).show(ui, |ui| {
        let rect = ui.max_rect();
        ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, palette.border));
        // The empty title bar moves the window; a double click maximizes it.
        let drag = ui.interact(rect, ui.scope_id().with("drag"), Sense::click_and_drag());
        if drag.drag_started() {
            intents.push(Intent::Window(ViewportCommand::StartDrag));
        }
        if drag.double_clicked() {
            let maximized = ui.ctx().input(|input| input.viewport().maximized.unwrap_or(false));
            intents.push(Intent::Window(ViewportCommand::Maximized(!maximized)));
        }
        let left = rect.left() + if app.mac { MAC_TRAFFIC_LIGHTS } else { 17.0 };
        let center_y = rect.center().y;
        ui.painter().text(Pos2::new(left, center_y), Align2::LEFT_CENTER, icons::TERMINAL_WINDOW, FontId::new(19.0, egui::FontFamily::Name(style::ICONS.into())), palette.accent());
        ui.painter().text(Pos2::new(left + 27.0, center_y), Align2::LEFT_CENTER, "omniTerm", FontId::proportional(13.0), palette.text_soft());
        let brand_rect = Rect::from_min_size(Pos2::new(left - 5.0, rect.top() + 3.0), Vec2::new(120.0, TITLE_HEIGHT - 6.0));
        let brand = ui.interact(brand_rect, ui.scope_id().with("byte-brand"), Sense::click_and_drag()).on_hover_text("omniTerm · 连点品牌，藏着一点小惊喜");
        if brand.drag_started() { intents.push(Intent::Window(ViewportCommand::StartDrag)); }
        if brand.clicked() { app.easter_eggs.tap_brand(ui.input(|input| input.time)); }
        let controls = if app.mac { 0.0 } else { 138.0 };
        let actions_right = rect.right() - controls;
        // Keep the command centre bounded between the brand and layout controls,
        // including at the minimum supported window size.
        let width = (rect.width() * 0.38).clamp(180.0, 560.0);
        let center = Rect::from_center_size(Pos2::new(rect.center().x - 35.0, center_y), Vec2::new(width, 25.0));
        let response = ui.interact(center, ui.scope_id().with("command-center"), Sense::click()).on_hover_text("搜索命令、连接和设置 · Ctrl+Shift+P");
        ui.painter().rect(center, 5.0, if response.hovered() { palette.hover() } else { alpha(palette.tokens.ink, 0.035) }, Stroke::new(1.0, palette.border_strong), egui::StrokeKind::Inside);
        let directory = app.focused().map(|pane| pane.cwd().to_string()).unwrap_or_else(|| app.cwd.clone());
        ui.painter().text(center.left_center() + Vec2::new(12.0, 0.0), Align2::LEFT_CENTER, icons::COMMAND, FontId::new(13.0, egui::FontFamily::Name(style::ICONS.into())), palette.muted());
        let mut job = egui::text::LayoutJob::simple_singleline(format!("{}  —  搜索命令", basename(&directory)), FontId::proportional(12.0), palette.text_soft());
        job.wrap = egui::text::TextWrapping::truncate_at_width(center.width() - 65.0);
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(Pos2::new(center.center().x - galley.size().x / 2.0, center_y - galley.size().y / 2.0), galley, palette.text_soft());
        if response.clicked() { intents.push(Intent::Action(Action::Palette)); }
        ui.scope_builder(egui::UiBuilder::new().max_rect(Rect::from_min_max(Pos2::new(center.right() + 12.0, rect.top()), Pos2::new(actions_right - 8.0, rect.bottom()))).layout(Layout::right_to_left(Align::Center)), |ui| {
            if icon_button(ui, icons::ROBOT, "切换 Agent 面板", true).clicked() { intents.push(Intent::Action(Action::ToggleAgent)); }
            if icon_button(ui, icons::SIDEBAR_SIMPLE, "切换资源管理器", true).clicked() { intents.push(Intent::Action(Action::ToggleSidebar)); }
            theme_menu(app, ui);
        });
        if !app.mac {
            window_controls(ui, Rect::from_min_max(Pos2::new(actions_right, rect.top()), rect.right_bottom()), intents);
        }
    });
    if !app.mac {
        resize_edges(ui, intents);
    }
}

pub(super) fn theme_menu(app: &mut App, ui: &mut Ui) {
    let palette = style::palette(ui);
    let response = icon_button(ui, icons::PALETTE, "颜色主题", true).on_hover_text("切换主题");
    egui::Popup::menu(&response).show(|ui| {
        ui.set_min_width(190.0);
        for theme in THEMES.iter().filter(|theme| theme.id.starts_with("code")).chain(THEMES.iter().filter(|theme| !theme.id.starts_with("code"))) {
            let selected = app.settings.theme == theme.id;
            let row = ui.horizontal(|ui| {
                let (dot, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                ui.painter().circle(dot.center(), 5.5, color(theme.background), Stroke::new(1.0, palette.border_strong));
                ui.add(egui::Button::selectable(selected, theme.name).frame_when_inactive(false))
            });
            if row.inner.clicked() {
                app.settings.theme = theme.id.to_string();
                ui.close();
            }
        }
    });
}

/// Compact navigation with real workbench actions, independent of panel docking.
pub fn activity_bar(app: &mut App, ui: &mut Ui, intents: &mut Vec<Intent>) {
    let palette = style::palette(ui);
    let has_files = app.focused_remote().is_some() || app.focused().is_some_and(|pane| pane.session.kind == crate::core::profile::Kind::Ssh);
    egui::Panel::left("activity-bar").exact_size(48.0).resizable(false).frame(egui::Frame::new().fill(palette.panel)).show(ui, |ui| {
        let rect = ui.max_rect();
        ui.painter().vline(rect.right() - 0.5, rect.y_range(), Stroke::new(1.0, palette.border));
        ui.spacing_mut().item_spacing.y = 2.0;
        if activity(ui, icons::HARD_DRIVES, "连接与工作区", app.sidebar_visible && (!has_files || !app.sidebar_files)).clicked() {
            if app.sidebar_visible && (!has_files || !app.sidebar_files) { app.sidebar_visible = false; }
            else { app.sidebar_visible = true; app.sidebar_files = false; }
        }
        if activity(ui, icons::FOLDER_OPEN, "远程文件 · 先选择 SSH / SFTP 会话", app.sidebar_visible && has_files && app.sidebar_files).clicked() {
            if !has_files { intents.push(Intent::Notify("先选择一个 SSH 或 SFTP 会话，即可浏览它的远程文件。".into(), false)); }
            else if app.sidebar_visible && app.sidebar_files { app.sidebar_visible = false; }
            else { app.sidebar_visible = true; app.sidebar_files = true; }
        }
        if activity(ui, icons::ROBOT, "omniTerm Agent", app.agent_visible).clicked() { intents.push(Intent::Action(Action::ToggleAgent)); }
        if activity(ui, icons::STACK, "监控面板", app.monitor_visible).clicked() { app.monitor_visible = !app.monitor_visible; }
        if activity(ui, icons::CPU, "进程监视器", app.processes.open).clicked() { app.open_processes(); }
        ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
            ui.add_space(6.0);
            if activity(ui, icons::GEAR_SIX, "设置", false).clicked() { intents.push(Intent::Action(Action::Settings)); }
            theme_menu(app, ui);
        });
    });
}

fn activity(ui: &mut Ui, glyph: &str, label: &str, active: bool) -> egui::Response {
    let palette = style::palette(ui);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(48.0, 46.0), Sense::click());
    if response.hovered() { ui.painter().rect_filled(rect.shrink2(Vec2::new(5.0, 3.0)), 4.0, palette.hover()); }
    if active { ui.painter().vline(rect.left() + 1.0, rect.y_range().shrink(5.0), Stroke::new(2.0, palette.accent())); }
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::new(23.0, egui::FontFamily::Name(style::ICONS.into())), if active { palette.text() } else { palette.muted() });
    response.on_hover_text(label)
}

/// Minimize, maximize and close, drawn in the theme's colors: Windows and Linux windows have
/// no system title bar here.
fn window_controls(ui: &mut Ui, rect: Rect, intents: &mut Vec<Intent>) {
    let palette = style::palette(ui);
    let maximized = ui.ctx().input(|input| input.viewport().maximized.unwrap_or(false));
    let width = rect.width() / 3.0;
    let buttons = [
        (icons::MINUS, "最小化", ViewportCommand::Minimized(true), false),
        (if maximized { icons::CORNERS_IN } else { icons::SQUARE }, if maximized { "还原" } else { "最大化" }, ViewportCommand::Maximized(!maximized), false),
        (icons::X, "关闭", ViewportCommand::Close, true),
    ];
    for (index, (glyph, label, command, close)) in buttons.into_iter().enumerate() {
        let button = Rect::from_min_size(Pos2::new(rect.left() + width * index as f32, rect.top()), Vec2::new(width, rect.height()));
        let response = ui.interact(button, ui.scope_id().with(("window", index)), Sense::click()).on_hover_text(label);
        if response.hovered() {
            ui.painter().rect_filled(button, 0.0, if close { Color32::from_rgb(0xc4, 0x2b, 0x1c) } else { palette.hover() });
        }
        let fg = if response.hovered() && close { Color32::WHITE } else { palette.text_soft() };
        ui.painter().text(button.center(), Align2::CENTER_CENTER, glyph, FontId::new(14.0, egui::FontFamily::Name(style::ICONS.into())), fg);
        if response.clicked() {
            intents.push(Intent::Window(command));
        }
    }
}

/// Undecorated windows resize from their edges and corners.
fn resize_edges(ui: &mut Ui, intents: &mut Vec<Intent>) {
    let ctx = ui.ctx();
    if ctx.input(|input| input.viewport().maximized.unwrap_or(false) || input.viewport().fullscreen.unwrap_or(false)) {
        return;
    }
    let screen = ctx.content_rect();
    let Some(pointer) = ctx.input(|input| input.pointer.hover_pos()) else { return };
    let (left, right) = (pointer.x - screen.left() < RESIZE_EDGE, screen.right() - pointer.x < RESIZE_EDGE);
    let (top, bottom) = (pointer.y - screen.top() < RESIZE_EDGE, screen.bottom() - pointer.y < RESIZE_EDGE);
    let direction = match (left, right, top, bottom) {
        (true, _, true, _) => (ResizeDirection::NorthWest, CursorIcon::ResizeNorthWest),
        (_, true, true, _) => (ResizeDirection::NorthEast, CursorIcon::ResizeNorthEast),
        (true, _, _, true) => (ResizeDirection::SouthWest, CursorIcon::ResizeSouthWest),
        (_, true, _, true) => (ResizeDirection::SouthEast, CursorIcon::ResizeSouthEast),
        (true, ..) => (ResizeDirection::West, CursorIcon::ResizeWest),
        (_, true, ..) => (ResizeDirection::East, CursorIcon::ResizeEast),
        (_, _, true, _) => (ResizeDirection::North, CursorIcon::ResizeNorth),
        (_, _, _, true) => (ResizeDirection::South, CursorIcon::ResizeSouth),
        _ => return,
    };
    ctx.set_cursor_icon(direction.1);
    if ctx.input(|input| input.pointer.primary_pressed()) {
        intents.push(Intent::Window(ViewportCommand::BeginResize(direction.0)));
    }
}

pub fn status_bar(app: &mut App, ui: &mut Ui, intents: &mut Vec<Intent>) {
    let palette = style::palette(ui);
    egui::Panel::bottom("statusbar").exact_size(STATUS_HEIGHT).frame(egui::Frame::new().fill(palette.panel).inner_margin(egui::Margin::symmetric(13, 0))).show(ui, |ui| {
        let rect = ui.max_rect();
        ui.painter().hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, palette.border_strong));
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 7.0;
            let small = |text: &str| RichText::new(text).size(9.5).color(palette.muted());
            ui.label(icon(icons::MONITOR, 13.0).color(palette.muted()));
            ui.label(small(platform_name()));
            separator(ui);
            status_dot(ui, true);
            let running = app.panes.values().filter(|pane| pane.running()).count();
            ui.label(small(&format!("{running} 个活动会话")));
            if app.active_tab().is_some_and(|tab| tab.broadcast) {
                separator(ui);
                ui.label(icon(icons::BROADCAST, 12.0).color(palette.muted()));
                ui.label(small("广播输入中"));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let key = super::shortcuts::label(Action::Palette, app.mac);
                if ui.add(egui::Button::new(RichText::new(format!("{}  {key}", icons::COMMAND)).size(9.5).color(palette.muted())).frame(false)).on_hover_text("命令面板").clicked() {
                    intents.push(Intent::Action(Action::Palette));
                }
                separator(ui);
                let ready = crate::agent::engine::resolve().is_ok();
                if app.easter_eggs.discovered {
                    super::easter_eggs::badge(&mut app.easter_eggs, ui);
                    separator(ui);
                }
                ui.label(small(if ready { "omniTerm Agent 已就绪" } else { "omniTerm Agent 未就绪" }));
                status_dot(ui, ready);
            });
        });
    });
}

fn separator(ui: &mut Ui) {
    let palette = style::palette(ui);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(1.0, 11.0), Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), Stroke::new(1.0, palette.border_strong));
}

pub fn status_dot(ui: &mut Ui, on: bool) {
    let palette = style::palette(ui);
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(7.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.0, if on { color(palette.tokens.accent) } else { palette.faint() });
}

fn platform_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(windows) {
        "Windows"
    } else {
        "Linux"
    }
}

pub fn toast(app: &mut App, ctx: &egui::Context) {
    let Some(toast) = &app.toast else { return };
    if std::time::Instant::now() >= toast.until {
        app.toast = None;
        return;
    }
    ctx.request_repaint_after(toast.until - std::time::Instant::now());
    let palette = style::palette_of(ctx);
    let mut close = false;
    egui::Area::new(egui::Id::unique("toast")).anchor(Align2::RIGHT_BOTTOM, Vec2::new(-18.0, -40.0)).order(egui::Order::Foreground).show(ctx, |ui| {
        egui::Frame::new().fill(palette.surface()).stroke(Stroke::new(1.0, if toast.error { alpha(palette.tokens.danger, 0.4) } else { palette.border_strong })).corner_radius(8).inner_margin(egui::Margin::symmetric(12, 9)).shadow(egui::epaint::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(60) }).show(ui, |ui| {
            ui.set_max_width(420.0);
            ui.horizontal(|ui| {
                let (glyph, tint) = if toast.error { (icons::WARNING_CIRCLE, palette.danger()) } else { (icons::CHECK, palette.accent()) };
                ui.label(icon(glyph, 18.0).color(tint));
                ui.add(egui::Label::new(RichText::new(&toast.message).size(12.0).color(palette.text_soft())).wrap());
                close = icon_button(ui, icons::X, "关闭提示", true).clicked();
            });
        });
    });
    if close {
        app.toast = None;
    }
}


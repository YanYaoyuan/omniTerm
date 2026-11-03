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

pub const TITLE_HEIGHT: f32 = 43.0;
pub const STATUS_HEIGHT: f32 = 27.0;
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
        let brand_width = if app.sidebar_visible && app.dock.sidebar == super::dock::Edge::Left { app.dock.size(super::dock::Edge::Left) } else { 230.0 };
        let mut left = rect.left() + if app.mac { MAC_TRAFFIC_LIGHTS } else { 22.0 };
        let center_y = rect.center().y;
        ui.painter().text(Pos2::new(left, center_y), Align2::LEFT_CENTER, icons::TERMINAL_WINDOW, FontId::new(19.0, egui::FontFamily::Name(style::ICONS.into())), palette.accent());
        left += 26.0;
        ui.painter().text(Pos2::new(left, center_y), Align2::LEFT_CENTER, "omniTerm", FontId::proportional(13.0), palette.text_soft());

        let controls = if app.mac { 0.0 } else { 138.0 };
        let actions_right = rect.right() - controls;
        let actions = ui.scope_builder(egui::UiBuilder::new().max_rect(Rect::from_min_max(Pos2::new(rect.left() + brand_width, rect.top()), Pos2::new(actions_right - 12.0, rect.bottom()))).layout(Layout::right_to_left(Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let agent_label = format!("{}（{}）", if app.agent_visible { "隐藏 omniTerm Agent 面板（对话继续运行）" } else { "显示 omniTerm Agent 面板" }, super::shortcuts::label(Action::ToggleAgent, app.mac));
            if title_button(ui, icons::ROBOT, "omniTerm Agent", app.agent_visible).on_hover_text(agent_label).clicked() {
                intents.push(Intent::Action(Action::ToggleAgent));
            }
            if title_button(ui, icons::CPU, "进程", false).on_hover_text("打开进程监视窗口").clicked() {
                app.open_processes();
            }
            if title_button(ui, icons::STACK, "监控", app.monitor_visible).on_hover_text("显示或隐藏监控面板").clicked() {
                app.monitor_visible = !app.monitor_visible;
            }
            let palette_key = super::shortcuts::label(Action::Palette, app.mac);
            if title_button(ui, icons::COMMAND, &format!("命令  {palette_key}"), false).on_hover_text(format!("命令面板（{palette_key}）")).clicked() {
                intents.push(Intent::Action(Action::Palette));
            }
            theme_menu(app, ui);
            ui.min_rect().left()
        });
        // The title fills what is left between the brand and the actions, truncated to fit.
        let title_rect = Rect::from_min_max(Pos2::new(rect.left() + brand_width + 8.0, rect.top()), Pos2::new(actions.inner - 8.0, rect.bottom()));
        if title_rect.width() > 60.0 {
            let tab = app.active_tab().map(|tab| super::terminals::tab_label(app, tab)).unwrap_or_else(|| "工作区".into());
            let directory = app.focused().map(|pane| pane.cwd().to_string()).unwrap_or_else(|| app.cwd.clone());
            let text = format!("{}  /  {}", basename(&directory), tab);
            let mut job = egui::text::LayoutJob::simple_singleline(text, FontId::proportional(11.0), palette.muted());
            job.wrap = egui::text::TextWrapping::truncate_at_width(title_rect.width() - 20.0);
            let galley = ui.painter().layout_job(job);
            let start = Pos2::new(title_rect.center().x - (galley.size().x + 18.0) / 2.0, center_y);
            ui.painter().text(start, Align2::LEFT_CENTER, icons::FOLDER_SIMPLE, FontId::new(13.0, egui::FontFamily::Name(style::ICONS.into())), palette.muted());
            ui.painter().galley(Pos2::new(start.x + 18.0, center_y - galley.size().y / 2.0), galley, palette.muted());
        }
        if !app.mac {
            window_controls(ui, Rect::from_min_max(Pos2::new(actions_right, rect.top()), rect.right_bottom()), intents);
        }
    });
    if !app.mac {
        resize_edges(ui, intents);
    }
}

fn title_button(ui: &mut Ui, glyph: &str, label: &str, active: bool) -> egui::Response {
    let palette = style::palette(ui);
    let color = if active { palette.text_soft() } else { palette.muted() };
    let glyph = RichText::new(glyph).family(egui::FontFamily::Name(style::ICONS.into())).size(15.0).color(color);
    let label = RichText::new(label).size(11.0).color(color);
    ui.add(egui::Button::new((glyph, label)).fill(Color32::TRANSPARENT).stroke(Stroke::NONE).corner_radius(5))
}

fn theme_menu(app: &mut App, ui: &mut Ui) {
    let palette = style::palette(ui);
    let response = title_button(ui, icons::PALETTE, "主题", false).on_hover_text("切换主题");
    egui::Popup::menu(&response).show(|ui| {
        ui.set_min_width(190.0);
        for theme in &THEMES {
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


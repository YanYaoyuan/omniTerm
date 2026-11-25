// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! BYTE's little break room: opt-in art, a cat and a local packet snake.
use super::style;
use crate::play::{COLUMNS, Direction, Phase, ROWS, Snake};
use eframe::egui::{self, Align2, Color32, FontId, Id, Key, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq)]
enum Page { Cat, Game }
#[derive(Clone, Copy)]
enum Pose { Rest, Hello, Play }

pub struct State {
    pub open: bool,
    pub discovered: bool,
    pub best: u16,
    pub still: bool,
    page: Page,
    pose: Pose,
    until: f64,
    pats: u32,
    fish: u32,
    taps: u8,
    last_tap: f64,
    last_step: f64,
    release_keys: bool,
    just_opened: bool,
    opening_frame: Option<u64>,
    backdrop_pressed: bool,
    texture: Option<egui::TextureHandle>,
    frames: [Rect; 3],
    texture_tried: bool,
    game: Snake,
    desktop: super::desktop_pet::DesktopPet,
}

impl State {
    pub fn new(discovered: bool, best: u16, still: bool) -> Self {
        Self { open: false, discovered, best, still, page: Page::Cat, pose: Pose::Rest, until: 0.0, pats: 0, fish: 0, taps: 0, last_tap: -10.0, last_step: 0.0, release_keys: false, just_opened: false, opening_frame: None, backdrop_pressed: false, texture: None, frames: [Rect::NOTHING; 3], texture_tried: false, game: Snake::new(42), desktop: Default::default() }
    }
    pub fn reveal(&mut self) { self.discovered = true; self.open = true; self.just_opened = true; self.backdrop_pressed = false; }
    pub fn tap_brand(&mut self, now: f64) {
        self.taps = if now - self.last_tap > 1.4 { 1 } else { self.taps + 1 };
        self.last_tap = now;
        if self.taps >= 5 { self.taps = 0; self.reveal(); }
    }
    pub fn blocks_input(&mut self, ctx: &egui::Context) -> bool {
        if self.release_keys && ctx.input(|i| i.keys_down.is_empty()) { self.release_keys = false; }
        self.open || self.release_keys
    }
    pub fn blocking(&self) -> bool { self.open || self.release_keys }
    fn close(&mut self) { self.open = false; self.game.pause(); self.release_keys = true; }
    fn touch(&mut self, now: f64, food: bool) {
        self.pose = if food { self.fish = self.fish.saturating_add(1); Pose::Play } else { self.pats = self.pats.saturating_add(1); Pose::Hello };
        self.until = now + 3.0;
    }
}

pub fn badge(state: &mut State, ui: &mut egui::Ui) {
    if !state.discovered { return; }
    let (rect, response) = ui.allocate_exact_size(Vec2::new(25.0, 21.0), Sense::click());
    cat_face(ui.painter(), rect.center(), 8.0);
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(if state.desktop.enabled() { "BYTE 在桌面陪你 · 点击打开，右键暂停或收回" } else { "BYTE 在这里 · 点击小憩一下，右键可隐藏入口" });
    if response.clicked() { state.reveal(); }
    response.context_menu(|ui| {
        if state.desktop.enabled() {
            if ui.button(if state.desktop.paused() { "继续桌面追逐" } else { "暂停桌面追逐" }).clicked() { state.desktop.pause(ui.ctx()); ui.close(); }
            if ui.button("收回桌面小猫").clicked() { state.desktop.stop(ui.ctx()); ui.close(); }
            ui.separator();
        }
        if ui.button("隐藏小猫入口").clicked() { state.discovered = false; ui.close(); }
    });
}

fn cat_face(p: &egui::Painter, center: Pos2, radius: f32) {
    let ink = Color32::from_rgb(27, 43, 76);
    let cream = Color32::from_rgb(255, 234, 196);
    for sign in [-1.0, 1.0] {
        p.add(egui::Shape::convex_polygon(vec![center + Vec2::new(sign * radius, -radius * 0.1), center + Vec2::new(sign * radius * 0.8, -radius * 1.15), center + Vec2::new(sign * radius * 0.2, -radius * 0.6)], ink, Stroke::NONE));
    }
    p.circle(center, radius, cream, Stroke::new(1.1, ink));
    for sign in [-1.0, 1.0] { p.circle_filled(center + Vec2::new(sign * radius * 0.36, -radius * 0.07), radius * 0.11, ink); }
    p.line_segment([center + Vec2::new(-radius * 0.16, radius * 0.36), center + Vec2::new(radius * 0.16, radius * 0.36)], Stroke::new(1.0, ink));
}

pub fn show(state: &mut State, ctx: &egui::Context) {
    state.desktop.show(ctx);
    if !state.open { return; }
    let palette = style::palette_of(ctx);
    let now = ctx.input(|i| i.time);
    let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true) && !i.viewport().minimized.unwrap_or(false));
    if !focused || state.page != Page::Game { state.game.pause(); }
    if !state.texture_tried {
        state.texture_tried = true;
        if let Ok(image) = image::load_from_memory(include_bytes!("../../assets/easter-eggs/byte-sprites.png")) {
            let limit = ctx.input(|i| i.max_texture_side).clamp(1, u32::MAX as usize) as u32;
            let image = if image.width() > limit || image.height() > limit {
                image.resize(limit, limit, image::imageops::FilterType::Triangle)
            } else { image };
            let rgba = image.to_rgba8();
            state.frames = atlas_frames(&rgba);
            state.texture = Some(ctx.load_texture("byte-cat", egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()), egui::TextureOptions::LINEAR));
        }
    }
    if std::mem::take(&mut state.just_opened) { state.opening_frame = Some(ctx.cumulative_frame_nr()); }
    let opening = state.opening_frame == Some(ctx.cumulative_frame_nr());
    let mut close = false;
    let response = egui::Modal::new(Id::unique("byte-playroom"))
        .area(egui::Modal::default_area(Id::unique("byte-playroom")).default_size(Vec2::new(500.0, 540.0)))
        .backdrop_color(Color32::from_black_alpha(125))
        .frame(egui::Frame::new().fill(palette.surface()).stroke(Stroke::new(1.0, palette.border_strong)).corner_radius(18).inner_margin(20))
        .show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 72.0).clamp(150.0, 460.0));
            ui.set_height((ctx.content_rect().height() - 100.0).clamp(180.0, 500.0));
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("LITTLE MOMENTS / omniTerm").size(10.0).color(palette.accent()));
                    ui.label(RichText::new("BYTE 的小憩角落").size(21.0).strong().color(palette.text()));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close |= ui.button("关闭 · Esc").clicked();
                });
            });
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut state.page, Page::Cat, "陪小猫一会儿");
                ui.selectable_value(&mut state.page, Page::Game, "数据包贪吃蛇");
            });
            ui.add_space(10.0);
            egui::ScrollArea::vertical().max_height((ctx.content_rect().height() - 210.0).max(120.0)).show(ui, |ui| {
                match state.page {
                    Page::Cat => cat(state, ui, now, focused),
                    Page::Game => snake(state, ui, now, focused),
                }
            });
        });
    // A reopening click belongs to the status-bar button. Only close on a
    // backdrop click whose press started after this modal was already open.
    if !opening && ctx.input(|i| i.pointer.primary_pressed()) {
        state.backdrop_pressed = ctx.input(|i| i.pointer.interact_pos().is_some_and(|p| !response.response.rect.contains(p)));
    }
    let outside = state.backdrop_pressed && response.backdrop_response.clicked();
    if ctx.input(|i| i.pointer.primary_released()) { state.backdrop_pressed = false; }
    let escape = response.is_top_modal && !response.any_popup_open && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
    if close || outside || escape { state.close(); }
}

fn cat(state: &mut State, ui: &mut egui::Ui, now: f64, focused: bool) {
    let palette = style::palette(ui);
    let pose = if now < state.until { state.pose } else { Pose::Rest };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 230.0), Sense::click());
    ui.painter().rect_filled(rect, 14.0, palette.hover());
    if let Some(texture) = &state.texture {
        // Split the atlas in transparent gutters, keeping the leaping cat's motion marks.
        let uv = state.frames[match pose { Pose::Rest => 0, Pose::Hello => 1, Pose::Play => 2 }];
        let ratio = texture.size_vec2().x * uv.width() / (texture.size_vec2().y * uv.height());
        let motion = if state.still || !focused { 0.0 } else { (now as f32 * 2.3).sin() };
        let height = 209.0_f32.min(rect.width() / ratio - 20.0).max(50.0) * (1.0 + motion * 0.009);
        let target = Rect::from_center_size(rect.center() + Vec2::new(0.0, motion * 1.7), Vec2::new(height * ratio, height));
        ui.painter().image(texture.id(), target, uv, Color32::WHITE);
    } else { cat_face(ui.painter(), rect.center(), 55.0); }
    let before = (state.pats, state.fish, state.until);
    if response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("点一下，摸摸 BYTE").clicked() { state.touch(now, false); }
    ui.add_space(10.0);
    ui.vertical_centered(|ui| {
        let message = match pose { Pose::Rest => "你写代码，我来陪班。", Pose::Hello => "收到一份摸摸，信号满格。", Pose::Play => "数据包里，怎么都是小鱼干！" };
        ui.label(RichText::new(message).color(palette.text_soft()));
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button("摸摸脑袋").clicked() { state.touch(now, false); }
            if ui.button("投喂小鱼干").clicked() { state.touch(now, true); }
            if ui.button("让它打个盹").clicked() { state.until = 0.0; }
        });
        ui.add_space(8.0);
        ui.label(RichText::new(format!("本次陪伴  {} 次摸摸 · {} 包小鱼干", state.pats, state.fish)).size(11.0).color(palette.muted()));
        ui.checkbox(&mut state.still, "静态插画（减少动画）");
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            if state.desktop.enabled() {
                let response = ui.button(if state.desktop.paused() { "继续桌面追逐" } else { "暂停桌面追逐" });
                if response.clicked() { state.desktop.pause(ui.ctx()); }
                if ui.button("收回桌面小猫").clicked() { state.desktop.stop(ui.ctx()); }
            } else if ui.button("放到桌面追鼠标").clicked() && let Some(texture) = &state.texture
                && state.desktop.start(ui.ctx(), texture.clone(), state.frames) { state.close(); }
        });
        if let Some(error) = state.desktop.error() { ui.colored_label(palette.accent(), error); }
        else { ui.label(RichText::new("跨应用追逐 · 点击穿透 · 右键状态栏小猫可暂停或收回").size(10.0).color(palette.muted())); }
    });
    if before != (state.pats, state.fish, state.until) { ui.ctx().request_repaint(); }
    if focused && !state.still { ui.ctx().request_repaint_after(Duration::from_millis(40)); }
    else if now < state.until { ui.ctx().request_repaint_after(Duration::from_secs_f64((state.until - now).max(0.01))); }
}

fn snake(state: &mut State, ui: &mut egui::Ui, now: f64, focused: bool) {
    if focused {
        let keys: Vec<_> = ui.input(|i| i.events.iter().filter_map(|e| match e { egui::Event::Key { key, pressed: true, modifiers, repeat, .. } if !modifiers.any() => Some((*key, *repeat)), _ => None }).collect());
        for (key, repeat) in keys {
            if matches!(key, Key::ArrowUp | Key::ArrowDown | Key::ArrowLeft | Key::ArrowRight | Key::W | Key::A | Key::S | Key::D | Key::Space) {
                ui.ctx().input_mut(|i| i.consume_key(egui::Modifiers::NONE, key));
            }
            match key {
                Key::ArrowUp | Key::W => state.game.turn(Direction::Up),
                Key::ArrowDown | Key::S => state.game.turn(Direction::Down),
                Key::ArrowLeft | Key::A => state.game.turn(Direction::Left),
                Key::ArrowRight | Key::D => state.game.turn(Direction::Right),
                Key::Space if !repeat => toggle_game(state, now),
                _ => {}
            }
        }
    }
    let interval = (0.18 - f64::from(state.game.score) * 0.003).max(0.085);
    if focused && state.game.phase == Phase::Running && now - state.last_step >= interval {
        state.game.step(); state.last_step = now; state.best = state.best.max(state.game.score);
    }
    let palette = style::palette(ui);
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("已接收 {} 包", state.game.score)).strong().color(palette.text()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { ui.label(RichText::new(format!("最佳 {}", state.best)).color(palette.muted())); });
    });
    ui.add_space(5.0);
    let unit = ((ui.available_width() - 16.0) / f32::from(COLUMNS)).min(21.5);
    let size = Vec2::new(unit * f32::from(COLUMNS) + 16.0, unit * f32::from(ROWS) + 16.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 12.0, Color32::from_rgb(21, 33, 58));
    let origin = rect.min + Vec2::splat(8.0);
    let tile = |(x, y): (i16, i16)| Rect::from_min_size(origin + Vec2::new(f32::from(x) * unit, f32::from(y) * unit), Vec2::splat(unit)).shrink(1.4);
    for y in 0..ROWS { for x in 0..COLUMNS { p.rect_filled(tile((x, y)), 3.0, Color32::from_rgb(29, 44, 71)); } }
    if let Some(food) = state.game.food { fish(&p, tile(food)); }
    for (index, cell) in state.game.body.iter().enumerate().rev() {
        let tile = tile(*cell);
        p.rect_filled(tile, 4.0, if index == 0 { Color32::from_rgb(108, 208, 255) } else { Color32::from_rgb(57, 147, 244) });
        if index == 0 {
            let vertical = matches!(state.game.direction, Direction::Left | Direction::Right);
            for sign in [-1.0, 1.0] {
                let offset = if vertical { Vec2::new(0.0, sign * unit * 0.17) } else { Vec2::new(sign * unit * 0.17, 0.0) };
                p.circle_filled(tile.center() + offset, (unit * 0.07).max(0.7), Color32::from_rgb(21, 33, 58));
            }
        }
    }
    if state.game.phase != Phase::Running {
        p.rect_filled(rect.shrink(1.0), 12.0, Color32::from_black_alpha(95));
        let text = match state.game.phase { Phase::Ready => "一条小蛇，接住快乐的数据包", Phase::Paused => "暂停一下，连接还在", Phase::Over => "线路打结啦 · 再来一局？", Phase::Won => "满格连接！", Phase::Running => "" };
        p.text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(if rect.width() < 300.0 { 12.0 } else { 16.0 }), Color32::WHITE);
    }
    ui.add_space(9.0);
    ui.horizontal(|ui| {
        let label = match state.game.phase { Phase::Running => "暂停", Phase::Paused => "继续", Phase::Ready => "出发", _ => "再来一局" };
        if ui.button(label).clicked() { toggle_game(state, now); }
        for (text, direction) in [("←", Direction::Left), ("↑", Direction::Up), ("↓", Direction::Down), ("→", Direction::Right)] {
            if ui.button(text).clicked() { state.game.turn(direction); }
        }
    });
    ui.add_space(5.0);
    ui.label(RichText::new("方向键 / WASD 转向 · 空格开始或暂停 · Esc 返回工作").size(11.0).color(palette.muted()));
    ui.label(RichText::new("切走窗口会自动暂停。慢一点，也可以很开心。").size(11.0).color(palette.muted()));
    if focused && state.game.phase == Phase::Running { ui.ctx().request_repaint_after(Duration::from_millis(35)); }
}

fn toggle_game(state: &mut State, now: f64) {
    if state.game.phase == Phase::Running { state.game.pause(); }
    else { state.game.start(); state.last_step = now; }
}

fn fish(p: &egui::Painter, rect: Rect) {
    let center = rect.center(); let r = rect.height() * 0.33;
    let orange = Color32::from_rgb(255, 174, 91);
    p.circle_filled(center, r, orange);
    p.add(egui::Shape::convex_polygon(vec![center + Vec2::new(r * 0.6, 0.0), center + Vec2::new(r * 1.8, -r), center + Vec2::new(r * 1.8, r)], orange, Stroke::NONE));
    p.circle_filled(center - Vec2::new(r * 0.35, r * 0.18), (r * 0.15).max(0.6), Color32::from_rgb(27, 43, 76));
}

fn atlas_frames(image: &image::RgbaImage) -> [Rect; 3] {
    let width = image.width(); let height = image.height();
    [(0.0, 0.335), (0.335, 0.62), (0.62, 1.0)].map(|(left, right)| {
        let start = (left * width as f32) as u32; let end = (right * width as f32) as u32;
        let (mut x0, mut y0, mut x1, mut y1) = (end, height, start, 0);
        for y in 0..height { for x in start..end {
            if image.get_pixel(x, y)[3] >= 16 { x0 = x0.min(x); y0 = y0.min(y); x1 = x1.max(x + 1); y1 = y1.max(y + 1); }
        } }
        if x1 <= x0 || y1 <= y0 { return Rect::from_min_max(Pos2::new(left, 0.0), Pos2::new(right, 1.0)); }
        Rect::from_min_max(Pos2::new(x0.saturating_sub(8).max(start) as f32 / width as f32, y0.saturating_sub(8) as f32 / height as f32), Pos2::new((x1 + 8).min(end) as f32 / width as f32, (y1 + 8).min(height) as f32 / height as f32))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discover_requires_five_nearby_taps_and_closing_blocks_held_keys() {
        let mut state = State::new(false, 0, false);
        for t in [0.0, 0.2, 0.4, 0.6] { state.tap_brand(t); }
        assert!(!state.open);
        state.tap_brand(4.0); assert!(!state.open);
        for t in [4.2, 4.4, 4.6, 4.8] { state.tap_brand(t); }
        assert!(state.open && state.discovered);
        state.close(); assert!(!state.open && state.blocking());
        assert!(!state.blocks_input(&egui::Context::default()));
    }
    #[test]
    fn artwork_has_real_transparency() {
        let image = image::load_from_memory(include_bytes!("../../assets/easter-eggs/byte-sprites.png")).unwrap().to_rgba8();
        assert!(image.width() > image.height() * 2);
        assert_eq!(image.get_pixel(0, 0)[3], 0);
        assert!(image.pixels().any(|p| p[3] == 255));
    }
    #[test]
    fn atlas_respects_the_renderers_texture_limit() {
        let ctx = egui::Context::default();
        let mut state = State::new(true, 0, true); state.open = true;
        ctx.run_ui(egui::RawInput { max_texture_side: Some(1024), ..Default::default() }, |ui| {
            show(&mut state, ui.ctx());
            let size = state.texture.as_ref().unwrap().size();
            assert!(size[0] <= 1024 && size[1] <= 1024);
            assert!(state.frames.iter().all(|frame| frame.is_positive() && frame.is_finite()));
        }).drop_without_applying_deltas();
    }
    #[test]
    fn controls_fit_the_smallest_supported_window_without_scrolling() {
        for (page, expected) in [(Page::Cat, ["投喂小鱼干", "放到桌面追鼠标"]), (Page::Game, ["出发", "方向键 / WASD"])] {
            let ctx = egui::Context::default();
            let mut state = State::new(true, 0, true); state.open = true; state.page = page;
            let mut seen = [false; 2];
            for _ in 0..3 {
                let output = ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 650.0))), ..Default::default() }, |ui| show(&mut state, ui.ctx()));
                for shape in &output.shapes {
                    if let egui::Shape::Text(text) = &shape.shape {
                        for (index, label) in expected.iter().enumerate() {
                            if text.galley.job.text.contains(label) && shape.clip_rect.contains_rect(Rect::from_min_size(text.pos, text.galley.size())) { seen[index] = true; }
                        }
                    }
                }
                output.drop_without_applying_deltas();
            }
            assert!(seen.into_iter().all(|s| s), "some controls were clipped: {expected:?}");
        }
    }
    #[test]
    fn reopening_from_an_outside_button_does_not_close_on_the_opening_click() {
        let ctx = egui::Context::default();
        let mut state = State::new(true, 0, true); state.reveal();
        let raw = || egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 650.0))), ..Default::default() };
        for _ in 0..3 { ctx.run_ui(raw(), |ui| show(&mut state, ui.ctx())).drop_without_applying_deltas(); }
        state.close();
        ctx.run_ui(raw(), |ui| show(&mut state, ui.ctx())).drop_without_applying_deltas();
        let click = || vec![egui::Event::PointerMoved(Pos2::new(894.0, 637.0)), egui::Event::PointerButton { pos: Pos2::new(894.0, 637.0), button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE }, egui::Event::PointerButton { pos: Pos2::new(894.0, 637.0), button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE }];
        state.reveal();
        let mut passes = 0;
        ctx.run_ui(egui::RawInput { events: click(), ..raw() }, |ui| {
            show(&mut state, ui.ctx());
            if passes == 0 { ui.ctx().request_discard("exercise modal opening across layout passes"); }
            passes += 1;
        }).drop_without_applying_deltas();
        assert!(passes >= 2);
        assert!(state.open);
        ctx.run_ui(raw(), |ui| show(&mut state, ui.ctx())).drop_without_applying_deltas();
        ctx.run_ui(egui::RawInput { events: click(), ..raw() }, |ui| show(&mut state, ui.ctx())).drop_without_applying_deltas();
        assert!(!state.open, "later backdrop clicks must still close it");
    }
    #[test]
    fn space_pauses_once_and_closing_waits_for_held_keys() {
        let ctx = egui::Context::default();
        let mut state = State::new(true, 0, true); state.open = true; state.page = Page::Game; state.game.start();
        let event = |pressed, repeat| egui::Event::Key { key: Key::Space, physical_key: Some(Key::Space), pressed, repeat, modifiers: egui::Modifiers::NONE };
        for repeat in [false, true] {
            ctx.run_ui(egui::RawInput { events: vec![event(true, repeat)], ..Default::default() }, |ui| {
                show(&mut state, ui.ctx());
                assert_eq!(state.game.phase, Phase::Paused);
                assert!(!ui.input(|i| i.key_pressed(Key::Space)));
            }).drop_without_applying_deltas();
        }
        state.close(); assert!(state.blocks_input(&ctx));
        ctx.run_ui(egui::RawInput { events: vec![event(false, false)], ..Default::default() }, |ui| {
            assert!(!state.blocks_input(ui.ctx()));
        }).drop_without_applying_deltas();
    }
}

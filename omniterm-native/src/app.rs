// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! The window: tabs of split terminals, drawn on the GPU, with keyboard, mouse, IME and
//! clipboard input. The chrome around the terminals is egui; the terminals are not.

use crate::backend::{Backend, Notice, home_dir, local_shell_plan};
use crate::glyphs::Fonts;
use crate::gpu::{self, GridPaint, Instance};
use crate::grid::{self, PADDING};
use crate::input::{key_bytes, paste_bytes};
use crate::layout::{Divider, Node, Rect as Fraction};
use crate::terminal::{Size, Terminal};
use crate::theme::THEMES;
use alacritty_terminal::index::Side;
use alacritty_terminal::selection::SelectionType;
use alacritty_terminal::term::TermMode;
use eframe::egui::{self, Color32, Event, Id, Key, Modifiers, Pos2, Rect, Sense, Stroke, Vec2, ViewportCommand};
use eframe::egui_wgpu;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const FONT_POINTS: f32 = 13.0;
const LINE_HEIGHT: f32 = 1.22;
const GAP: f32 = 2.0;

struct Pane {
    terminal: Arc<Terminal>,
    key: u64,
    title: Option<String>,
    exit: Option<i64>,
    /// The quads of the last build, and what they were built from.
    quads: Arc<Vec<Instance>>,
    built: (u64, [u32; 2], bool, usize, u32, u64),
    version: u64,
}

struct Tab {
    root: Node,
    focus: String,
    zoomed: bool,
}

/// What `--smoke` checks: a shell starts, runs a command, and its output reaches the screen.
pub struct Smoke {
    started: Instant,
    typed: bool,
    seen: Option<Duration>,
    frames: u64,
    screenshot: Arc<Mutex<Option<Vec<u8>>>>,
    requested: bool,
    pub output: std::path::PathBuf,
}

impl Smoke {
    pub fn new(output: std::path::PathBuf) -> Self {
        Self { started: Instant::now(), typed: false, seen: None, frames: 0, screenshot: Arc::default(), requested: false, output }
    }
}

pub struct App {
    backend: Backend,
    fonts: Fonts,
    font_points: f32,
    scale: f32,
    theme: usize,
    panes: HashMap<String, Pane>,
    tabs: Vec<Tab>,
    active: usize,
    next_key: u64,
    fullscreen: bool,
    render_state: egui_wgpu::RenderState,
    selecting: Option<String>,
    preedit: String,
    error: Option<String>,
    frame_ms: f32,
    smoke: Option<Smoke>,
    mac: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, smoke: Option<Smoke>) -> Self {
        let render_state = cc.wgpu_render_state.clone().expect("omniTerm draws with wgpu");
        gpu::install(&render_state);
        let context = cc.egui_ctx.clone();
        let backend = Backend::new(move || context.request_repaint());
        let scale = cc.egui_ctx.pixels_per_point();
        style(&cc.egui_ctx);
        let mut app = Self {
            backend,
            fonts: Fonts::new(FONT_POINTS * scale, LINE_HEIGHT),
            font_points: FONT_POINTS,
            scale,
            theme: 0,
            panes: HashMap::new(),
            tabs: Vec::new(),
            active: 0,
            next_key: 1,
            fullscreen: false,
            render_state,
            selecting: None,
            preedit: String::new(),
            error: None,
            frame_ms: 0.0,
            smoke,
            mac: cfg!(target_os = "macos"),
        };
        app.new_tab();
        app
    }

    fn open_pane(&mut self) -> Option<String> {
        match self.backend.open(local_shell_plan(&home_dir()), Size { cols: 100, rows: 30 }) {
            Ok(terminal) => {
                let id = terminal.id.clone();
                self.panes.insert(id.clone(), Pane { terminal, key: self.next_key, title: None, exit: None, quads: Arc::default(), built: (0, [0, 0], false, usize::MAX, 0, u64::MAX), version: 0 });
                self.next_key += 1;
                Some(id)
            }
            Err(error) => {
                self.error = Some(format!("无法启动终端：{error}"));
                None
            }
        }
    }

    fn new_tab(&mut self) {
        if let Some(id) = self.open_pane() {
            self.tabs.push(Tab { root: Node::Pane(id.clone()), focus: id, zoomed: false });
            self.active = self.tabs.len() - 1;
        }
    }

    fn split(&mut self, row: bool) {
        let Some(tab) = self.tabs.get(self.active) else { return self.new_tab() };
        let focus = tab.focus.clone();
        if let Some(id) = self.open_pane() {
            let tab = &mut self.tabs[self.active];
            tab.root = tab.root.split(&focus, &id, row);
            tab.focus = id;
            tab.zoomed = false;
        }
    }

    fn close_pane(&mut self, id: &str) {
        self.backend.close(id);
        self.panes.remove(id);
        self.tabs.retain_mut(|tab| match tab.root.remove(id) {
            Some(root) => {
                if tab.focus == id {
                    tab.focus = root.panes()[0].clone();
                }
                tab.zoomed &= root.panes().len() > 1;
                tab.root = root;
                true
            }
            None => false,
        });
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
        let live: Vec<u64> = self.panes.values().map(|pane| pane.key).collect();
        gpu::forget(&self.render_state, &live);
    }

    fn focused(&self) -> Option<&Pane> {
        self.tabs.get(self.active).and_then(|tab| self.panes.get(&tab.focus))
    }

    fn set_font(&mut self, points: f32) {
        self.font_points = points.clamp(8.0, 32.0);
        self.fonts.set_size(self.font_points * self.scale);
    }

    fn toggle_fullscreen(&mut self, ctx: &egui::Context) {
        self.fullscreen = !self.fullscreen;
        ctx.send_viewport_cmd(ViewportCommand::Fullscreen(self.fullscreen));
    }

    fn handle_notices(&mut self) {
        while let Ok(notice) = self.backend.notices.try_recv() {
            match notice {
                Notice::Exited { id, code } => {
                    if let Some(pane) = self.panes.get_mut(&id) {
                        pane.exit = Some(code);
                    }
                }
                Notice::Title { id, title } => {
                    if let Some(pane) = self.panes.get_mut(&id) {
                        pane.title = title;
                    }
                }
            }
        }
    }

    /// App shortcuts. Returns true when the key was the app's and must not reach the terminal.
    fn shortcut(&mut self, ctx: &egui::Context, key: Key, modifiers: Modifiers) -> bool {
        // macOS uses Command; elsewhere Ctrl+Shift, so Ctrl+letters stay with the shell.
        let app = if self.mac { modifiers.mac_cmd } else { modifiers.ctrl && modifiers.shift };
        if key == Key::F11 && !modifiers.any() || (self.mac && modifiers.mac_cmd && modifiers.ctrl && key == Key::F) {
            self.toggle_fullscreen(ctx);
            return true;
        }
        if !app {
            return false;
        }
        match key {
            Key::T => self.new_tab(),
            Key::W => {
                if let Some(id) = self.tabs.get(self.active).map(|tab| tab.focus.clone()) {
                    self.close_pane(&id);
                }
            }
            Key::D | Key::E if !(self.mac && modifiers.shift) => self.split(true),
            Key::D | Key::O => self.split(false),
            Key::Enter => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.zoomed = !tab.zoomed && tab.root.panes().len() > 1;
                }
            }
            Key::C => {
                self.copy(ctx);
            }
            Key::V => self.paste_clipboard(),
            Key::Equals | Key::Plus => self.set_font(self.font_points + 1.0),
            Key::Minus => self.set_font(self.font_points - 1.0),
            Key::Num0 => self.set_font(FONT_POINTS),
            Key::CloseBracket | Key::OpenBracket => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    let ids = tab.root.panes();
                    let at = ids.iter().position(|id| *id == tab.focus).unwrap_or(0);
                    let step = if key == Key::CloseBracket { 1 } else { ids.len() - 1 };
                    tab.focus = ids[(at + step) % ids.len()].clone();
                }
            }
            _ => return false,
        }
        true
    }

    fn copy(&mut self, ctx: &egui::Context) -> bool {
        let Some(pane) = self.focused() else { return false };
        match pane.terminal.selection_text() {
            Some(text) => {
                ctx.copy_text(text);
                true
            }
            None => false,
        }
    }

    fn paste_clipboard(&mut self) {
        let text = arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text());
        match text {
            Ok(text) => self.paste(&text),
            Err(error) => self.error = Some(format!("无法读取剪贴板：{error}")),
        }
    }

    fn paste(&mut self, text: &str) {
        let Some(pane) = self.focused() else { return };
        let bracketed = pane.terminal.mode().contains(TermMode::BRACKETED_PASTE);
        let id = pane.terminal.id.clone();
        pane.terminal.scroll_to_bottom();
        self.backend.write(&id, paste_bytes(text, bracketed));
    }

    fn send(&mut self, data: String) {
        if let Some(pane) = self.focused() {
            pane.terminal.clear_selection();
            pane.terminal.scroll_to_bottom();
            let id = pane.terminal.id.clone();
            self.backend.write(&id, data);
        }
    }

    /// Keyboard, IME and clipboard events for the focused terminal.
    fn keyboard(&mut self, ctx: &egui::Context) {
        let (events, modifiers) = ctx.input(|input| (input.events.clone(), input.modifiers));
        let mac = self.mac;
        // Alt alone is Meta for the terminal; Ctrl+Alt is AltGr on Windows and types text.
        // On macOS Option types characters, as in Terminal.app by default.
        let meta_or_control = |m: Modifiers| if mac { m.ctrl } else { m.ctrl != m.alt };
        let mut handled_key = false;
        for event in events {
            match event {
                Event::Key { key, pressed: true, modifiers, .. } => {
                    if self.shortcut(ctx, key, modifiers) {
                        handled_key = true;
                        continue;
                    }
                    if mac && modifiers.mac_cmd {
                        continue;
                    }
                    let terminal_modifiers = if mac { Modifiers { alt: false, ..modifiers } } else { modifiers };
                    let app_cursor = self.focused().is_some_and(|pane| pane.terminal.mode().contains(TermMode::APP_CURSOR));
                    if let Some(bytes) = key_bytes(key, terminal_modifiers, app_cursor) {
                        handled_key = true;
                        self.send(bytes);
                    }
                }
                Event::Text(text) => {
                    if !(handled_key && meta_or_control(modifiers)) && self.preedit.is_empty() {
                        self.send(text);
                    }
                }
                // Clipboard keys: Ctrl+C copies a selection and otherwise interrupts, like
                // Windows Terminal; Command+C on macOS only ever copies.
                Event::Copy => {
                    if !self.copy(ctx) && !mac && !modifiers.shift {
                        self.send("\x03".into());
                    }
                }
                Event::Cut => {
                    if !mac {
                        self.send("\x18".into());
                    }
                }
                Event::Paste(text) => self.paste(&text),
                Event::Ime(egui::ImeEvent::Preedit { text, .. }) => self.preedit = text,
                Event::Ime(egui::ImeEvent::Commit(text)) => {
                    self.preedit.clear();
                    self.send(text);
                }
                _ => {}
            }
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let theme = &THEMES[self.theme];
        ui.horizontal_centered(|ui| {
            ui.add_space(6.0);
            let mut select = None;
            let mut close = None;
            for (index, tab) in self.tabs.iter().enumerate() {
                let pane = self.panes.get(&tab.focus);
                let title = pane.and_then(|pane| pane.title.clone()).unwrap_or_else(|| "终端".to_string());
                let count = tab.root.panes().len();
                let label = if count > 1 { format!("{title}  ·{count}") } else { title };
                let selected = index == self.active;
                if ui.selectable_label(selected, label).clicked() {
                    select = Some(index);
                }
                if ui.small_button("×").on_hover_text("关闭此标签").clicked() {
                    close = Some(index);
                }
                ui.add_space(4.0);
            }
            if ui.button("+").on_hover_text(if self.mac { "新建终端（⌘T）" } else { "新建终端（Ctrl+Shift+T）" }).clicked() {
                self.new_tab();
            }
            if let Some(index) = select {
                self.active = index;
            }
            if let Some(index) = close {
                for id in self.tabs[index].root.panes() {
                    self.close_pane(&id);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(6.0);
                let fullscreen = if self.fullscreen { "退出全屏" } else { "全屏" };
                if ui.button(fullscreen).on_hover_text(if self.mac { "⌃⌘F" } else { "F11" }).clicked() {
                    self.toggle_fullscreen(ui.ctx());
                }
                if ui.button("⬓").on_hover_text("向下分屏").clicked() {
                    self.split(false);
                }
                if ui.button("◫").on_hover_text("向右分屏").clicked() {
                    self.split(true);
                }
                egui::ComboBox::from_id_salt("theme").selected_text(theme.name).show_ui(ui, |ui| {
                    for (index, item) in THEMES.iter().enumerate() {
                        ui.selectable_value(&mut self.theme, index, item.name);
                    }
                });
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            ui.add_space(8.0);
            let running = self.panes.values().filter(|pane| pane.exit.is_none()).count();
            ui.label(format!("{running} 个活动会话"));
            ui.separator();
            ui.label(format!("{} {:.0}pt", self.fonts.family(), self.font_points));
            ui.separator();
            ui.label(format!("GPU · {}", self.render_state.adapter.get_info().backend));
            if let Some(error) = &self.error {
                ui.separator();
                ui.colored_label(Color32::from_rgb(0xe5, 0x8a, 0x91), error);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                ui.label(format!("帧 {:.1} ms", self.frame_ms));
            });
        });
    }

    fn terminals(&mut self, ui: &mut egui::Ui) {
        let area = ui.max_rect();
        let Some(tab) = self.tabs.get(self.active) else {
            ui.centered_and_justified(|ui| {
                if ui.button("新建终端").clicked() {
                    self.new_tab();
                }
            });
            return;
        };
        let (mut panes, mut dividers) = (Vec::new(), Vec::new());
        let whole = Fraction { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
        if tab.zoomed {
            panes.push((tab.focus.clone(), whole));
        } else {
            tab.root.layout(whole, &mut Vec::new(), &mut panes, &mut dividers);
        }
        let to_rect = |fraction: Fraction| {
            Rect::from_min_size(
                area.min + Vec2::new(fraction.x * area.width(), fraction.y * area.height()),
                Vec2::new(fraction.w * area.width(), fraction.h * area.height()),
            )
        };
        let many = panes.len() > 1;
        let uploads: Arc<Mutex<Vec<crate::glyphs::Upload>>> = Arc::default();
        let epoch = self.fonts.epoch;
        for (id, fraction) in &panes {
            let rect = to_rect(*fraction).shrink(if many { GAP / 2.0 } else { 0.0 });
            let focused = self.tabs[self.active].focus == *id;
            self.pane(ui, id, rect, focused, many, &uploads);
        }
        // Glyphs first seen while building go up before any pane is drawn: every callback
        // prepares only after the whole frame is laid out. If the atlas started over meanwhile,
        // the panes built before that point are rebuilt next frame.
        uploads.lock().unwrap_or_else(|p| p.into_inner()).append(&mut self.fonts.uploads);
        if self.fonts.epoch != epoch {
            ui.ctx().request_repaint();
        }
        for divider in dividers {
            self.divider(ui, area, divider);
        }
    }

    fn divider(&mut self, ui: &mut egui::Ui, area: Rect, divider: Divider) {
        let parent = divider.parent;
        let rect = if divider.row {
            let x = area.min.x + divider.position * area.width();
            Rect::from_min_max(Pos2::new(x - 3.0, area.min.y + parent.y * area.height()), Pos2::new(x + 3.0, area.min.y + (parent.y + parent.h) * area.height()))
        } else {
            let y = area.min.y + divider.position * area.height();
            Rect::from_min_max(Pos2::new(area.min.x + parent.x * area.width(), y - 3.0), Pos2::new(area.min.x + (parent.x + parent.w) * area.width(), y + 3.0))
        };
        let response = ui.interact(rect, Id::unique(("divider", &divider.path)), Sense::drag());
        let response = response.on_hover_cursor(if divider.row { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::ResizeVertical });
        if response.dragged()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let ratio = if divider.row {
                ((pointer.x - area.min.x) / area.width() - parent.x) / parent.w
            } else {
                ((pointer.y - area.min.y) / area.height() - parent.y) / parent.h
            };
            self.tabs[self.active].root.set_ratio(&divider.path, ratio);
        }
        if response.hovered() || response.dragged() {
            ui.painter().rect_filled(rect.shrink(2.0), 1.0, Color32::from_rgba_unmultiplied(0x8d, 0xb4, 0xff, 120));
        }
    }

    fn pane(&mut self, ui: &mut egui::Ui, id: &str, rect: Rect, focused: bool, outlined: bool, uploads: &Arc<Mutex<Vec<crate::glyphs::Upload>>>) {
        let scale = self.scale;
        let size = [(rect.width() * scale).round(), (rect.height() * scale).round()];
        let (cols, rows) = grid::grid_size(size, self.fonts.cell);
        let Some(pane) = self.panes.get(id) else { return };
        let terminal = pane.terminal.clone();
        self.backend.resize(&terminal, Size { cols, rows });

        // Input: focus on click, selection on drag, scrolling with the wheel.
        let widget = Id::unique(("pane", pane.key));
        let response = ui.interact(rect, widget, Sense::click_and_drag());
        let cell = |position: Pos2| {
            let x = ((position.x - rect.min.x) * scale - PADDING) / self.fonts.cell.width;
            let y = ((position.y - rect.min.y) * scale - PADDING) / self.fonts.cell.height;
            let side = if x.fract() < 0.5 { Side::Left } else { Side::Right };
            (x.max(0.0) as usize, y.max(0.0) as usize, side)
        };
        if response.clicked() || response.drag_started() {
            self.tabs[self.active].focus = id.to_string();
            response.request_focus();
        }
        if response.double_clicked()
            && let Some(position) = response.interact_pointer_pos()
        {
            let (column, row, side) = cell(position);
            terminal.start_selection(column, row, side, SelectionType::Semantic);
        } else if response.drag_started()
            && let Some(position) = response.interact_pointer_pos()
        {
            let (column, row, side) = cell(position);
            terminal.start_selection(column, row, side, SelectionType::Simple);
            self.selecting = Some(id.to_string());
        } else if response.dragged()
            && self.selecting.as_deref() == Some(id)
            && let Some(position) = response.interact_pointer_pos()
        {
            let (column, row, side) = cell(position);
            terminal.extend_selection(column, row, side);
        } else if response.clicked() {
            terminal.clear_selection();
        }
        if response.drag_stopped() {
            self.selecting = None;
        }
        if focused {
            if !response.has_focus() && !ui.ctx().memory(|memory| memory.focused().is_some()) {
                response.request_focus();
            }
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(widget, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true });
            });
        }
        if response.hovered() {
            let delta = ui.input(|input| input.smooth_scroll_delta.y);
            let lines = (delta * scale / self.fonts.cell.height).round() as i32;
            if lines != 0 {
                let mode = terminal.mode();
                if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
                    let key = if lines > 0 { "\x1bOA" } else { "\x1bOB" };
                    self.backend.write(&terminal.id, key.repeat(lines.unsigned_abs() as usize));
                } else {
                    terminal.scroll(lines);
                }
            }
        }

        // Quads are rebuilt only when something they depend on changed.
        let generation = terminal.generation.load(Ordering::Acquire);
        let pane = self.panes.get_mut(id).expect("pane exists");
        let built = (generation, [size[0] as u32, size[1] as u32], focused, self.theme, self.fonts.cell.width.to_bits() ^ self.fonts.cell.height.to_bits(), self.fonts.epoch);
        if pane.built != built {
            let frame = grid::Frame { theme: &THEMES[self.theme], size, focused, background_alpha: 1.0 };
            pane.quads = Arc::new(grid::build(&terminal.lock(), &mut self.fonts, &frame));
            pane.built = built;
            pane.version += 1;
        }
        let paint = GridPaint { pane: pane.key, size, instances: pane.quads.clone(), version: pane.version, uploads: uploads.clone() };
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, paint));
        if outlined && focused {
            ui.painter().rect_stroke(rect, 3.0, Stroke::new(1.0, Color32::from_rgba_unmultiplied(0x8d, 0xb4, 0xff, 90)), egui::StrokeKind::Inside);
        }
        if let Some(code) = pane.exit {
            ui.painter().text(rect.right_bottom() - Vec2::new(12.0, 10.0), egui::Align2::RIGHT_BOTTOM, format!("会话已结束（{code}）"), egui::FontId::proportional(12.0), Color32::from_gray(160));
        }

        // The IME composes at the cursor, and what it composes is shown there until committed.
        if focused {
            let (column, row) = {
                let term = terminal.lock();
                let point = term.grid().cursor.point;
                (point.column.0 as f32, point.line.0 as f32)
            };
            let cursor = rect.min + Vec2::new((PADDING + column * self.fonts.cell.width) / scale, (PADDING + row * self.fonts.cell.height) / scale);
            let cursor_rect = Rect::from_min_size(cursor, Vec2::new(1.0, self.fonts.cell.height / scale));
            ui.ctx().output_mut(|output| {
                output.ime = Some(egui::output::IMEOutput { purpose: egui::IMEPurpose::Terminal, rect, cursor_rect, should_interrupt_composition: false });
            });
            if !self.preedit.is_empty() {
                let galley = ui.painter().layout_no_wrap(self.preedit.clone(), egui::FontId::monospace(self.font_points), THEMES[self.theme].cursor_color());
                let backdrop = Rect::from_min_size(cursor, galley.size()).expand(2.0);
                ui.painter().rect_filled(backdrop, 2.0, THEMES[self.theme].background_color());
                ui.painter().galley(cursor, galley, Color32::WHITE);
            }
        }
    }

    fn smoke(&mut self, ctx: &egui::Context) {
        let Some(smoke) = self.smoke.as_mut() else { return };
        smoke.frames += 1;
        let Some(pane) = self.tabs.first().and_then(|tab| self.panes.get(&tab.focus)) else { return };
        let elapsed = smoke.started.elapsed();
        if !smoke.typed && elapsed > Duration::from_millis(1500) {
            smoke.typed = true;
            let command = if cfg!(windows) { "Write-Output ('omniterm-' + 'smoke-' + 42)\r" } else { "printf 'omniterm-%s-%s\\n' smoke 42\r" };
            let id = pane.terminal.id.clone();
            self.backend.write(&id, command.to_string());
        }
        if smoke.seen.is_none() && smoke.typed && pane.terminal.screen_text().lines().any(|line| line.trim() == "omniterm-smoke-42") {
            smoke.seen = Some(elapsed);
        }
        if smoke.seen.is_some() && !smoke.requested {
            smoke.requested = true;
            let target = smoke.screenshot.clone();
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |image| {
                let rgba: Vec<u8> = image.pixels.iter().flat_map(|pixel| pixel.to_array()).collect();
                *target.lock().unwrap_or_else(|p| p.into_inner()) = Some(crate::png::encode(image.size[0] as u32, image.size[1] as u32, &rgba));
            })));
        }
        let shot = smoke.screenshot.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(png) = shot {
            let written = std::fs::write(&smoke.output, png).is_ok();
            let report = serde_json::json!({
                "smoke": "passed",
                "firstOutputMs": smoke.seen.map(|seen| seen.as_millis()),
                "frames": smoke.frames,
                "screenshot": written.then(|| smoke.output.display().to_string()),
                "adapter": format!("{:?}", self.render_state.adapter.get_info().backend),
                "font": self.fonts.family(),
            });
            println!("{report}");
            self.backend.shutdown();
            std::process::exit(0);
        }
        if elapsed > Duration::from_secs(40) {
            eprintln!("smoke failed: no output after {elapsed:?}; screen:\n{}", pane.terminal.screen_text());
            std::process::exit(1);
        }
        ctx.request_repaint_after(Duration::from_millis(50));
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let started = Instant::now();
        let ctx = ui.ctx().clone();
        let scale = ctx.pixels_per_point();
        if (scale - self.scale).abs() > f32::EPSILON {
            self.scale = scale;
            self.fonts.set_size(self.font_points * scale);
        }
        self.handle_notices();
        if self.focused().is_some() {
            self.keyboard(&ctx);
        }
        let theme = THEMES[self.theme];
        if !self.fullscreen {
            egui::Panel::top("tabs").exact_size(34.0).frame(egui::Frame::new().fill(theme.chrome_color())).show(ui, |ui| self.top_bar(ui));
            egui::Panel::bottom("status").exact_size(24.0).frame(egui::Frame::new().fill(theme.chrome_color())).show(ui, |ui| self.status_bar(ui));
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme.background_color())).show(ui, |ui| self.terminals(ui));
        if ctx.input(|input| input.viewport().close_requested()) {
            self.backend.shutdown();
        }
        self.smoke(&ctx);
        self.frame_ms = self.frame_ms * 0.9 + started.elapsed().as_secs_f32() * 1000.0 * 0.1;
    }
}

impl crate::theme::Theme {
    pub fn background_color(&self) -> Color32 {
        Color32::from_rgb(self.background.r, self.background.g, self.background.b)
    }
    pub fn cursor_color(&self) -> Color32 {
        Color32::from_rgb(self.cursor.r, self.cursor.g, self.cursor.b)
    }
    /// The bars around the terminals: the background, nudged toward the text color.
    pub fn chrome_color(&self) -> Color32 {
        let lift = |background: u8, text: u8| (background as f32 + (text as f32 - background as f32) * 0.055).round() as u8;
        Color32::from_rgb(lift(self.background.r, self.foreground.r), lift(self.background.g, self.foreground.g), lift(self.background.b, self.foreground.b))
    }
}

fn style(ctx: &egui::Context) {
    ctx.set_visuals(egui::Visuals::dark());
    // CJK in the chrome comes from the system; egui's bundled fonts have no Chinese glyphs.
    let mut definitions = egui::FontDefinitions::default();
    for path in CJK_FONTS {
        if let Ok(bytes) = std::fs::read(path) {
            definitions.font_data.insert("cjk".into(), Arc::new(egui::FontData::from_owned(bytes)));
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                definitions.families.entry(family).or_default().push("cjk".into());
            }
            break;
        }
    }
    ctx.set_fonts(definitions);
}

const CJK_FONTS: &[&str] = &[
    "C:\\Windows\\Fonts\\msyh.ttc",
    "C:\\Windows\\Fonts\\simsun.ttc",
    "/System/Library/Fonts/PingFang.ttc",
    "/System/Library/Fonts/STHeiti Light.ttc",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
];

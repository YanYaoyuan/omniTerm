// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! A deferred native viewport keeps following while the main window is unfocused.
use eframe::egui::{self, Color32, Pos2, Rect, Vec2, ViewportCommand, ViewportId};
use std::{sync::{Arc, Mutex}, time::{Duration, Instant}};
use crate::desktop_pointer::{Sample, Tracker};
const SIZE: Vec2 = Vec2::new(160.0, 125.0);
fn id() -> ViewportId { ViewportId::from_hash_of("byte-desktop-pet") }

#[derive(Default)]
pub struct DesktopPet { inner: Arc<Mutex<Inner>> }
#[derive(Default)]
struct Inner {
    enabled: bool, paused: bool, error: Option<String>, tracker: Option<Tracker>,
    art: Option<Art>, motion: Motion, last_tick: Option<Instant>, elapsed: f32, last_position: Option<Pos2>,
}
struct Art { idle: egui::TextureHandle, poses: [Rect; 3], run: egui::TextureHandle, frames: [Rect; 4] }

impl DesktopPet {
    pub fn enabled(&self) -> bool { self.inner.lock().unwrap().enabled }
    pub fn paused(&self) -> bool { self.inner.lock().unwrap().paused }
    pub fn error(&self) -> Option<String> { self.inner.lock().unwrap().error.clone() }
    pub fn stop(&self, ctx: &egui::Context) {
        let mut inner = self.inner.lock().unwrap();
        inner.enabled = false; inner.tracker = None;
        ctx.send_viewport_cmd_to(id(), ViewportCommand::Close);
        ctx.request_repaint();
    }
    pub fn pause(&self, ctx: &egui::Context) {
        let mut inner = self.inner.lock().unwrap(); inner.paused = !inner.paused; inner.last_tick = None;
        ctx.request_repaint_of(id()); ctx.request_repaint();
    }
    pub fn start(&self, ctx: &egui::Context, idle: egui::TextureHandle, poses: [Rect; 3]) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let result = (|| {
            if ctx.embed_viewports() { return Err("当前渲染环境不支持独立桌宠窗口。".to_owned()); }
            let tracker = Tracker::new()?;
            let sample = tracker.sample()?;
            if inner.art.is_none() {
                let image = image::load_from_memory(include_bytes!("../../assets/easter-eggs/byte-run.png")).map_err(|e| e.to_string())?;
                let limit = ctx.input(|i| i.max_texture_side).clamp(1, u32::MAX as usize) as u32;
                let image = if image.width() > limit || image.height() > limit { image.resize(limit, limit, image::imageops::FilterType::Triangle) } else { image };
                let rgba = image.to_rgba8();
                // Keep a shared vertical baseline across the animation cycle.
                let frames = std::array::from_fn(|i| Rect::from_min_max(Pos2::new(i as f32 / 4.0, 0.20), Pos2::new((i + 1) as f32 / 4.0, 0.81)));
                let run = ctx.load_texture("byte-running", egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()), egui::TextureOptions::LINEAR);
                inner.art = Some(Art { idle, poses, run, frames });
            }
            inner.motion = Motion::default(); inner.motion.step(&sample, units(ctx), 0.0);
            inner.tracker = Some(tracker);
            Ok::<(), String>(())
        })();
        match result {
            Ok(()) => { inner.enabled = true; inner.paused = false; inner.error = None; inner.elapsed = 0.0; inner.last_tick = None; inner.last_position = None; ctx.request_repaint(); true }
            Err(error) => { inner.error = Some(error); false }
        }
    }
    pub fn show(&self, ctx: &egui::Context) {
        let position = {
            let inner = self.inner.lock().unwrap();
            if !inner.enabled { return; }
            (inner.motion.center.unwrap_or(Pos2::ZERO) - SIZE * units(ctx) * 0.5) / units(ctx)
        };
        let shared = self.inner.clone();
        ctx.show_viewport_deferred(id(), egui::ViewportBuilder::default()
            .with_title("BYTE · omniTerm desktop companion")
            .with_inner_size(SIZE).with_position(position).with_resizable(false)
            .with_decorations(false).with_transparent(true).with_has_shadow(false)
            .with_active(false).with_taskbar(false).with_mouse_passthrough(true)
            .with_window_level(egui::WindowLevel::AlwaysOnTop)
            // EWMH docks are omitted from X11 task switchers and survive root
            // minimization. No strut is set, so no desktop space is reserved.
            .with_window_type(egui::X11WindowType::Dock), move |ui, class| {
                if class == egui::ViewportClass::EmbeddedWindow { return; }
                let ctx = ui.ctx();
                let mut inner = shared.lock().unwrap();
                if !inner.enabled || ctx.input(|i| i.viewport().close_requested()) {
                    inner.enabled = false; inner.tracker = None;
                    ctx.send_viewport_cmd(ViewportCommand::Close); ctx.request_repaint_of(ViewportId::ROOT); return;
                }
                let now = Instant::now();
                let dt = inner.last_tick.replace(now).map_or(0.0, |t| now.duration_since(t).as_secs_f32().min(0.05));
                if !inner.paused {
                    let sample = inner.tracker.as_ref().unwrap().sample();
                    match sample {
                        Ok(sample) if sample.pointer.is_finite() && sample.bounds.is_positive() => {
                            inner.motion.step(&sample, units(ctx), dt); inner.elapsed += dt;
                        }
                        Ok(_) | Err(_) => {
                            inner.error = Some("桌面位置暂不可用，请收回后重新放出小猫。".into());
                            inner.enabled = false; inner.tracker = None;
                            ctx.send_viewport_cmd(ViewportCommand::Close); ctx.request_repaint_of(ViewportId::ROOT); return;
                        }
                    }
                }
                let position = (inner.motion.center.unwrap() - SIZE * units(ctx) * 0.5) / units(ctx);
                if inner.last_position.is_none_or(|last| last.distance(position) > 0.25) {
                    ctx.send_viewport_cmd(ViewportCommand::OuterPosition(position)); inner.last_position = Some(position);
                }
                let art = inner.art.as_ref().unwrap();
                let walking = !inner.paused && inner.motion.moving;
                let (texture, mut uv) = if walking { (&art.run, art.frames[(inner.elapsed * 10.0) as usize % 4]) }
                    else { (&art.idle, art.poses[usize::from(!inner.paused && inner.motion.idle < 2.5)]) };
                let ratio = texture.size_vec2().x * uv.width() / (texture.size_vec2().y * uv.height());
                let size = Vec2::new(140.0, 107.0).min(Vec2::new(107.0 * ratio, 140.0 / ratio));
                let rect = Rect::from_center_size(ctx.content_rect().center(), size);
                if !inner.motion.right { std::mem::swap(&mut uv.min.x, &mut uv.max.x); }
                ctx.layer_painter(egui::LayerId::background()).image(texture.id(), rect, uv, Color32::WHITE);
                if !inner.paused { ctx.request_repaint_after(Duration::from_millis(if walking { 33 } else { 100 })); }
            });
    }
}
// Quartz uses desktop points. Win32 / X11 use physical pixels, while viewport
// commands are egui points. Recompute on each frame for zoom and monitor DPI.
fn units(ctx: &egui::Context) -> f32 {
    if cfg!(target_os = "macos") { ctx.zoom_factor() } else { ctx.pixels_per_point() }
}

#[derive(Default)]
struct Motion { center: Option<Pos2>, cursor: Option<Pos2>, moving: bool, right: bool, idle: f32 }
impl Motion {
    fn step(&mut self, sample: &Sample, scale: f32, dt: f32) {
        let half = SIZE * scale * 0.5;
        let clamp = |p: Pos2| {
            let low = sample.bounds.min + half.min(sample.bounds.size() * 0.5);
            let high = sample.bounds.max - half.min(sample.bounds.size() * 0.5);
            Pos2::new(p.x.clamp(low.x, high.x), p.y.clamp(low.y, high.y))
        };
        let p = self.center.get_or_insert_with(|| clamp(sample.pointer + Vec2::new(-180.0, 70.0) * scale));
        *p = clamp(*p);
        self.idle = if self.cursor.is_none_or(|last| last.distance(sample.pointer) > scale * 2.0) { 0.0 } else { self.idle + dt };
        self.cursor = Some(sample.pointer);
        let diff = sample.pointer - *p;
        let distance = diff.length();
        // Stop outside the visible cat: it never covers the cursor or overshoots.
        let gap = 95.0 * scale;
        let travel = ((distance - gap).max(0.0) * 4.0).min(420.0 * scale) * dt.min(0.05);
        let next = clamp(*p + diff.normalized() * travel.min((distance - gap).max(0.0)));
        self.moving = next.distance(*p) > 0.1 * scale;
        if diff.x.abs() > 8.0 * scale { self.right = diff.x > 0.0; }
        *p = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn follows_global_pointer_without_overshooting_at_any_scale() {
        for scale in [1.0, 1.5, 2.0] {
            let sample = Sample { pointer: Pos2::new(1400.0, 700.0) * scale, bounds: Rect::from_min_size(Pos2::new(-1600.0, -200.0) * scale, Vec2::new(3520.0, 1280.0) * scale) };
            let mut motion = Motion::default();
            for _ in 0..400 { motion.step(&sample, scale, 0.033); }
            let distance = motion.center.unwrap().distance(sample.pointer) / scale;
            assert!((94.9..96.0).contains(&distance), "{distance}");
            assert!(!motion.moving); assert!(motion.right); assert!(motion.idle > 2.5);
            let left = Sample { pointer: Pos2::new(-1400.0, 200.0) * scale, ..sample };
            motion.step(&left, scale, 0.033); assert!(!motion.right); assert!(motion.moving);
        }
    }
    #[test]
    fn screen_edges_and_small_displays_never_push_the_window_outside() {
        for bounds in [Rect::from_min_size(Pos2::new(-800.0, -600.0), Vec2::new(800.0, 600.0)), Rect::from_min_size(Pos2::ZERO, Vec2::splat(40.0))] {
            let mut motion = Motion::default();
            for pointer in [bounds.min, bounds.max] {
                for _ in 0..100 { motion.step(&Sample { pointer, bounds }, 1.0, 1.0); }
                assert!(bounds.contains(motion.center.unwrap()));
                if bounds.width() > SIZE.x { assert!(bounds.contains_rect(Rect::from_center_size(motion.center.unwrap(), SIZE))); }
            }
        }
    }
}

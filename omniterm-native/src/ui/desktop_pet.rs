// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Fixed transparent GPU canvas; pointer I/O never runs on the render thread.
use eframe::egui::{self, Pos2, Rect, Vec2, ViewportCommand, ViewportId};
use serde::Serialize;
use std::{sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};
use crate::desktop_pointer::{Sample, Tracker};
use super::pet_rig;
const SIZE: Vec2 = Vec2::new(160.0, 125.0);
const MAX_ANIMATION_STEP: f32 = 1.0 / 30.0;
fn id() -> ViewportId { ViewportId::from_hash_of("byte-desktop-pet") }

struct Feed {
    sample: Arc<Mutex<Result<Sample, String>>>, stopped: Arc<AtomicBool>, paused: Arc<AtomicBool>, worker: std::thread::JoinHandle<()>,
}
impl Feed {
    fn new(ctx: egui::Context) -> Result<Self, String> {
        let mut tracker = Tracker::new()?;
        let initial = tracker.sample()?;
        let sample = Arc::new(Mutex::new(Ok(initial)));
        let stopped = Arc::new(AtomicBool::new(false)); let paused = Arc::new(AtomicBool::new(false));
        let (output, stop, pause) = (sample.clone(), stopped.clone(), paused.clone());
        let worker = std::thread::Builder::new().name("byte-pointer".into()).spawn(move || {
            let mut previous = initial;
            while !stop.load(Ordering::Acquire) {
                if !pause.load(Ordering::Acquire) {
                    let next = tracker.sample();
                    let changed = next.as_ref().map_or(true, |s| s.pointer != previous.pointer || s.bounds != previous.bounds);
                    if let Ok(s) = &next { previous = *s; }
                    let failed = next.is_err();
                    *output.lock().unwrap() = next;
                    if changed { ctx.request_repaint_of(id()); }
                    if failed { break; }
                }
                std::thread::park_timeout(Duration::from_millis(if pause.load(Ordering::Relaxed) { 100 } else { 8 }));
            }
        }).map_err(|e| e.to_string())?;
        Ok(Self { sample, stopped, paused, worker })
    }
    fn read(&self) -> Result<Sample, String> { self.sample.lock().unwrap().clone() }
    fn pause(&self, paused: bool) { self.paused.store(paused, Ordering::Release); self.worker.thread().unpark(); }
}
impl Drop for Feed {
    fn drop(&mut self) { self.stopped.store(true, Ordering::Release); self.worker.thread().unpark(); }
}

#[derive(Clone, Default, Serialize)]
struct Renderer { name: String, backend: String, kind: String, hardware: bool }
#[derive(Default)]
pub struct DesktopPet { inner: Arc<Mutex<Inner>> }
struct Inner {
    enabled: bool, paused: bool, asynchronous: bool, error: Option<String>, feed: Option<Feed>,
    motion: Motion, last_tick: Option<Instant>, elapsed: f32, animation_time: f32, canvas: Rect, desktop_bounds: Rect, surface_limit: f32, canvas_units: f32,
    facing: f32, gait: f32, sleep: f32, renderer: Renderer, stats: Stats, synthetic: bool,
}
impl Default for Inner {
    fn default() -> Self {
        Self { enabled: false, paused: false, asynchronous: false, error: None, feed: None, motion: Motion::default(), last_tick: None, elapsed: 0.0, animation_time: 0.0, canvas: Rect::NOTHING, desktop_bounds: Rect::NOTHING, surface_limit: 8192.0, canvas_units: 1.0, facing: 1.0, gait: 0.0, sleep: 0.0, renderer: Renderer::default(), stats: Stats::default(), synthetic: false }
    }
}
#[derive(Default)]
struct Stats { intervals: Vec<f64>, work: Vec<f64>, canvas_updates: u32, last_frame: Option<u64>, root_start: u64, surface_modes: Vec<String>, resumes: u32 }
impl DesktopPet {
    pub fn enabled(&self) -> bool { self.inner.lock().unwrap().enabled }
    pub fn paused(&self) -> bool { self.inner.lock().unwrap().paused }
    pub fn error(&self) -> Option<String> { self.inner.lock().unwrap().error.clone() }
    pub fn renderer(&self, render: &eframe::egui_wgpu::RenderState) {
        let info = render.adapter.get_info();
        let mut inner = self.inner.lock().unwrap(); inner.asynchronous = crate::gpu::prefer_asynchronous(&info); inner.surface_limit = render.device.limits().max_texture_dimension_2d as f32;
        inner.renderer = Renderer { name: info.name.clone(), backend: format!("{:?}", info.backend), kind: format!("{:?}", info.device_type), hardware: !matches!(info.device_type, eframe::wgpu::DeviceType::Cpu) };
    }
    pub fn surface_config(&self) -> eframe::egui_wgpu::SurfaceConfig {
        let inner = self.inner.lock().unwrap();
        if inner.enabled && !inner.paused && inner.asynchronous { crate::gpu::asynchronous_surface() }
        else { crate::gpu::surface_config() }
    }
    pub fn renderer_label(&self) -> String {
        let inner = self.inner.lock().unwrap();
        if inner.renderer.name.is_empty() { return String::new(); }
        format!("{} · {} · {}{}", if inner.renderer.hardware { "GPU" } else { "软件渲染" }, inner.renderer.name, inner.renderer.backend, if inner.enabled && !inner.paused && inner.asynchronous { " · 低延迟" } else { "" })
    }
    pub fn stop(&self, ctx: &egui::Context) {
        let mut inner = self.inner.lock().unwrap(); inner.enabled = false; inner.feed = None;
        ctx.send_viewport_cmd_to(id(), ViewportCommand::Close); ctx.request_repaint();
    }
    pub fn pause(&self, ctx: &egui::Context) {
        let mut inner = self.inner.lock().unwrap(); inner.paused = !inner.paused; inner.last_tick = None; if !inner.paused { inner.stats.resumes += 1; }
        if let Some(feed) = &inner.feed { feed.pause(inner.paused); }
        ctx.request_repaint_of(id()); ctx.request_repaint();
    }
    pub fn start(&self, ctx: &egui::Context) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let result = (|| {
            if ctx.embed_viewports() { return Err("当前渲染环境不支持独立桌宠窗口。".to_owned()); }
            let feed = Feed::new(ctx.clone())?;
            let sample = feed.read()?;
            inner.motion = Motion::default(); inner.motion.step(&sample, units(ctx), 0.0);
            inner.desktop_bounds = sample.bounds; inner.canvas_units = units(ctx);
            inner.canvas = canvas(sample.bounds, inner.motion.center.unwrap(), inner.surface_limit * units(ctx) / ctx.pixels_per_point());
            inner.feed = Some(feed);
            Ok::<(), String>(())
        })();
        match result {
            Ok(()) => { inner.enabled = true; inner.paused = false; inner.error = None; inner.elapsed = 0.0; inner.animation_time = 0.0; inner.last_tick = None; inner.facing = 1.0; inner.gait = 0.0; inner.sleep = 0.0; inner.stats = Stats { root_start: ctx.cumulative_frame_nr_for(ViewportId::ROOT), ..Default::default() }; ctx.request_repaint(); true }
            Err(error) => { inner.error = Some(error); false }
        }
    }
    pub fn show(&self, ctx: &egui::Context) {
        let (bounds, scale) = { let inner = self.inner.lock().unwrap(); if !inner.enabled { return; } (inner.canvas, inner.canvas_units) };
        let shared = self.inner.clone();
        ctx.show_viewport_deferred(id(), egui::ViewportBuilder::default()
            .with_title("BYTE · omniTerm desktop companion")
            .with_inner_size(bounds.size() / scale).with_position(bounds.min / scale).with_resizable(false)
            .with_decorations(false).with_transparent(true).with_has_shadow(false)
            .with_active(false).with_taskbar(false).with_drag_and_drop(false).with_mouse_passthrough(true)
            .with_window_level(egui::WindowLevel::AlwaysOnTop).with_window_type(egui::X11WindowType::Dock), move |ui, class| {
                if class == egui::ViewportClass::EmbeddedWindow { return; }
                let ctx = ui.ctx(); let started = Instant::now();
                let mut inner = shared.lock().unwrap();
                if !inner.enabled || ctx.input(|i| i.viewport().close_requested()) {
                    inner.enabled = false; inner.feed = None;
                    ctx.send_viewport_cmd(ViewportCommand::Close); ctx.request_repaint_of(ViewportId::ROOT); return;
                }
                // Use the canvas monitor's scale, which can differ from the
                // terminal window's monitor on mixed-DPI desktops.
                inner.canvas_units = units(ctx);
                let wanted_size = inner.canvas.size() / inner.canvas_units;
                if (ctx.content_rect().size() - wanted_size).abs().max_elem() > 2.0 {
                    ctx.send_viewport_cmd(ViewportCommand::InnerSize(wanted_size));
                }
                let frame = ctx.cumulative_frame_nr();
                if inner.stats.last_frame != Some(frame) {
                    inner.stats.last_frame = Some(frame);
                    let dt = inner.last_tick.replace(started).map_or(0.0, |last| started.duration_since(last).as_secs_f32());
                    if dt > 0.0 && inner.elapsed > 1.0 && !inner.paused && inner.stats.intervals.len() < 20000 { inner.stats.intervals.push(f64::from(dt) * 1000.0); }
                    if !inner.paused {
                        let result = inner.feed.as_ref().unwrap().read();
                        match result {
                            Ok(mut sample) if sample.pointer.is_finite() && sample.bounds.is_positive() => {
                                inner.elapsed += dt; inner.animation_time += dt.min(MAX_ANIMATION_STEP);
                                if inner.synthetic {
                                    let t = inner.elapsed;
                                    sample.pointer = sample.bounds.center() + Vec2::new((t * 0.9).cos() * sample.bounds.width() * 0.3, (t * 1.3).sin() * sample.bounds.height() * 0.25);
                                }
                                inner.motion.step(&sample, units(ctx), dt);
                                let limit = inner.surface_limit * units(ctx) / ctx.pixels_per_point();
                                // Ordinary desktops use one stationary canvas. Only a display
                                // reconfiguration or a desktop larger than the surface limit moves it.
                                if inner.desktop_bounds != sample.bounds || !inner.canvas.contains_rect(Rect::from_center_size(inner.motion.center.unwrap(), SIZE * units(ctx)).expand(12.0)) {
                                    inner.desktop_bounds = sample.bounds;
                                    let next = canvas(sample.bounds, inner.motion.center.unwrap(), limit);
                                    if next != inner.canvas {
                                        inner.canvas = next; inner.stats.canvas_updates += 1;
                                        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(next.min / units(ctx)));
                                        ctx.send_viewport_cmd(ViewportCommand::InnerSize(next.size() / units(ctx)));
                                    }
                                }
                            }
                            _ => { inner.error = Some("桌面位置暂不可用，请收回后重新放出小猫。".into()); inner.enabled = false; inner.feed = None; ctx.send_viewport_cmd(ViewportCommand::Close); ctx.request_repaint_of(ViewportId::ROOT); return; }
                        }
                    }
                    let blend = if inner.paused { 0.0 } else { 1.0 - (-dt.min(MAX_ANIMATION_STEP) * 14.0).exp() };
                    let target_facing = if inner.motion.right { 1.0 } else { -1.0 };
                    inner.facing += (target_facing - inner.facing) * blend;
                    let gait = if inner.paused || !inner.motion.moving { 0.0 } else { (inner.motion.speed / 240.0).clamp(0.0, 1.0) };
                    inner.gait += (gait - inner.gait) * blend;
                    let sleep = f32::from(inner.paused || inner.motion.idle > 2.5);
                    inner.sleep += (sleep - inner.sleep) * blend;
                }
                let center = (inner.motion.center.unwrap() - inner.canvas.min.to_vec2()) / units(ctx);
                pet_rig::paint(&ctx.layer_painter(egui::LayerId::background()), center, &pet_rig::Pose { phase: pet_rig::phase(inner.motion.travel), gait: inner.gait, facing: inner.facing, sleep: inner.sleep, time: inner.animation_time });
                if !inner.paused {
                    if inner.synthetic || inner.motion.moving || inner.gait > 0.01 || inner.sleep > 0.01 && inner.sleep < 0.99 {
                        if inner.asynchronous { ctx.request_repaint_after(Duration::from_secs_f64(1.0 / 120.0)); }
                        else { ctx.request_repaint(); }
                    }
                    else { ctx.request_repaint_after(Duration::from_millis(100)); }
                }
                if inner.stats.work.len() < 20000 { inner.stats.work.push(started.elapsed().as_secs_f64() * 1000.0); }
            });
    }
    fn report(&self, ctx: &egui::Context) -> serde_json::Value {
        let inner = self.inner.lock().unwrap();
        let summarize = |values: &[f64]| {
            let mut sorted = values.to_vec(); sorted.sort_by(f64::total_cmp);
            let p = |q: f64| sorted.get(((sorted.len().saturating_sub(1)) as f64 * q) as usize).copied().unwrap_or(0.0);
            serde_json::json!({"count": sorted.len(), "medianMs": p(0.5), "p95Ms": p(0.95), "p99Ms": p(0.99)})
        };
        let total: f64 = inner.stats.intervals.iter().sum();
        serde_json::json!({"renderer": inner.renderer, "requestedPresentModes": inner.stats.surface_modes, "pauseResumeCycles": inner.stats.resumes, "presentation": if inner.asynchronous { "AutoNoVsync" } else { "AutoVsync" }, "frameIntervals": summarize(&inner.stats.intervals), "uiWork": summarize(&inner.stats.work), "warmupSeconds": 1.0, "slowFrames": inner.stats.intervals.iter().enumerate().filter(|(_, ms)| **ms > 30.0).map(|(n, ms)| serde_json::json!([n, ms])).collect::<Vec<_>>(), "callbackFps": inner.stats.intervals.len() as f64 / (total / 1000.0).max(0.001), "canvasUpdatesDuringRun": inner.stats.canvas_updates, "canvasPixels": [inner.canvas.width(), inner.canvas.height()], "rootFrames": ctx.cumulative_frame_nr_for(ViewportId::ROOT).saturating_sub(inner.stats.root_start), "motion": "continuous-joints", "pointerIoOnUiThread": false})
    }
}
fn units(ctx: &egui::Context) -> f32 { if cfg!(target_os = "macos") { ctx.zoom_factor() } else { ctx.pixels_per_point() } }
// Keep the surface within the device limit on very wide multi-monitor desktops.
// Hysteresis in `show` keeps this large tile stationary until the cat reaches its edge.
fn canvas(bounds: Rect, center: Pos2, max_side: f32) -> Rect {
    let size = bounds.size().min(Vec2::splat(max_side));
    let min = (center - size * 0.5).max(bounds.min).min(bounds.max - size);
    Rect::from_min_size(min, size)
}

#[derive(Default)]
struct Motion { center: Option<Pos2>, cursor: Option<Pos2>, velocity: Vec2, moving: bool, right: bool, idle: f32, speed: f32, travel: f32 }
impl Motion {
    fn step(&mut self, sample: &Sample, scale: f32, dt: f32) {
        let half = SIZE * scale * 0.5;
        let low = sample.bounds.min + half.min(sample.bounds.size() * 0.5);
        let high = sample.bounds.max - half.min(sample.bounds.size() * 0.5);
        let clamp = |p: Pos2| Pos2::new(p.x.clamp(low.x, high.x), p.y.clamp(low.y, high.y));
        let p = self.center.get_or_insert_with(|| clamp(sample.pointer + Vec2::new(-180.0, 70.0) * scale)); *p = clamp(*p);
        self.idle = if self.cursor.is_none_or(|last| last.distance(sample.pointer) > scale * 1.0) { 0.0 } else { self.idle + dt };
        self.cursor = Some(sample.pointer);
        let before = *p;
        // Fixed small steps preserve 30/60/144 Hz acceleration. A stalled present
        // must not jump the cat and its joints forward by a whole lost interval.
        let steps = (dt.clamp(0.0, MAX_ANIMATION_STEP) * 240.0).ceil().max(1.0) as u32;
        let step = dt.clamp(0.0, MAX_ANIMATION_STEP) / steps as f32;
        for _ in 0..steps {
            let diff = sample.pointer - *p; let distance = diff.length();
            let desired = diff.normalized() * ((distance - 95.0 * scale).max(0.0) * 5.0).min(560.0 * scale);
            self.velocity += (desired - self.velocity) * (1.0 - (-step * 15.0).exp());
            let mut next = clamp(*p + self.velocity * step);
            if next.distance(sample.pointer) < 95.0 * scale && distance >= 95.0 * scale { next = sample.pointer - diff.normalized() * (95.0 * scale); self.velocity = Vec2::ZERO; }
            *p = next;
            if diff.x.abs() > 18.0 * scale { self.right = diff.x > 0.0; }
        }
        self.speed = self.velocity.length() / scale;
        self.moving = self.speed > 2.0 && p.distance(before) > 0.002 * scale;
        self.travel = (self.travel + p.distance(before) / scale).rem_euclid(pet_rig::STRIDE);
    }
}

/// A short, isolated GPU probe. Its target is simulated; it never moves the real cursor.
pub fn benchmark(seconds: f32, output: std::path::PathBuf, cycle_pause: bool) -> Result<(), Box<dyn std::error::Error>> {
    struct Probe { pet: DesktopPet, started: Instant, seconds: f32, cycle_pause: bool, stage: u8 }
    impl eframe::App for Probe {
        fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] { [0.0; 4] }
        fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
            let elapsed = self.started.elapsed().as_secs_f32();
            if self.cycle_pause && self.stage == 0 && elapsed > self.seconds * 0.4 { self.pet.pause(ui.ctx()); self.stage = 1; }
            if self.cycle_pause && self.stage == 1 && elapsed > self.seconds * 0.4 + 0.7 { self.pet.pause(ui.ctx()); self.stage = 2; }
            frame.set_wgpu_surface_config(self.pet.surface_config());
            if let Some(config) = frame.wgpu_surface_config() {
                let mode = format!("{:?}", config.present_mode);
                let mut inner = self.pet.inner.lock().unwrap();
                if inner.stats.surface_modes.last() != Some(&mode) { inner.stats.surface_modes.push(mode); }
            }
            self.pet.show(ui.ctx());
            ui.label("BYTE GPU performance probe");
            if self.started.elapsed().as_secs_f32() >= self.seconds { ui.ctx().send_viewport_cmd(ViewportCommand::Close); }
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
    }
    let pet = DesktopPet::default(); let shared = pet.inner.clone(); let context = Arc::new(Mutex::new(None)); let context_out = context.clone();
    let options = eframe::NativeOptions { renderer: eframe::Renderer::Wgpu, wgpu_options: crate::gpu::configuration(), viewport: egui::ViewportBuilder::default().with_title("omniTerm · BYTE GPU probe").with_inner_size([230.0, 55.0]).with_position([40.0, 40.0]).with_active(false).with_mouse_passthrough(true).with_taskbar(false).with_decorations(false).with_transparent(true), ..Default::default() };
    eframe::run_native("BYTE GPU probe", options, Box::new(move |cc| {
        pet.renderer(cc.wgpu_render_state.as_ref().unwrap());
        if !pet.start(&cc.egui_ctx) { return Err(pet.error().unwrap_or_default().into()); }
        { let mut inner = pet.inner.lock().unwrap(); inner.synthetic = true; inner.feed.as_ref().unwrap().pause(true); }
        *context_out.lock().unwrap() = Some(cc.egui_ctx.clone());
        Ok(Box::new(Probe { pet, started: Instant::now(), seconds: seconds.clamp(2.0, 60.0), cycle_pause, stage: 0 }))
    }))?;
    let pet = DesktopPet { inner: shared }; let context = context.lock().unwrap().take().unwrap();
    let report = pet.report(&context);
    std::fs::write(&output, serde_json::to_vec_pretty(&report)?)?;
    println!("{}", report);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn follows_without_overshooting_at_any_scale() {
        for scale in [1.0, 1.5, 2.0] {
            let sample = Sample { pointer: Pos2::new(1400.0, 700.0) * scale, bounds: Rect::from_min_size(Pos2::new(-1600.0, -200.0) * scale, Vec2::new(3520.0, 1280.0) * scale) };
            let mut motion = Motion::default();
            for _ in 0..400 { motion.step(&sample, scale, 0.033); }
            assert!((94.9..96.0).contains(&(motion.center.unwrap().distance(sample.pointer) / scale)));
            assert!(!motion.moving); assert!(motion.right);
            motion.step(&Sample { pointer: Pos2::new(-1400.0, 200.0) * scale, ..sample }, scale, 0.033); assert!(!motion.right); assert!(motion.moving);
        }
    }
    #[test]
    fn acceleration_is_continuous_and_independent_of_refresh_rate() {
        let sample = Sample { pointer: Pos2::new(1700.0, 900.0), bounds: Rect::from_min_size(Pos2::ZERO, Vec2::new(2000.0, 1100.0)) };
        let mut endpoints = Vec::new();
        for hz in [30, 60, 144] {
            let mut motion = Motion { center: Some(Pos2::new(200.0, 300.0)), ..Default::default() };
            let mut last_speed = 0.0;
            for _ in 0..hz * 2 {
                motion.step(&sample, 1.0, 1.0 / hz as f32);
                assert!((motion.speed - last_speed).abs() < 250.0); last_speed = motion.speed;
            }
            endpoints.push(motion.center.unwrap());
        }
        assert!(endpoints[0].distance(endpoints[2]) < 3.0);
    }
    #[test]
    fn a_stalled_frame_does_not_teleport_the_cat_or_advance_a_whole_stride() {
        let sample = Sample { pointer: Pos2::new(1700.0, 900.0), bounds: Rect::from_min_size(Pos2::ZERO, Vec2::new(2000.0, 1100.0)) };
        let mut motion = Motion { center: Some(Pos2::new(200.0, 300.0)), velocity: Vec2::new(560.0, 0.0), ..Default::default() };
        let before = motion.center.unwrap();
        motion.step(&sample, 1.0, 0.25);
        assert!(before.distance(motion.center.unwrap()) <= 560.0 * MAX_ANIMATION_STEP + 0.01);
        assert!(motion.travel <= 560.0 * MAX_ANIMATION_STEP + 0.01);
    }
    #[test]
    fn desktop_canvas_is_stationary_and_respects_gpu_surface_limits() {
        let bounds = Rect::from_min_size(Pos2::new(-1920.0, 0.0), Vec2::new(5760.0, 2160.0));
        assert_eq!(canvas(bounds, Pos2::new(100.0, 300.0), 8192.0), bounds);
        assert_eq!(canvas(bounds, Pos2::new(3000.0, 1200.0), 8192.0), bounds);
        let tile = canvas(bounds, Pos2::new(3000.0, 1200.0), 2048.0);
        assert!(bounds.contains_rect(tile)); assert!(tile.size().max_elem() <= 2048.0);
    }
    #[test]
    fn screen_edges_and_small_displays_keep_the_cat_inside() {
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

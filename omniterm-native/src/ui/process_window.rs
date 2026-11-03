// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! The process window: every process on this machine or an SSH host, sampled on a background
//! thread every two seconds, searchable and sortable, with a CPU trend for the selected one and
//! a read-only monitor a click away. It can reach no terminal, approval or credential.

use super::style::{self, icon};
use crate::icons;
use crate::monitor::probe::{self, Platform, Process};
use eframe::egui::{self, Align, Layout, RichText, Sense, Stroke, Vec2, ViewportBuilder, ViewportId};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const INTERVAL: Duration = Duration::from_secs(2);
const TREND: usize = 60;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sort {
    Cpu,
    Memory,
    Pid,
    Name,
}

#[derive(Default)]
struct Snapshot {
    processes: Vec<Process>,
    error: Option<String>,
    trends: HashMap<u64, VecDeque<f64>>,
}

pub struct ProcessWindow {
    pub open: bool,
    host: Option<String>,
    query: String,
    sort: Sort,
    selected: Option<u64>,
    data: Arc<Mutex<Snapshot>>,
    sampling: Option<Arc<AtomicBool>>,
}

impl Default for ProcessWindow {
    fn default() -> Self {
        Self { open: false, host: None, query: String::new(), sort: Sort::Cpu, selected: None, data: Arc::default(), sampling: None }
    }
}

impl ProcessWindow {
    pub fn open_for(&mut self, host: Option<String>) {
        if self.host != host {
            self.stop();
            *self.data.lock().unwrap_or_else(|p| p.into_inner()) = Snapshot::default();
            self.selected = None;
        }
        self.host = host;
        self.open = true;
    }

    fn stop(&mut self) {
        if let Some(flag) = self.sampling.take() {
            flag.store(true, Ordering::Release);
        }
    }

    /// The sampler runs only while the window is open.
    fn ensure_sampler(&mut self, remotes: &Arc<crate::remote::manager::Manager>, ctx: &egui::Context) {
        if self.sampling.is_some() {
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        self.sampling = Some(stop.clone());
        let (data, host, remotes, ctx) = (self.data.clone(), self.host.clone(), remotes.clone(), ctx.clone());
        std::thread::spawn(move || {
            let mut previous: Option<(Vec<Process>, Instant)> = None;
            while !stop.load(Ordering::Acquire) {
                let platform = if host.is_none() { Platform::local() } else { Platform::Linux };
                let script = probe::process_list_script(platform);
                let output = match &host {
                    None => probe::run_local(script),
                    Some(session) => remotes.get(session).ok_or_else(|| "这个 SSH 会话已经关闭。".to_string()).and_then(|remote| remote.exec(script, Duration::from_secs(10))),
                };
                {
                    let mut snapshot = data.lock().unwrap_or_else(|p| p.into_inner());
                    match output {
                        Ok(output) => {
                            let mut list = probe::parse_processes(&output, platform);
                            if let Some((before, at)) = &previous {
                                probe::with_process_rates(&mut list, before, at.elapsed().as_secs_f64());
                            }
                            previous = Some((list.clone(), Instant::now()));
                            let live: std::collections::HashSet<u64> = list.iter().map(|p| p.pid).collect();
                            snapshot.trends.retain(|pid, _| live.contains(pid));
                            for process in &list {
                                let trend = snapshot.trends.entry(process.pid).or_default();
                                trend.push_back(process.cpu);
                                if trend.len() > TREND {
                                    trend.pop_front();
                                }
                            }
                            snapshot.processes = list;
                            snapshot.error = None;
                        }
                        Err(error) => snapshot.error = Some(error),
                    }
                }
                ctx.request_repaint_of(ViewportId::from_hash_of("omniterm-processes"));
                let until = Instant::now() + INTERVAL;
                while Instant::now() < until && !stop.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        });
    }
}

/// Shows the window while it is open. `start` begins a read-only monitor of a process.
pub fn show(window: &mut ProcessWindow, ctx: &egui::Context, remotes: &Arc<crate::remote::manager::Manager>, hosts: &[(String, String)], mut start: impl FnMut(Option<String>, u64, String)) {
    if !window.open {
        window.stop();
        return;
    }
    window.ensure_sampler(remotes, ctx);
    let builder = ViewportBuilder::default().with_title("omniTerm 进程监视").with_inner_size([980.0, 680.0]).with_min_inner_size([640.0, 420.0]);
    ctx.show_viewport_immediate(ViewportId::from_hash_of("omniterm-processes"), builder, |ctx, _| {
        if ctx.input(|input| input.viewport().close_requested()) {
            window.open = false;
            return;
        }
        let palette = style::palette_of(ctx);
        egui::CentralPanel::default().frame(egui::Frame::new().fill(palette.surface()).inner_margin(14)).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("omniTerm · 进程监视").strong().size(13.0));
                ui.add_space(12.0);
                let current = window.host.as_ref().and_then(|id| hosts.iter().find(|(host, _)| host == id)).map(|(_, label)| label.clone()).unwrap_or_else(|| "本机".into());
                let mut chosen = window.host.clone();
                egui::ComboBox::from_id_salt("process-host").selected_text(current).show_ui(ui, |ui| {
                    ui.selectable_value(&mut chosen, None, "本机");
                    for (id, label) in hosts {
                        ui.selectable_value(&mut chosen, Some(id.clone()), label);
                    }
                });
                if chosen != window.host {
                    window.open_for(chosen);
                }
                ui.add(egui::TextEdit::singleline(&mut window.query).hint_text("按名称或 PID 搜索").desired_width(ui.available_width()));
            });
            ui.add_space(8.0);
            let snapshot = window.data.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(error) = &snapshot.error {
                ui.colored_label(palette.danger(), error);
            }
            let query = window.query.to_lowercase();
            let mut list: Vec<&Process> = snapshot.processes.iter().filter(|process| query.is_empty() || process.command.to_lowercase().contains(&query) || process.pid.to_string().starts_with(&query)).collect();
            match window.sort {
                Sort::Cpu => list.sort_by(|a, b| b.cpu.total_cmp(&a.cpu)),
                Sort::Memory => list.sort_by_key(|process| std::cmp::Reverse(process.memory_kb)),
                Sort::Pid => list.sort_by_key(|process| process.pid),
                Sort::Name => list.sort_by_key(|process| process.command.to_lowercase()),
            }
            let detail_height = if window.selected.is_some() { 120.0 } else { 0.0 };
            ui.horizontal(|ui| {
                for (sort, label, width) in [(Sort::Pid, "PID", 70.0), (Sort::Name, "名称", 300.0), (Sort::Cpu, "CPU %", 90.0), (Sort::Memory, "内存", 110.0)] {
                    let text = if window.sort == sort { format!("{label} ↓") } else { label.to_string() };
                    if ui.add_sized([width, 22.0], egui::Button::new(RichText::new(text).size(11.0).strong()).frame(false)).clicked() {
                        window.sort = sort;
                    }
                }
                ui.label(RichText::new("父进程").size(11.0).strong());
            });
            ui.separator();
            egui::ScrollArea::vertical().max_height(ui.available_height() - detail_height).auto_shrink([false, false]).show_rows(ui, 22.0, list.len(), |ui, rows| {
                for process in &list[rows] {
                    let selected = window.selected == Some(process.pid);
                    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::click());
                    if selected {
                        ui.painter().rect_filled(rect, 3.0, style::alpha(palette.tokens.accent, 0.16));
                    } else if response.hovered() {
                        ui.painter().rect_filled(rect, 3.0, palette.hover());
                    }
                    let cells = [(process.pid.to_string(), 0.0), (process.command.clone(), 76.0), (format!("{:.1}", process.cpu), 382.0), (crate::ui::remote_panel::format_bytes(process.memory_kb as f64 * 1024.0), 478.0), (process.ppid.to_string(), 594.0)];
                    for (text, x) in cells {
                        ui.painter().text(rect.left_center() + Vec2::new(x + 4.0, 0.0), egui::Align2::LEFT_CENTER, text, egui::FontId::monospace(11.0), palette.text_soft());
                    }
                    if response.clicked() {
                        window.selected = Some(process.pid);
                    }
                }
            });
            if let Some(pid) = window.selected
                && let Some(process) = snapshot.processes.iter().find(|process| process.pid == pid)
            {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(icon(icons::CPU, 16.0).color(palette.accent()));
                    ui.label(RichText::new(format!("{} · PID {}", process.command, process.pid)).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if style::primary_button(ui, &format!("{}  监控此进程", icons::STACK)).on_hover_text("创建只读监控：不会执行任何操作").clicked() {
                            start(window.host.clone(), process.pid, process.command.clone());
                        }
                    });
                });
                if let Some(trend) = snapshot.trends.get(&pid).filter(|trend| trend.len() > 1) {
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 48.0), Sense::hover());
                    let high = trend.iter().copied().fold(1.0, f64::max);
                    let points: Vec<egui::Pos2> = trend.iter().enumerate().map(|(index, value)| egui::pos2(rect.left() + rect.width() * index as f32 / (trend.len() - 1) as f32, rect.bottom() - (value / high) as f32 * rect.height())).collect();
                    ui.painter().add(egui::Shape::line(points, Stroke::new(1.5, palette.accent())));
                    ui.label(RichText::new(format!("CPU 最近 {} 次采样，峰值 {high:.1}%", trend.len())).size(10.0).color(palette.muted()));
                }
            }
        });
    });
}

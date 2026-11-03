// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! The monitoring panel: monitors, their trends and history, and the form that creates them.

use super::style::{self, icon};
use super::{App, Intent};
use crate::icons;
use crate::monitor::probe::{Platform, Target};
use crate::monitor::service::{Monitor, Status};
use eframe::egui::{self, Align, Layout, RichText, Sense, Stroke, Ui, Vec2};
use serde_json::{Value, json};

#[derive(Clone, PartialEq)]
pub struct Form {
    host: String,
    kind: &'static str,
    process: String,
    pid: String,
    tid: String,
    path: String,
    region: String,
    layout: String,
    field: String,
    value: String,
    interval: String,
    duration: String,
    with_action: bool,
    command: String,
    terminal: String,
    max_runs: String,
}

impl Default for Form {
    fn default() -> Self {
        Self { host: "local".into(), kind: "process", process: String::new(), pid: String::new(), tid: String::new(), path: String::new(), region: String::new(), layout: String::new(), field: String::new(), value: String::new(), interval: "5".into(), duration: "60".into(), with_action: false, command: String::new(), terminal: String::new(), max_runs: "1".into() }
    }
}

/// Mirrors what a host can read, so a choice that can't work is explained before it is made.
fn kind_note(kind: &str, local: bool) -> &'static str {
    if !local {
        return if matches!(kind, "thread" | "sharedMemory") { "需要 Linux 远程主机。" } else { "" };
    }
    match (Platform::local(), kind) {
        (Platform::Windows, "sharedMemory") => "Windows 本机暂不支持。",
        (Platform::Mac, "thread" | "sharedMemory") => "macOS 暂不支持。",
        _ => "",
    }
}

fn number(value: &str, label: &str) -> Result<f64, String> {
    value.trim().parse::<f64>().ok().filter(|n| n.is_finite()).ok_or_else(|| format!("请填写{label}。"))
}

fn request(form: &Form, terminals: &[Value]) -> Result<Value, String> {
    let target = match form.kind {
        "process" if !form.process.is_empty() && form.process.chars().all(|c| c.is_ascii_digit()) => json!({ "kind": "process", "pid": form.process.parse::<u64>().unwrap_or(0) }),
        "process" => json!({ "kind": "process", "name": form.process.trim() }),
        "thread" => json!({ "kind": "thread", "pid": number(&form.pid, "进程 ID")? as u64, "tid": number(&form.tid, "线程 ID")? as u64 }),
        "log" => json!({ "kind": "log", "path": form.path.trim() }),
        _ => {
            let layout: Vec<Value> = form
                .layout
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(|line| {
                    let parts: Vec<&str> = line.split(':').map(str::trim).collect();
                    json!({ "field": parts.first().copied().unwrap_or_default(), "offset": parts.get(1).and_then(|o| o.parse::<u64>().ok()), "type": parts.get(2).copied().unwrap_or_default() })
                })
                .collect();
            json!({ "kind": "sharedMemory", "name": form.region.trim(), "access": "posix-shm", "layout": layout })
        }
    };
    let conditions = match form.field.as_str() {
        "" => json!([]),
        "exists" => json!([{ "op": "gone" }]),
        "text" => json!([{ "field": "text", "op": "contains", "value": form.value }]),
        field => json!([{ "field": field, "op": ">", "value": number(&form.value, "条件数值")? }]),
    };
    let host = if form.host == "local" { json!("local") } else { json!({ "sessionId": form.host }) };
    let mut request = json!({ "host": host, "target": target, "conditions": conditions, "intervalSeconds": number(&form.interval, "采样间隔")?, "durationMinutes": number(&form.duration, "时长")? });
    if form.with_action {
        let terminal = terminals.iter().find(|terminal| terminal["ref"]["sessionId"].as_str() == Some(&form.terminal)).ok_or("请选择自动操作的目标终端。")?;
        request["action"] = json!({ "command": form.command, "terminal": terminal["ref"], "maxRuns": number(&form.max_runs, "最多执行次数")? as u64 });
    }
    Ok(request)
}

pub fn summary(target: &Target) -> String {
    match target {
        Target::Process { pid: Some(pid), .. } => format!("进程 {pid}"),
        Target::Process { name, .. } => format!("进程 {}", name.as_deref().unwrap_or_default()),
        Target::Thread { pid, tid } => format!("线程 {pid}/{tid}"),
        Target::Log { path } => path.clone(),
        Target::SharedMemory { name, .. } => format!("共享内存 {name}"),
    }
}

const HIDDEN: &[&str] = &["at", "exists", "text", "size", "offset", "nextOffset", "pid", "command", "cpuSeconds"];

fn metrics(monitor: &Monitor) -> Vec<(String, f64)> {
    let Some(sample) = &monitor.last_sample else { return Vec::new() };
    sample.iter().filter(|(key, _)| !HIDDEN.contains(&key.as_str())).filter_map(|(key, value)| Some((key.clone(), value.as_f64()?))).collect()
}

fn sparkline(ui: &mut Ui, monitor: &Monitor, key: &str) {
    let values: Vec<f64> = monitor.series.iter().filter_map(|sample| sample.get(key)?.as_f64()).collect();
    if values.len() < 2 {
        return;
    }
    let palette = style::palette(ui);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::hover());
    let (low, high) = values.iter().fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    let span = if high > low { high - low } else { 1.0 };
    let points: Vec<egui::Pos2> = values.iter().enumerate().map(|(index, value)| egui::pos2(rect.left() + rect.width() * index as f32 / (values.len() - 1) as f32, rect.bottom() - 1.0 - ((value - low) / span) as f32 * (rect.height() - 2.0))).collect();
    ui.painter().add(egui::Shape::line(points, Stroke::new(1.2, palette.accent())));
}

pub fn show(app: &mut App, ui: &mut Ui, intents: &mut Vec<Intent>) {
    let palette = style::palette(ui);
    for (label, message) in app.monitors.take_alerts() {
        intents.push(Intent::Notify(format!("{}：{message}", if label.is_empty() { "监控".into() } else { label }), false));
    }
    ui.horizontal(|ui| {
        ui.set_height(41.0);
        super::dock::grip(app, ui, super::dock::Panel::Monitor);
        ui.label(icon(icons::STACK, 17.0).color(palette.accent()));
        ui.label(RichText::new("监控").strong().size(12.5).color(palette.text()));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if style::icon_button(ui, icons::X, "隐藏监控面板", true).clicked() {
                app.monitor_visible = false;
            }
        });
    });
    ui.separator();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if app.monitor_form.is_none() && style::secondary_button(ui, &format!("{}  新建监控", icons::PLUS)).clicked() {
            app.monitor_form = Some(Form::default());
        }
        if app.monitor_form.is_some() {
            form(app, ui, intents);
        }
        let list = app.monitors.list();
        if list.is_empty() && app.monitor_form.is_none() {
            ui.add(egui::Label::new(RichText::new("直接读取主机上的进程、线程、日志或共享内存，不依赖终端屏幕，也不消耗模型调用。").size(11.0).color(palette.muted())).wrap());
        }
        for monitor in list {
            card(app, ui, &monitor, intents);
            ui.add_space(6.0);
        }
    });
}

fn card(app: &App, ui: &mut Ui, monitor: &Monitor, intents: &mut Vec<Intent>) {
    let palette = style::palette(ui);
    egui::Frame::new().fill(palette.panel_soft).stroke(Stroke::new(1.0, palette.border_strong)).corner_radius(6).inner_margin(8).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(if monitor.label.is_empty() { summary(&monitor.target) } else { monitor.label.clone() }).strong().size(11.5));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(monitor.status.label()).size(10.0).color(palette.accent()));
            });
        });
        let note = |ui: &mut Ui, text: String| {
            ui.add(egui::Label::new(RichText::new(text).size(10.5).color(palette.muted())).wrap());
        };
        note(ui, format!("{} · {} · 每 {} 秒{}", if monitor.host.is_none() { "本机" } else { "SSH" }, summary(&monitor.target), monitor.interval_seconds, if monitor.proposed_by == "agent" { " · Agent 提议" } else { "" }));
        for condition in &monitor.conditions {
            note(ui, if condition.op == "gone" { "条件：目标消失".into() } else { format!("条件：{} {} {}", condition.field, condition.op, condition.value.as_str().map(String::from).unwrap_or_else(|| condition.value.to_string())) });
        }
        if let Some(action) = &monitor.action {
            note(ui, format!("{}（最多 {} 次，已执行 {} 次）", if monitor.status == Status::Pending { "批准后可自动执行：" } else { "自动操作：" }, action.max_runs, monitor.action_runs));
            egui::Frame::new().fill(palette.surface()).corner_radius(4).inner_margin(4).show(ui, |ui| {
                ui.label(RichText::new(&action.command).monospace().size(10.5));
            });
        }
        let values = metrics(monitor);
        if monitor.last_sample.as_ref().and_then(|sample| sample.get("exists")) == Some(&Value::Bool(false)) {
            note(ui, "目标 不存在".into());
        } else if !values.is_empty() {
            ui.label(RichText::new(values.iter().map(|(key, value)| format!("{key} {}", (value * 100.0).round() / 100.0)).collect::<Vec<_>>().join(" · ")).monospace().size(10.5));
            sparkline(ui, monitor, &values[0].0);
        }
        egui::CollapsingHeader::new(RichText::new(format!("操作记录（{}）", monitor.history.len())).size(10.0)).id_salt(("history", &monitor.id)).show(ui, |ui| {
            for entry in monitor.history.iter().rev().take(12) {
                ui.label(RichText::new(&entry.message).size(10.0).color(palette.muted()));
            }
        });
        let live = matches!(monitor.status, Status::Running | Status::Paused);
        let id = monitor.id.clone();
        ui.horizontal_wrapped(|ui| {
            let monitors = app.monitors.clone();
            if monitor.status == Status::Pending {
                if style::primary_button(ui, &format!("{}  批准", icons::CHECK)).clicked() {
                    let id = id.clone();
                    std::thread::spawn(move || {
                        let _ = monitors.approve(&id);
                    });
                }
                if style::ghost_button(ui, "拒绝").clicked() {
                    app.monitors.deny(&id);
                }
            }
            if monitor.status == Status::Running && style::ghost_button(ui, &format!("{}  暂停", icons::PAUSE)).clicked() {
                app.monitors.pause(&id);
            }
            if monitor.status == Status::Paused && style::ghost_button(ui, &format!("{}  继续", icons::PLAY)).clicked() {
                let monitors = app.monitors.clone();
                let id = id.clone();
                std::thread::spawn(move || monitors.resume(&id));
            }
            if live && monitor.action.is_some() && style::ghost_button(ui, "撤销自动操作").clicked() {
                app.monitors.revoke(&id);
            }
            if live && style::ghost_button(ui, &format!("{}  停止", icons::STOP)).clicked() {
                app.monitors.stop(&id);
            }
            if !live && monitor.status != Status::Pending && style::ghost_button(ui, &format!("{}  删除", icons::TRASH)).clicked() && let Err(error) = app.monitors.remove(&id) {
                intents.push(Intent::Notify(error, true));
            }
        });
    });
}

fn form(app: &mut App, ui: &mut Ui, intents: &mut Vec<Intent>) {
    let palette = style::palette(ui);
    let terminals: Vec<Value> = app.backend.request("workspace.list", json!({})).ok().and_then(|list| list["terminals"].as_array().cloned()).unwrap_or_default();
    let Some(form) = app.monitor_form.as_mut() else { return };
    let mut done = false;
    let label = |ui: &mut Ui, text: &str| {
        ui.label(RichText::new(text).size(10.5).color(palette.muted()));
    };
    egui::Frame::new().fill(palette.panel_soft).corner_radius(6).inner_margin(8).show(ui, |ui| {
        label(ui, "主机");
        let hosts: Vec<&Value> = terminals.iter().filter(|terminal| terminal["kind"].as_str() == Some("ssh")).collect();
        let host_name = if form.host == "local" { "本机".to_string() } else { hosts.iter().find(|t| t["ref"]["sessionId"].as_str() == Some(&form.host)).and_then(|t| t["label"].as_str()).unwrap_or("SSH").to_string() };
        egui::ComboBox::from_id_salt("monitor-host").width(ui.available_width()).selected_text(host_name).show_ui(ui, |ui| {
            ui.selectable_value(&mut form.host, "local".into(), "本机");
            for terminal in &hosts {
                let id = terminal["ref"]["sessionId"].as_str().unwrap_or_default().to_string();
                ui.selectable_value(&mut form.host, id, terminal["label"].as_str().unwrap_or_default());
            }
        });
        label(ui, "监控对象");
        let kinds = [("process", "进程"), ("thread", "线程"), ("log", "日志文件"), ("sharedMemory", "共享内存（只读）")];
        egui::ComboBox::from_id_salt("monitor-kind").width(ui.available_width()).selected_text(kinds.iter().find(|(k, _)| *k == form.kind).map(|(_, l)| *l).unwrap_or_default()).show_ui(ui, |ui| {
            for (kind, text) in kinds {
                ui.selectable_value(&mut form.kind, kind, text);
            }
        });
        let note = kind_note(form.kind, form.host == "local");
        if !note.is_empty() {
            ui.label(RichText::new(note).size(10.5).color(palette.danger()));
        }
        let field = |ui: &mut Ui, text: &str, value: &mut String, hint: &str| {
            label(ui, text);
            ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(f32::INFINITY));
        };
        match form.kind {
            "process" => field(ui, "进程 ID 或进程名", &mut form.process, ""),
            "thread" => {
                field(ui, "进程 ID", &mut form.pid, "");
                field(ui, "线程 ID", &mut form.tid, "");
            }
            "log" => field(ui, "日志路径", &mut form.path, if form.host == "local" && cfg!(windows) { "C:\\logs\\app.log" } else { "/var/log/app.log" }),
            _ => {
                field(ui, "区域名称（/dev/shm 下）", &mut form.region, "");
                label(ui, "布局（每行：字段:偏移:类型，例如 count:0:u32）");
                ui.add(egui::TextEdit::multiline(&mut form.layout).desired_rows(3).desired_width(f32::INFINITY));
            }
        }
        let fields: Vec<String> = match form.kind {
            "process" => vec!["cpu".into(), "memoryKb".into(), "threads".into()],
            "thread" => vec!["cpu".into()],
            "log" => vec!["text".into()],
            _ => form.layout.lines().filter_map(|line| line.split(':').next()).map(|f| f.trim().to_string()).filter(|f| !f.is_empty()).collect(),
        };
        label(ui, "条件字段");
        egui::ComboBox::from_id_salt("monitor-field").width(ui.available_width()).selected_text(match form.field.as_str() { "" => "不设置".to_string(), "exists" => "目标消失".to_string(), other => other.to_string() }).show_ui(ui, |ui| {
            ui.selectable_value(&mut form.field, String::new(), "不设置");
            for item in &fields {
                ui.selectable_value(&mut form.field, item.clone(), item);
            }
            ui.selectable_value(&mut form.field, "exists".into(), "目标消失");
        });
        if !form.field.is_empty() && form.field != "exists" {
            let text = if form.field == "text" { "包含文本" } else { "大于" };
            field(ui, text, &mut form.value, "");
        }
        ui.horizontal(|ui| {
            ui.vertical(|ui| field(ui, "采样间隔（秒）", &mut form.interval, "5"));
            ui.vertical(|ui| field(ui, "时长（分钟）", &mut form.duration, "60"));
        });
        ui.checkbox(&mut form.with_action, "条件满足时自动执行一条命令");
        if form.with_action {
            field(ui, "命令（固定不变，Agent 不能修改）", &mut form.command, "");
            label(ui, "目标终端");
            let targets: Vec<&Value> = terminals.iter().filter(|t| matches!(t["kind"].as_str(), Some("shell" | "ssh")) && t["status"].as_str() == Some("running")).collect();
            let selected = targets.iter().find(|t| t["ref"]["sessionId"].as_str() == Some(&form.terminal)).map(|t| format!("{} · {}", t["label"].as_str().unwrap_or_default(), t["shortId"].as_str().unwrap_or_default())).unwrap_or_else(|| "请选择".into());
            egui::ComboBox::from_id_salt("monitor-terminal").width(ui.available_width()).selected_text(selected).show_ui(ui, |ui| {
                for terminal in targets {
                    let id = terminal["ref"]["sessionId"].as_str().unwrap_or_default().to_string();
                    ui.selectable_value(&mut form.terminal, id, format!("{} · {}", terminal["label"].as_str().unwrap_or_default(), terminal["shortId"].as_str().unwrap_or_default()));
                }
            });
            field(ui, "最多执行次数", &mut form.max_runs, "1");
            ui.add(egui::Label::new(RichText::new("批准后，在设定时长内，这条命令可以不经确认地执行。它仍要通过就绪检查；未开启“自动执行”的 SSH 终端不会执行。").size(10.5).color(palette.muted())).wrap());
        }
        ui.horizontal(|ui| {
            if style::primary_button(ui, &format!("{}  创建", icons::PLUS)).clicked() {
                match request(form, &terminals).and_then(|request| app.monitors.propose(&request)) {
                    Ok(_) => done = true,
                    Err(error) => intents.push(Intent::Notify(error, true)),
                }
            }
            if style::ghost_button(ui, "取消").clicked() {
                done = true;
            }
        });
    });
    if done {
        app.monitor_form = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_form_builds_the_desktop_apps_request() {
        let form = Form { process: "nginx".into(), field: "cpu".into(), value: "80".into(), ..Form::default() };
        let built = request(&form, &[]).unwrap();
        assert_eq!(built["target"], json!({ "kind": "process", "name": "nginx" }));
        assert_eq!(built["conditions"][0]["op"], json!(">"));
        assert!(request(&Form { with_action: true, ..form }, &[]).unwrap_err().contains("目标终端"));
    }
}

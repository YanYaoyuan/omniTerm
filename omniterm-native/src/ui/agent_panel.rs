// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! The omniTerm Agent panel: the conversation, approval cards, which terminals are shared and
//! which one may run commands, and model provider settings.

use super::style::{self, alpha, icon, icon_button};
use super::{App, Intent};
use crate::agent::providers::{self, Auth, Profile, WireApi};
use crate::agent::runtime::Event;
use crate::agent::service::AgentEvent;
use crate::icons;
use eframe::egui::{self, Align, Key, Layout, RichText, Stroke, Ui};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(1500);
const GRANT_MS: u64 = 10 * 60 * 1000;
const OPEN_OPERATIONS: &[&str] = &["pendingApproval", "approved", "queued", "running"];

fn operation_label(status: &str) -> &str {
    match status {
        "pendingApproval" => "等待批准",
        "approved" => "已批准",
        "queued" => "排队中",
        "running" => "执行中",
        "staged" => "已输入，等待手动提交",
        "delivered" => "已发送给程序",
        "completed" => "已完成",
        "denied" => "已拒绝",
        "cancelled" => "已取消",
        "invalidated" => "已失效",
        "blocked" => "未发送",
        "interrupted" => "已中断",
        "takenOver" => "已由用户接管",
        "outcomeUnknown" => "结果未知",
        other => other,
    }
}

fn interaction_label(state: &str) -> &str {
    match state {
        "ready" => "就绪",
        "busy" => "忙碌",
        "fullscreen" => "全屏程序",
        "password" => "密码提示",
        "exited" => "已退出",
        "unsupported" => "不支持命令",
        _ => "状态未知",
    }
}

/// Broker reasons are written for the model; these say what they mean to the person at the card.
fn operation_reason(reason: &str) -> String {
    const REASONS: &[(&str, &str)] = &[
        ("full-screen", "目标终端正在运行全屏程序，shell 命令没有发送。如需向这个程序输入，请让 Agent 使用“发送给运行中的程序”。"),
        ("not program input", "终端在 shell 提示符，这不是程序输入，文本没有发送。"),
        ("not sent with Enter", "无法确认终端状态，它可能是 shell，所以没有发送。可以选择“只发送文本”。"),
        ("the text was not sent", "目标终端正在等待密码，文本没有发送。"),
        ("password", "目标终端正在等待密码，命令没有发送。"),
        ("unsent text", "终端输入行上还有未提交的内容，命令没有发送。请先按回车提交或按 Ctrl+C 清除。"),
        ("stayed busy", "终端一直处于忙碌状态，命令没有发送。如果它已回到 shell 提示符，请在终端按一次回车后重试。"),
        ("cannot be confirmed", "无法确认终端状态，命令没有提交。请选择“输入但不回车”，由你检查后按回车。"),
    ];
    REASONS.iter().find(|(pattern, _)| reason.contains(pattern)).map(|(_, text)| text.to_string()).unwrap_or_else(|| reason.to_string())
}

enum Entry {
    User(String),
    Assistant { id: String, text: String },
    Reasoning { id: String, text: String },
    Tool { id: String, tool: String, status: String, success: Option<bool> },
    Context { terminals: Vec<String>, unavailable: usize, provider: String, model: String },
    Operation(Value),
    Error { text: String, detail: Option<String> },
    Notice(String),
}

pub struct ProviderForm {
    profile: Profile,
    key: String,
    editing: bool,
    error: Option<String>,
    testing: bool,
    result: std::sync::Arc<std::sync::Mutex<Option<Result<(bool, bool), String>>>>,
}

impl ProviderForm {
    fn new(profile: Option<Profile>) -> Self {
        let editing = profile.is_some();
        let profile = profile.unwrap_or_else(|| Profile { id: String::new(), preset_id: "openai".into(), name: "OpenAI".into(), auth: Auth::ApiKey, wire_api: WireApi::Responses, base_url: "https://api.openai.com/v1".into(), model: String::new(), chat_only: false, context_budget_bytes: 64 * 1024, verified: Default::default() });
        Self { profile, key: String::new(), editing, error: None, testing: false, result: Default::default() }
    }
}

#[derive(Default)]
pub struct AgentState {
    entries: Vec<Entry>,
    running: bool,
    retrying: bool,
    terminals: Vec<Value>,
    polled: Option<Instant>,
    context: BTreeSet<String>,
    targets: BTreeSet<String>,
    context_touched: bool,
    attached: HashMap<String, String>,
    pub profile: Option<String>,
    providers: Vec<Profile>,
    providers_loaded: bool,
    picker: bool,
    paused: bool,
    text: String,
    pub form: Option<ProviderForm>,
    settings: bool,
    scroll_to_end: bool,
}

impl AgentState {
    pub fn with_profile(profile: Option<String>) -> Self {
        Self { profile, ..Self::default() }
    }

    fn apply(&mut self, event: AgentEvent) {
        let find = |entries: &mut Vec<Entry>, matches: &dyn Fn(&Entry) -> bool| entries.iter_mut().position(|entry| matches(entry));
        self.scroll_to_end = true;
        match event {
            AgentEvent::Engine(Event::TurnStarted(_)) => self.running = true,
            AgentEvent::Engine(Event::TextDelta { item, delta }) => match find(&mut self.entries, &|entry| matches!(entry, Entry::Assistant { id, .. } if *id == item)) {
                Some(index) => {
                    if let Entry::Assistant { text, .. } = &mut self.entries[index] {
                        text.push_str(&delta);
                    }
                }
                None => self.entries.push(Entry::Assistant { id: item, text: delta }),
            },
            AgentEvent::Engine(Event::TextDone { item, text: done }) => match find(&mut self.entries, &|entry| matches!(entry, Entry::Assistant { id, .. } if *id == item)) {
                Some(index) => self.entries[index] = Entry::Assistant { id: item, text: done },
                None => self.entries.push(Entry::Assistant { id: item, text: done }),
            },
            AgentEvent::Engine(Event::ReasoningDelta { item, delta }) => match find(&mut self.entries, &|entry| matches!(entry, Entry::Reasoning { id, .. } if *id == item)) {
                Some(index) => {
                    if let Entry::Reasoning { text, .. } = &mut self.entries[index] {
                        text.push_str(&delta);
                    }
                }
                None => self.entries.push(Entry::Reasoning { id: item, text: delta }),
            },
            AgentEvent::Engine(Event::Tool { item, tool, status, success }) => {
                let entry = Entry::Tool { id: item.clone(), tool, status, success };
                match find(&mut self.entries, &|entry| matches!(entry, Entry::Tool { id, .. } if *id == item)) {
                    Some(index) => self.entries[index] = entry,
                    None => self.entries.push(entry),
                }
            }
            AgentEvent::Engine(Event::TurnCompleted { .. }) => {
                self.running = false;
                self.retrying = false;
            }
            AgentEvent::Engine(Event::Retrying) => self.retrying = true,
            AgentEvent::Engine(Event::Error(message)) => self.entries.push(Entry::Error { text: message, detail: None }),
            AgentEvent::Engine(Event::Forbidden(_)) => {}
            AgentEvent::ContextShared { terminals, unavailable, provider, model } => self.entries.push(Entry::Context { terminals, unavailable, provider, model }),
            AgentEvent::Operation(operation) => {
                let id = operation["operationId"].as_str().unwrap_or_default().to_string();
                match find(&mut self.entries, &|entry| matches!(entry, Entry::Operation(op) if op["operationId"].as_str() == Some(&id))) {
                    Some(index) => self.entries[index] = Entry::Operation(operation),
                    None => self.entries.push(Entry::Operation(operation)),
                }
            }
            AgentEvent::Notice(text) => self.entries.push(Entry::Notice(text)),
            AgentEvent::Error { message, detail } => {
                self.entries.push(Entry::Error { text: message, detail });
                self.running = false;
                self.retrying = false;
            }
            AgentEvent::Reset => {
                self.running = false;
                self.retrying = false;
            }
        }
    }

    fn refs(&self, ids: &BTreeSet<String>) -> Vec<Value> {
        self.terminals.iter().filter(|terminal| terminal["ref"]["sessionId"].as_str().is_some_and(|id| ids.contains(id))).map(|terminal| terminal["ref"].clone()).collect()
    }

    fn scope(&self, ids: &BTreeSet<String>) -> String {
        let names: Vec<&str> = self.terminals.iter().filter(|terminal| terminal["ref"]["sessionId"].as_str().is_some_and(|id| ids.contains(id))).filter_map(|terminal| terminal["label"].as_str()).collect();
        match names.len() {
            0 => "无".into(),
            1 | 2 => names.join("、"),
            count => format!("{count} 个终端"),
        }
    }

    /// Attaches a terminal selection to the next message; nothing is sent until submitted.
    pub fn attach(&mut self, id: &str, selection: String) {
        self.context_touched = true;
        self.context.insert(id.to_string());
        self.attached.insert(id.to_string(), selection);
    }
}

fn executable(terminal: &Value) -> bool {
    matches!(terminal["kind"].as_str(), Some("shell" | "ssh")) && terminal["status"].as_str() == Some("running")
}

fn terminal_name(terminal: &Value) -> String {
    format!("{} · {}", terminal["label"].as_str().unwrap_or_default(), terminal["shortId"].as_str().unwrap_or_default())
}

pub fn show(app: &mut App, ui: &mut Ui, intents: &mut Vec<Intent>) {
    let palette = style::palette(ui);
    for event in app.agent.take_events() {
        app.agent_state.apply(event);
    }
    let focused = app.focused().map(|pane| pane.id().to_string());
    let state = &mut app.agent_state;
    if !state.providers_loaded {
        state.providers_loaded = true;
        state.providers = app.agent.providers.list().unwrap_or_default();
    }
    // The terminal list refreshes while the panel is shown; closed terminals leave the scope.
    if state.polled.is_none_or(|at| at.elapsed() >= POLL) {
        state.polled = Some(Instant::now());
        if let Ok(list) = app.backend.request("workspace.list", json!({})) {
            state.terminals = list["terminals"].as_array().cloned().unwrap_or_default();
            let live: BTreeSet<String> = state.terminals.iter().filter_map(|terminal| terminal["ref"]["sessionId"].as_str().map(String::from)).collect();
            state.context.retain(|id| live.contains(id));
            state.targets.retain(|id| live.contains(id));
            state.attached.retain(|id, _| live.contains(id));
        }
        ui.ctx().request_repaint_after(POLL);
    }
    // Until the user chooses a scope, it follows the focused terminal. Execution never does.
    if !state.context_touched
        && let Some(focused) = focused
        && state.terminals.iter().any(|terminal| terminal["ref"]["sessionId"].as_str() == Some(&focused))
    {
        state.context = [focused].into();
    }
    let profile = state.profile.as_ref().and_then(|id| state.providers.iter().find(|profile| profile.id == *id)).or(state.providers.first()).cloned();

    // Heading.
    ui.horizontal(|ui| {
        ui.set_height(41.0);
        ui.add_space(6.0);
        ui.label(icon(icons::ROBOT, 17.0).color(palette.accent()));
        ui.label(RichText::new("omniTerm Agent").strong().size(12.5).color(palette.text()));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if icon_button(ui, icons::X, "隐藏 omniTerm Agent 面板（对话继续运行）", true).clicked() {
                intents.push(Intent::Action(super::shortcuts::Action::ToggleAgent));
            }
            if icon_button(ui, icons::ARROW_CLOCKWISE, "新建对话", true).clicked() {
                app.agent.reset();
                app.agent_state.entries.clear();
            }
            if icon_button(ui, icons::GEAR_SIX, "模型设置", true).clicked() {
                app.agent_state.settings = true;
            }
            let state = &mut app.agent_state;
            if !state.providers.is_empty() {
                let current = profile.as_ref().map(|profile| profile.name.clone()).unwrap_or_default();
                egui::ComboBox::from_id_salt("agent-profile").width(120.0).selected_text(RichText::new(current).size(11.0)).show_ui(ui, |ui| {
                    for item in &state.providers {
                        if ui.selectable_label(Some(&item.id) == profile.as_ref().map(|p| &p.id), &item.name).clicked() {
                            state.profile = Some(item.id.clone());
                        }
                    }
                });
            }
        });
    });
    ui.separator();
    let state = &mut app.agent_state;
    if state.providers.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(30.0);
            ui.label(icon(icons::ROBOT, 28.0).color(palette.faint()));
            ui.label(RichText::new("先配置一个模型服务").size(13.0).color(palette.text_soft()));
            ui.add(egui::Label::new(RichText::new("支持 OpenAI、Claude、Qwen、Kimi、OpenRouter 和自定义端点。API Key 保存在系统钥匙串中。").size(11.0).color(palette.muted())).wrap());
            ui.add_space(8.0);
            if style::primary_button(ui, "配置模型服务").clicked() {
                state.form = Some(ProviderForm::new(None));
            }
        });
    } else {
        conversation(app, ui, intents);
        composer(app, ui, profile.as_ref());
    }
    providers_dialog(app, ui.ctx());
}

fn conversation(app: &mut App, ui: &mut Ui, intents: &mut Vec<Intent>) {
    let palette = style::palette(ui);
    let height = ui.available_height() - if app.agent_state.picker { 290.0 } else { 140.0 };
    let scroll = std::mem::take(&mut app.agent_state.scroll_to_end);
    egui::ScrollArea::vertical().max_height(height.max(80.0)).auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
        let state = &app.agent_state;
        if state.entries.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(24.0);
                ui.add(egui::Label::new(RichText::new("问我终端里发生了什么，或让我帮你执行命令。每条命令都会先请你批准。").size(11.5).color(palette.muted())).wrap());
            });
        }
        let mut decisions = Vec::new();
        for entry in &state.entries {
            match entry {
                Entry::User(text) => {
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        egui::Frame::new().fill(alpha(palette.tokens.accent, 0.14)).corner_radius(10).inner_margin(egui::Margin::symmetric(10, 7)).show(ui, |ui| {
                            ui.set_max_width(ui.available_width() * 0.85);
                            ui.add(egui::Label::new(RichText::new(text).size(12.0).color(palette.text())).wrap().selectable(true));
                        });
                    });
                }
                Entry::Assistant { text, .. } => {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        for part in citations(text, &state.terminals) {
                            match part {
                                Ok(text) => {
                                    ui.add(egui::Label::new(RichText::new(text).size(12.0).color(palette.text_soft())).selectable(true));
                                }
                                Err((short, session)) => {
                                    if ui.link(RichText::new(short).monospace().size(11.0)).on_hover_text("定位到这个终端").clicked() {
                                        intents.push(Intent::Focus(session));
                                    }
                                }
                            }
                        }
                    });
                }
                Entry::Reasoning { text, .. } => {
                    egui::CollapsingHeader::new(RichText::new("思考过程").size(10.5).color(palette.faint())).id_salt(text.len()).show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(text).size(10.5).color(palette.muted())).wrap());
                    });
                }
                Entry::Tool { tool, status, success, .. } => {
                    ui.horizontal(|ui| {
                        if status == "inProgress" {
                            ui.add(egui::Spinner::new().size(11.0));
                        } else {
                            let (glyph, color) = if *success == Some(false) { (icons::WARNING_CIRCLE, palette.danger()) } else { (icons::CHECK, palette.faint()) };
                            ui.label(icon(glyph, 12.0).color(color));
                        }
                        ui.label(RichText::new(tool).monospace().size(10.5).color(palette.faint()));
                    });
                }
                Entry::Context { terminals, unavailable, provider, model } => {
                    let missing = if *unavailable > 0 { format!("；{unavailable} 个已关闭") } else { String::new() };
                    ui.horizontal_wrapped(|ui| {
                        ui.label(icon(icons::STACK, 13.0).color(palette.faint()));
                        ui.label(RichText::new(format!("已向 {provider} · {model} 共享：{}{missing}", terminals.join("、"))).size(10.5).color(palette.faint()));
                    });
                }
                Entry::Operation(operation) => card(ui, operation, &state.terminals, &mut decisions),
                Entry::Error { text, detail } => {
                    egui::Frame::new().fill(alpha(palette.tokens.danger, 0.08)).corner_radius(6).inner_margin(8).show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(icon(icons::WARNING_CIRCLE, 15.0).color(palette.danger()));
                            ui.add(egui::Label::new(RichText::new(text).size(11.5)).wrap());
                        });
                        if let Some(detail) = detail {
                            egui::CollapsingHeader::new(RichText::new("技术详情").size(10.0)).id_salt(("error", detail.len())).show(ui, |ui| {
                                ui.add(egui::Label::new(RichText::new(detail).monospace().size(9.5)).wrap().selectable(true));
                            });
                        }
                    });
                }
                Entry::Notice(text) => {
                    ui.add(egui::Label::new(RichText::new(text).size(10.5).italics().color(palette.muted())).wrap());
                }
            }
            ui.add_space(6.0);
        }
        if state.running {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(12.0));
                ui.label(RichText::new(if state.retrying { "模型服务繁忙，正在重试…" } else { "正在思考…" }).size(10.5).color(palette.muted()));
            });
        }
        if scroll {
            ui.scroll_to_cursor(Some(Align::Max));
        }
        for (method, params) in decisions {
            if let Err(error) = app.backend.request(method, params) {
                intents.push(Intent::Notify(error, true));
            }
        }
    });
}

/// Splits model text around terminal short IDs it knows, so citations can link to them.
fn citations(text: &str, terminals: &[Value]) -> Vec<Result<String, (String, String)>> {
    let ids: HashMap<&str, &str> = terminals.iter().filter_map(|terminal| Some((terminal["shortId"].as_str()?, terminal["ref"]["sessionId"].as_str()?))).collect();
    let mut parts = Vec::new();
    let mut last = 0;
    let bytes = text.as_bytes();
    let mut index = 0;
    while index + 8 <= bytes.len() {
        let word = &text[index..index + 8];
        let boundary = |at: usize| at >= bytes.len() || !bytes[at].is_ascii_alphanumeric();
        let starts = index == 0 || !bytes[index - 1].is_ascii_alphanumeric();
        if starts && boundary(index + 8) && word.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) && let Some(session) = ids.get(word) {
            if index > last {
                parts.push(Ok(text[last..index].to_string()));
            }
            parts.push(Err((word.to_string(), session.to_string())));
            index += 8;
            last = index;
            continue;
        }
        index += text[index..].chars().next().map(char::len_utf8).unwrap_or(1);
    }
    if last < text.len() {
        parts.push(Ok(text[last..].to_string()));
    }
    parts
}

/// An approval card. Decisions are host requests made on the person's click.
fn card(ui: &mut Ui, operation: &Value, terminals: &[Value], decisions: &mut Vec<(&'static str, Value)>) {
    let palette = style::palette(ui);
    let id = operation["operationId"].clone();
    let status = operation["status"].as_str().unwrap_or_default();
    let target = terminals.iter().find(|terminal| terminal["ref"]["sessionId"] == operation["target"]["sessionId"]);
    let name = target.map(terminal_name).unwrap_or_else(|| format!("{}（已关闭）", &operation["target"]["sessionId"].as_str().unwrap_or_default()[..8.min(operation["target"]["sessionId"].as_str().unwrap_or_default().len())]));
    let interaction = target.and_then(|terminal| terminal["interaction"]["state"].as_str()).unwrap_or("unknown");
    let pending = status == "pendingApproval";
    let program = operation["kind"].as_str() == Some("programInput");
    let approve = |scope: &str, mode: &str| {
        let mut params = json!({ "operationId": id, "scope": scope, "mode": mode });
        if scope == "taskTarget" {
            params["ttlMs"] = json!(GRANT_MS);
        }
        ("operation.approve", params)
    };
    egui::Frame::new().fill(palette.panel_soft).stroke(Stroke::new(1.0, palette.border_strong)).corner_radius(6).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(icon(if program { icons::KEYBOARD } else { icons::CROSSHAIR }, 14.0).color(palette.muted()));
            ui.label(RichText::new(if program { format!("发送给运行中的程序 · {name}") } else { format!("执行于 {name}") }).size(11.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(operation_label(status)).size(10.0).color(palette.accent()));
            });
        });
        if let Some(rationale) = operation["rationale"].as_str().filter(|text| !text.is_empty()) {
            ui.add(egui::Label::new(RichText::new(rationale).size(11.0).color(palette.text_soft())).wrap());
        }
        egui::Frame::new().fill(palette.surface()).corner_radius(4).inner_margin(6).show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(operation["command"].as_str().unwrap_or_default()).monospace().size(11.0)).wrap().selectable(true));
        });
        if let Some(reason) = operation["reason"].as_str() {
            ui.add(egui::Label::new(RichText::new(operation_reason(reason)).size(10.5).color(palette.muted())).wrap());
        }
        if let Some(code) = operation["exitCode"].as_i64() {
            ui.label(RichText::new(format!("退出码 {code}")).size(10.5).color(palette.muted()));
        }
        if pending {
            if program {
                let shell_prompt = interaction == "ready";
                let enter_allowed = matches!(interaction, "busy" | "fullscreen");
                let suggestion = if operation["appendEnter"].as_bool() == Some(true) { "Agent 建议按回车。" } else { "Agent 建议不按回车。" };
                ui.add(egui::Label::new(RichText::new(format!("这不是 shell 命令：文本会输入给 {name} 中正在运行的程序。{suggestion}{}", if shell_prompt { " 该终端现在在 shell 提示符，不会发送。" } else { "" })).size(10.5).color(palette.muted())).wrap());
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(!shell_prompt, egui::Button::new("只发送文本")).clicked() {
                        decisions.push(approve("once", "stage"));
                    }
                    if ui.add_enabled(enter_allowed, egui::Button::new("发送并按回车")).on_disabled_hover_text("无法确认正在运行程序时不能按回车").clicked() {
                        decisions.push(approve("once", "run"));
                    }
                    if ui.button(format!("{}  拒绝", icons::X)).clicked() {
                        decisions.push(("operation.deny", json!({ "operationId": id })));
                    }
                });
            } else {
                let auto = target.is_some_and(|terminal| terminal["autoSubmit"].as_bool() == Some(true));
                let can_run = interaction == "ready" && auto;
                let manual = if interaction == "fullscreen" {
                    "全屏程序不是 shell，shell 命令无法执行。如需向这个程序输入，请让 Agent 使用“发送给运行中的程序”。"
                } else if target.is_some_and(|terminal| terminal["autoSubmit"].as_bool() == Some(false)) {
                    "该终端的命令需要你检查后手动按回车提交。"
                } else {
                    ""
                };
                ui.label(RichText::new(format!("终端状态：{}{}", interaction_label(interaction), if manual.is_empty() { String::new() } else { format!("。{manual}") })).size(10.5).color(palette.muted()));
                ui.horizontal_wrapped(|ui| {
                    if can_run && style::primary_button(ui, &format!("{}  批准执行", icons::CHECK)).clicked() {
                        decisions.push(approve("once", "run"));
                    }
                    if can_run && style::secondary_button(ui, &format!("{}  对此目标放行 10 分钟", icons::SHIELD_CHECK)).on_hover_text("10 分钟内，本任务发往这个终端的命令不再询问").clicked() {
                        decisions.push(approve("taskTarget", "run"));
                    }
                    if (if can_run { style::ghost_button(ui, "输入但不回车") } else { style::primary_button(ui, "输入但不回车") }).clicked() {
                        decisions.push(approve("once", "stage"));
                    }
                    if style::ghost_button(ui, &format!("{}  拒绝", icons::X)).clicked() {
                        decisions.push(("operation.deny", json!({ "operationId": id })));
                    }
                });
            }
        } else if OPEN_OPERATIONS.contains(&status) && style::ghost_button(ui, &format!("{}  {}", icons::STOP, if status == "running" { "中断（Ctrl+C）" } else { "取消" })).clicked() {
            decisions.push(("operation.cancel", json!({ "operationId": id })));
        }
    });
}

fn composer(app: &mut App, ui: &mut Ui, profile: Option<&Profile>) {
    let palette = style::palette(ui);
    let cwd = app.focused().map(|pane| pane.cwd().to_string()).unwrap_or_else(|| app.cwd.clone());
    let state = &mut app.agent_state;
    ui.separator();
    // Scope: which terminals are shared, and which one may run commands.
    let summary = format!("上下文：{} · 执行：{}", state.scope(&state.context), if state.targets.is_empty() { "只读".into() } else { state.scope(&state.targets) });
    if ui.add(egui::Button::new(RichText::new(format!("{}  {summary}  {}", icons::STACK, if state.picker { icons::CARET_DOWN } else { icons::CARET_RIGHT })).size(10.5).color(palette.muted())).frame(false)).clicked() {
        state.picker = !state.picker;
    }
    if state.picker {
        egui::Frame::new().fill(palette.panel_soft).corner_radius(6).inner_margin(8).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("终端").size(10.0).color(palette.faint()));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new("执行").size(10.0).color(palette.faint()));
                    ui.add_space(8.0);
                    ui.label(RichText::new("上下文").size(10.0).color(palette.faint()));
                });
            });
            egui::ScrollArea::vertical().id_salt("picker").max_height(150.0).show(ui, |ui| {
                for terminal in state.terminals.clone() {
                    let Some(id) = terminal["ref"]["sessionId"].as_str().map(String::from) else { continue };
                    ui.horizontal(|ui| {
                        let glyph = if terminal["kind"].as_str() == Some("shell") { icons::TERMINAL_WINDOW } else { icons::HARD_DRIVES };
                        ui.label(icon(glyph, 14.0).color(palette.muted()));
                        ui.label(RichText::new(terminal_name(&terminal)).size(11.0));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let mut target = state.targets.contains(&id);
                            if ui.add_enabled(executable(&terminal), egui::RadioButton::new(target, "")).clicked() {
                                target = !target;
                                state.targets = if target { [id.clone()].into() } else { BTreeSet::new() };
                            }
                            ui.add_space(14.0);
                            let mut shared = state.context.contains(&id);
                            if ui.checkbox(&mut shared, "").changed() {
                                state.context_touched = true;
                                if shared {
                                    state.context.insert(id.clone());
                                } else {
                                    state.context.remove(&id);
                                }
                            }
                        });
                    });
                }
            });
            ui.horizontal(|ui| {
                if ui.link(RichText::new("选择全部当前终端").size(10.5)).clicked() {
                    state.context_touched = true;
                    state.context = state.terminals.iter().filter_map(|terminal| terminal["ref"]["sessionId"].as_str().map(String::from)).collect();
                }
                if ui.add_enabled(!state.targets.is_empty(), egui::Link::new(RichText::new("只读（无执行目标）").size(10.5))).clicked() {
                    state.targets.clear();
                }
            });
        });
    }
    if !state.attached.is_empty() {
        ui.horizontal_wrapped(|ui| {
            let mut remove = None;
            for id in state.attached.keys() {
                let name = state.terminals.iter().find(|terminal| terminal["ref"]["sessionId"].as_str() == Some(id)).map(terminal_name).unwrap_or_default();
                if ui.add(egui::Button::new(RichText::new(format!("{} 已附加选中内容 · {name}  ×", icons::PAPER_PLANE_TILT)).size(10.0)).corner_radius(9)).clicked() {
                    remove = Some(id.clone());
                }
            }
            if let Some(id) = remove {
                state.attached.remove(&id);
            }
        });
    }
    // @ mentions add a terminal to the context.
    let mention = state.text.rsplit(|c: char| c.is_whitespace()).next().and_then(|word| word.strip_prefix('@')).map(str::to_lowercase);
    if let Some(query) = mention {
        let matches: Vec<Value> = state.terminals.iter().filter(|terminal| terminal["label"].as_str().is_some_and(|label| label.to_lowercase().contains(&query)) || terminal["shortId"].as_str().is_some_and(|short| short.starts_with(&query))).take(8).cloned().collect();
        for terminal in matches {
            if ui.button(format!("@ {}", terminal_name(&terminal))).clicked() {
                if let Some(id) = terminal["ref"]["sessionId"].as_str() {
                    state.context_touched = true;
                    state.context.insert(id.to_string());
                }
                let cut = state.text.rfind('@').unwrap_or(state.text.len());
                state.text.truncate(cut);
                state.text.push_str(&format!("@{} ", terminal["shortId"].as_str().unwrap_or_default()));
            }
        }
    }
    let editor = egui::TextEdit::multiline(&mut state.text).hint_text("问问终端里发生了什么… 输入 @ 引用终端").desired_rows(3).desired_width(f32::INFINITY);
    let response = ui.add(editor);
    let submit = response.has_focus() && ui.input(|input| input.key_pressed(Key::Enter) && !input.modifiers.shift);
    ui.horizontal(|ui| {
        let mut paused = state.paused;
        if style::toggle(ui, &mut paused, "暂停共享：不再读取终端").changed() {
            state.paused = paused;
            app.agent.pause(paused, state.refs(&state.context), state.refs(&state.targets));
        }
        ui.label(RichText::new("暂停共享").size(10.5).color(palette.muted()));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if state.running {
                if style::secondary_button(ui, &format!("{}  停止", icons::STOP)).clicked() {
                    app.agent.cancel();
                }
            } else if (style::primary_button(ui, &format!("{}  发送", icons::PAPER_PLANE_TILT)).clicked() || submit) && profile.is_some() {
                let text = state.text.trim_end_matches('\n').trim().to_string();
                if !text.is_empty() {
                    state.entries.push(Entry::User(text.clone()));
                    state.running = true;
                    state.scroll_to_end = true;
                    state.text.clear();
                    let selections = std::mem::take(&mut state.attached);
                    let context = if state.paused { Vec::new() } else { state.refs(&state.context) };
                    app.agent.send(profile.map(|profile| profile.id.clone()).unwrap_or_default(), text, context, state.refs(&state.targets), selections, cwd.clone(), state.paused);
                }
            }
        });
    });
}

fn providers_dialog(app: &mut App, ctx: &egui::Context) {
    let palette = style::palette_of(ctx);
    let state = &mut app.agent_state;
    if !state.settings && state.form.is_none() {
        return;
    }
    let mut close = false;
    let response = egui::Modal::new(egui::Id::unique("providers")).frame(egui::Frame::new().fill(palette.surface()).corner_radius(12).inner_margin(20).stroke(Stroke::new(1.0, palette.border_strong))).show(ctx, |ui| {
        ui.set_width(520.0);
        ui.horizontal(|ui| {
            ui.label(icon(icons::ROBOT, 22.0).color(palette.accent()));
            ui.vertical(|ui| {
                ui.label(RichText::new("模型设置").size(16.0).strong().color(palette.text()));
                ui.label(RichText::new("API Key 保存在系统钥匙串中，不会写入设置文件").size(11.0).color(palette.muted()));
            });
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| close = icon_button(ui, icons::X, "关闭", true).clicked());
        });
        ui.add_space(10.0);
        let mut saved = None;
        let mut back = false;
        if let Some(form) = state.form.as_mut() {
            provider_form(ui, form);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if style::primary_button(ui, "保存").clicked() {
                    match app.agent.providers.save(form.profile.clone()) {
                        Ok(profile) => {
                            if !form.key.trim().is_empty() && let Err(error) = app.agent.vault.set(&profile.id, &form.key) {
                                form.error = Some(error);
                            }
                            saved = Some(profile);
                        }
                        Err(error) => form.error = Some(error),
                    }
                }
                if form.editing && style::secondary_button(ui, if form.testing { "测试中…" } else { "测试连接" }).clicked() && !form.testing {
                    form.testing = true;
                    let slot = form.result.clone();
                    app.agent.test(form.profile.id.clone(), move |result| *slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(result));
                }
                if let Some(result) = form.result.lock().unwrap_or_else(|p| p.into_inner()).take() {
                    form.testing = false;
                    form.error = Some(match result {
                        Ok((_, true)) => "连接正常，工具调用可用。".into(),
                        Ok((_, false)) => "连接正常，但模型没有调用工具；可改为“仅聊天”。".into(),
                        Err(error) => crate::agent::runtime::describe(&error).0,
                    });
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if form.editing && style::danger_button(ui, "删除").clicked() {
                        let _ = app.agent.providers.delete(&form.profile.id);
                        app.agent.vault.delete(&form.profile.id);
                        saved = Some(form.profile.clone());
                    }
                    if style::ghost_button(ui, "返回").clicked() {
                        back = true;
                    }
                });
            });
        }
        if back {
            state.form = None;
        }
        if let Some(profile) = saved {
            state.providers = app.agent.providers.list().unwrap_or_default();
            if state.profile.is_none() {
                state.profile = Some(profile.id);
            }
            state.form = None;
            state.settings = true;
        }
        if state.form.is_none() {
            for profile in state.providers.clone() {
                ui.horizontal(|ui| {
                    let verified = if profile.verified.tools { "已验证" } else if profile.verified.connection { "已连接" } else { "未测试" };
                    ui.label(RichText::new(&profile.name).strong());
                    ui.label(RichText::new(format!("{} · {} · {verified}", profile.model, profile.wire_api.label())).size(10.5).color(palette.muted()));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("编辑").clicked() {
                            state.form = Some(ProviderForm::new(Some(profile.clone())));
                        }
                    });
                });
            }
            ui.add_space(8.0);
            if style::secondary_button(ui, &format!("{}  添加模型服务", icons::PLUS)).clicked() {
                state.form = Some(ProviderForm::new(None));
            }
        }
    });
    if close || response.should_close() {
        state.settings = false;
        state.form = None;
    }
}

fn provider_form(ui: &mut Ui, form: &mut ProviderForm) {
    let palette = style::palette(ui);
    let preset = providers::preset(&form.profile.preset_id).unwrap_or(&providers::PRESETS[0]);
    ui.label(RichText::new("服务").size(11.0).color(palette.muted()));
    egui::ComboBox::from_id_salt("preset").width(ui.available_width()).selected_text(preset.label).show_ui(ui, |ui| {
        for option in providers::PRESETS {
            if ui.selectable_label(option.id == preset.id, option.label).clicked() && option.id != preset.id {
                form.profile.preset_id = option.id.into();
                form.profile.wire_api = option.wire;
                form.profile.base_url = option.base_url.into();
                form.profile.auth = option.auth;
                if !form.editing {
                    form.profile.name = option.label.into();
                }
            }
        }
    });
    let preset = providers::preset(&form.profile.preset_id).unwrap_or(&providers::PRESETS[0]);
    let field = |ui: &mut Ui, label: &str, value: &mut String, hint: &str, password: bool| {
        ui.label(RichText::new(label).size(11.0).color(palette.muted()));
        ui.add(egui::TextEdit::singleline(value).hint_text(hint).password(password).desired_width(f32::INFINITY));
    };
    field(ui, "配置名称", &mut form.profile.name, "例如 工作用 GPT", false);
    field(ui, "模型 ID", &mut form.profile.model, "例如 gpt-5 或 claude-sonnet", false);
    if preset.protocols.len() > 1 {
        ui.label(RichText::new("协议").size(11.0).color(palette.muted()));
        ui.horizontal(|ui| {
            for wire in preset.protocols {
                ui.radio_value(&mut form.profile.wire_api, *wire, wire.label());
            }
        });
    }
    if preset.auth == Auth::ApiKey {
        field(ui, "Base URL", &mut form.profile.base_url, "https://…/v1", false);
        let hint = if form.editing { "已保存；留空则保持不变" } else { "粘贴 API Key" };
        field(ui, "API Key", &mut form.key, hint, true);
    } else {
        ui.add(egui::Label::new(RichText::new("使用 Agent 引擎中已经登录的 OpenAI 账号，不需要 API Key。").size(11.0).color(palette.muted())).wrap());
    }
    ui.horizontal(|ui| {
        style::toggle(ui, &mut form.profile.chat_only, "仅聊天");
        ui.label(RichText::new("仅聊天（模型不支持工具调用时使用，不能执行终端操作）").size(11.0));
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new("上下文预算").size(11.0).color(palette.muted()));
        let mut kib = form.profile.context_budget_bytes / 1024;
        egui::ComboBox::from_id_salt("budget").selected_text(format!("{kib} KiB")).show_ui(ui, |ui| {
            for option in [16, 32, 64, 128, 256, 512] {
                ui.selectable_value(&mut kib, option, format!("{option} KiB"));
            }
        });
        form.profile.context_budget_bytes = kib * 1024;
    });
    if let Some(error) = &form.error {
        ui.add(egui::Label::new(RichText::new(error).size(11.0).color(palette.text_soft())).wrap());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn citations_link_only_known_terminal_ids() {
        let terminals = vec![json!({ "shortId": "0123abcd", "ref": { "sessionId": "0123abcd-full" } })];
        let parts = citations("see 0123abcd and deadbeef.", &terminals);
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[1], Err(("0123abcd".to_string(), "0123abcd-full".to_string())));
        assert_eq!(parts[2], Ok(" and deadbeef.".to_string()));
    }

    #[test]
    fn broker_reasons_read_as_advice() {
        assert!(operation_reason("blocked: password prompt").contains("等待密码"));
        assert_eq!(operation_reason("something else"), "something else");
    }
}

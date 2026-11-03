// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! One Agent engine process per provider profile: the key reaches that process alone, and a
//! turn can never fall back to another provider. It speaks JSON-RPC on stdio; requests it
//! makes of us are served only for omniTerm's own tools and refused otherwise.

use super::engine::Engine;
use super::providers::{Auth, Profile};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const CREDENTIAL_ENV: &str = "OMNITERM_PROVIDER_API_KEY";
pub const PROVIDER_ID: &str = "omniterm";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// Seen only if the engine offered a built-in tool: terminal mode must stop, not continue.
const FORBIDDEN_ITEMS: &[&str] = &["commandExecution", "fileChange", "mcpToolCall", "webSearch", "collabAgentToolCall", "imageGeneration"];

pub fn instructions() -> &'static str {
    include_str!("../../assets/agent-instructions.txt").trim_end()
}

pub fn tools() -> Value {
    serde_json::from_str(include_str!("../../assets/agent-tools.json")).unwrap_or_else(|_| json!([]))
}

fn toml_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

/// Everything is passed per launch, so a user's engine config file is never rewritten.
pub fn arguments(profile: &Profile, state_home: Option<&str>) -> Vec<String> {
    let mut overrides: Vec<(String, String)> = vec![
        ("features.dynamic_tools_only".into(), "true".into()),
        ("features.code_mode".into(), "false".into()),
        ("features.hooks".into(), "false".into()),
        ("analytics.enabled".into(), "false".into()),
        ("history.persistence".into(), toml_string("none")),
        ("model".into(), toml_string(&profile.model)),
    ];
    // Another installation's databases carry their own migration checksums and can refuse this
    // build. Overrides here also beat `sqlite_home` and `log_dir` in the user's config.
    if let Some(home) = state_home {
        overrides.push(("sqlite_home".into(), toml_string(home)));
        overrides.push(("log_dir".into(), toml_string(home)));
    }
    // Naming the provider keeps a login profile on OpenAI even if the user's engine config
    // defaults to some other service, which would otherwise receive the terminal context.
    if profile.auth == Auth::EngineLogin {
        overrides.push(("model_provider".into(), toml_string("openai")));
    } else {
        let wire = serde_json::to_value(profile.wire_api).ok().and_then(|value| value.as_str().map(String::from)).unwrap_or_default();
        let prefix = format!("model_providers.{PROVIDER_ID}");
        overrides.extend([
            ("model_provider".into(), toml_string(PROVIDER_ID)),
            (format!("{prefix}.name"), toml_string(&profile.name)),
            (format!("{prefix}.base_url"), toml_string(&profile.base_url)),
            (format!("{prefix}.wire_api"), toml_string(&wire)),
            (format!("{prefix}.env_key"), toml_string(CREDENTIAL_ENV)),
            (format!("{prefix}.requires_openai_auth"), "false".into()),
        ]);
    }
    overrides.into_iter().flat_map(|(key, value)| ["-c".to_string(), format!("{key}={value}")]).collect()
}

/// What the engine reports, in the terms the panel needs.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    TurnStarted(String),
    TextDelta { item: String, delta: String },
    TextDone { item: String, text: String },
    ReasoningDelta { item: String, delta: String },
    Tool { item: String, tool: String, status: String, success: Option<bool> },
    TurnCompleted { turn: String, error: Option<String> },
    Retrying,
    Error(String),
    /// The engine started a built-in tool; nothing may keep running.
    Forbidden(String),
}

pub fn normalize(method: &str, params: &Value) -> Option<Event> {
    let text = |value: &Value| value.as_str().unwrap_or_default().to_string();
    match method {
        "turn/started" => Some(Event::TurnStarted(text(&params["turn"]["id"]))),
        "item/agentMessage/delta" => Some(Event::TextDelta { item: text(&params["itemId"]), delta: text(&params["delta"]) }),
        "item/reasoning/textDelta" | "item/reasoning/summaryTextDelta" => Some(Event::ReasoningDelta { item: text(&params["itemId"]), delta: text(&params["delta"]) }),
        "item/started" | "item/completed" => {
            let item = &params["item"];
            let kind = item["type"].as_str().unwrap_or_default();
            if method == "item/started" && FORBIDDEN_ITEMS.contains(&kind) {
                return Some(Event::Forbidden(kind.to_string()));
            }
            match kind {
                "agentMessage" if method == "item/completed" => Some(Event::TextDone { item: text(&item["id"]), text: text(&item["text"]) }),
                "dynamicToolCall" => Some(Event::Tool { item: text(&item["id"]), tool: text(&item["tool"]), status: text(&item["status"]), success: item["success"].as_bool() }),
                _ => None,
            }
        }
        "turn/completed" => Some(Event::TurnCompleted { turn: text(&params["turn"]["id"]), error: params["turn"]["error"]["message"].as_str().map(String::from) }),
        "error" if params["willRetry"].as_bool() == Some(true) => Some(Event::Retrying),
        "error" => Some(Event::Error(params["error"]["message"].as_str().unwrap_or("模型请求失败。").to_string())),
        _ => None,
    }
}

/// What a user can act on. The engine's own text is kept apart, as details for a bug report.
pub fn actionable(message: &str) -> String {
    let text = message.to_lowercase();
    let any = |words: &[&str]| words.iter().any(|word| text.contains(word));
    if any(&["401", "403", "unauthorized", "invalid_api_key", "invalid api key", "authentication", "rejected the api key"]) {
        return "服务拒绝了凭据，请在模型设置中检查 API Key 和权限。".into();
    }
    if any(&["429", "rate limit", "ratelimit", "rate-limit", "overloaded", "quota", "usage limit"]) {
        return "请求受到限速或额度不足，请稍后重试或检查服务额度。".into();
    }
    if any(&["timed out", "timeout", "time out", "stopped responding"]) {
        return "模型服务响应超时，未执行后续操作。请检查网络和 Base URL。".into();
    }
    if (text.contains("context") && (text.contains("window") || text.contains("length"))) || any(&["too many tokens", "prompt is too long"]) {
        return "内容超出模型上下文，请减少共享的终端或新建对话。".into();
    }
    if any(&["404", "does not exist"]) || (text.contains("model") && text.contains("not") && text.contains("found")) {
        return "找不到该模型或端点，请核对模型 ID、协议和 Base URL。".into();
    }
    if (text.contains("tool") || text.contains("function")) && any(&["support", "invalid", "unavailable"]) {
        return "该模型可能不支持工具调用，可在模型设置中改为“仅聊天”。".into();
    }
    if any(&["agent engine could not start", "agent engine stopped", "agent engine did not answer", "agent engine is not running"]) {
        return "omniTerm Agent 引擎未能启动或已退出，本轮没有继续执行。请重试；如果反复出现，请把技术详情发给开发者。".into();
    }
    if message.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)) {
        return message.to_string();
    }
    "模型服务返回了错误，本轮没有继续执行。可以重试，或查看技术详情。".into()
}

/// A friendly message, and the original as detail when they differ.
pub fn describe(message: &str) -> (String, Option<String>) {
    let friendly = actionable(message);
    let detail = (friendly != message).then(|| message.to_string());
    (friendly, detail)
}

type ToolHandler = Arc<dyn Fn(Value) -> Result<Value, String> + Send + Sync>;
/// Replies waited for, by request ID.
type Pending = Arc<Mutex<HashMap<u64, Sender<Result<Value, String>>>>>;

pub struct Runtime {
    stdin: Mutex<Option<ChildStdin>>,
    child: Mutex<Option<Child>>,
    pending: Pending,
    next: AtomicU64,
    stopped: Arc<AtomicBool>,
    pub model: String,
}

pub struct Launch<'a> {
    pub engine: &'a Engine,
    pub profile: &'a Profile,
    pub credential: Option<String>,
    pub codex_home: Option<String>,
    pub state_home: Option<String>,
}

impl Runtime {
    /// Starts the engine and completes the handshake. `notify` receives its events, `tool`
    /// serves its tool calls (on their own thread, so a slow tool never blocks the stream),
    /// and `exited` is called once if the engine stops on its own.
    pub fn start(launch: Launch, version: &str, notify: impl Fn(Event) + Send + 'static, tool: ToolHandler, exited: impl FnOnce(String) + Send + 'static) -> Result<Arc<Self>, String> {
        let mut args = launch.engine.args.clone();
        args.extend(arguments(launch.profile, launch.state_home.as_deref()));
        args.extend(["app-server".to_string(), "--listen".to_string(), "stdio://".to_string()]);
        let mut command = crate::remote::manager::command(&launch.engine.executable);
        command.args(&args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).env_remove(CREDENTIAL_ENV);
        if let Some(credential) = &launch.credential {
            command.env(CREDENTIAL_ENV, credential);
        }
        if let Some(home) = &launch.codex_home {
            command.env("CODEX_HOME", home);
        }
        if let Some(home) = &launch.state_home {
            command.env("CODEX_SQLITE_HOME", home);
        }
        let mut child = command.spawn().map_err(|error| format!("The Agent engine could not start: {error}"))?;
        let stdin = child.stdin.take().ok_or("The Agent engine could not start: no stdin")?;
        let stdout = child.stdout.take().ok_or("The Agent engine could not start: no stdout")?;
        let runtime = Arc::new(Self { stdin: Mutex::new(Some(stdin)), child: Mutex::new(Some(child)), pending: Arc::default(), next: AtomicU64::new(1), stopped: Arc::default(), model: launch.profile.model.clone() });
        let reader = runtime.clone();
        std::thread::Builder::new()
            .name("omniterm-agent".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
                    reader.receive(message, &notify, &tool);
                }
                let error = "The Agent engine stopped.".to_string();
                for (_, sender) in reader.pending.lock().unwrap_or_else(|p| p.into_inner()).drain() {
                    let _ = sender.send(Err(error.clone()));
                }
                if !reader.stopped.load(Ordering::Acquire) {
                    exited(error);
                }
            })
            .map_err(|error| error.to_string())?;
        runtime.request("initialize", json!({ "clientInfo": { "name": "omniterm_desktop", "title": "omniTerm", "version": version }, "capabilities": { "experimentalApi": true } }))?;
        runtime.send(&json!({ "method": "initialized" }));
        Ok(runtime)
    }

    fn send(&self, message: &Value) {
        if let Some(stdin) = self.stdin.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
            let mut line = message.to_string();
            line.push('\n');
            let _ = stdin.write_all(line.as_bytes()).and_then(|_| stdin.flush());
        }
    }

    pub fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = channel();
        self.pending.lock().unwrap_or_else(|p| p.into_inner()).insert(id, sender);
        self.send(&json!({ "id": id, "method": method, "params": params }));
        match receiver.recv_timeout(REQUEST_TIMEOUT) {
            Ok(result) => result,
            Err(_) => {
                self.pending.lock().unwrap_or_else(|p| p.into_inner()).remove(&id);
                Err(format!("The Agent engine did not answer {method}."))
            }
        }
    }

    fn receive(self: &Arc<Self>, message: Value, notify: &(impl Fn(Event) + Send), tool: &ToolHandler) {
        let method = message["method"].as_str();
        match (method, message.get("id")) {
            (Some(method), Some(id)) => self.serve(id.clone(), method.to_string(), message["params"].clone(), tool.clone()),
            (Some(method), None) => {
                if let Some(event) = normalize(method, &message["params"]) {
                    notify(event);
                }
            }
            (None, Some(id)) => {
                let Some(sender) = id.as_u64().and_then(|id| self.pending.lock().unwrap_or_else(|p| p.into_inner()).remove(&id)) else { return };
                let result = match message.get("error") {
                    Some(error) => Err(error["message"].as_str().unwrap_or("The Agent engine reported an error.").to_string()),
                    None => Ok(message["result"].clone()),
                };
                let _ = sender.send(result);
            }
            (None, None) => {}
        }
    }

    /// Requests the engine makes of its client. Only omniTerm's tools are served; anything
    /// else is a built-in path terminal mode has disabled, so it is declined.
    fn serve(self: &Arc<Self>, id: Value, method: String, params: Value, tool: ToolHandler) {
        if method == "item/tool/call" {
            let runtime = self.clone();
            std::thread::spawn(move || {
                let (text, success) = match tool(params) {
                    Ok(Value::String(text)) => (text, true),
                    Ok(value) => (value.to_string(), true),
                    Err(error) => (error, false),
                };
                runtime.send(&json!({ "id": id, "result": { "contentItems": [{ "type": "inputText", "text": text }], "success": success } }));
            });
            return;
        }
        let declined = match method.as_str() {
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => Some(json!({ "decision": "decline" })),
            "item/tool/requestUserInput" => Some(json!({ "answers": {} })),
            _ => None,
        };
        match declined {
            Some(result) => self.send(&json!({ "id": id, "result": result })),
            None => self.send(&json!({ "id": id, "error": { "code": -32601, "message": format!("omniTerm does not serve {method}.") } })),
        }
    }

    pub fn start_thread(&self, cwd: &str, with_tools: bool) -> Result<String, String> {
        let result = self.request(
            "thread/start",
            json!({
                "model": self.model,
                "cwd": cwd,
                "approvalPolicy": "never",
                "sandbox": "read-only",
                "ephemeral": true,
                "serviceName": "omniterm_desktop",
                "developerInstructions": instructions(),
                "environments": [],
                "dynamicTools": if with_tools { tools() } else { json!([]) },
            }),
        )?;
        result["thread"]["id"].as_str().map(String::from).ok_or_else(|| "The Agent engine returned no thread.".into())
    }

    pub fn start_turn(&self, thread: &str, text: &str) -> Result<String, String> {
        let result = self.request("turn/start", json!({ "threadId": thread, "input": [{ "type": "text", "text": text, "text_elements": [] }] }))?;
        result["turn"]["id"].as_str().map(String::from).ok_or_else(|| "The Agent engine returned no turn.".into())
    }

    pub fn interrupt(&self, thread: &str, turn: &str) -> Result<(), String> {
        self.request("turn/interrupt", json!({ "threadId": thread, "turnId": turn })).map(|_| ())
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.stdin.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(mut child) = self.child.lock().unwrap_or_else(|p| p.into_inner()).take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::providers::{Verified, WireApi};

    fn profile(auth: Auth) -> Profile {
        Profile { id: "p".into(), preset_id: "custom".into(), name: "Mine".into(), auth, wire_api: WireApi::Chat, base_url: "https://example.com/v1".into(), model: "m1".into(), chat_only: false, context_budget_bytes: 65536, verified: Verified::default() }
    }

    #[test]
    fn launches_pin_the_provider_and_keep_state_private() {
        let args = arguments(&profile(Auth::ApiKey), Some("/data/state"));
        let joined = args.join(" ");
        assert!(joined.contains("model_provider=\"omniterm\""));
        assert!(joined.contains("model_providers.omniterm.env_key=\"OMNITERM_PROVIDER_API_KEY\""));
        assert!(joined.contains("model_providers.omniterm.wire_api=\"chat\""));
        assert!(joined.contains("sqlite_home=\"/data/state\""));
        assert!(joined.contains("features.dynamic_tools_only=true"));
        let login = arguments(&profile(Auth::EngineLogin), None).join(" ");
        assert!(login.contains("model_provider=\"openai\"") && !login.contains("base_url"));
    }

    #[test]
    fn engine_events_become_panel_events_and_built_in_tools_trip_the_wire() {
        assert_eq!(normalize("item/agentMessage/delta", &json!({ "itemId": "a", "delta": "hi" })), Some(Event::TextDelta { item: "a".into(), delta: "hi".into() }));
        assert_eq!(normalize("item/started", &json!({ "item": { "type": "commandExecution" } })), Some(Event::Forbidden("commandExecution".into())));
        assert_eq!(normalize("error", &json!({ "willRetry": true })), Some(Event::Retrying));
        assert_eq!(normalize("unknown", &json!({})), None);
    }

    #[test]
    fn failures_read_as_something_to_do() {
        assert_eq!(describe("HTTP 401 Unauthorized").0, "服务拒绝了凭据，请在模型设置中检查 API Key 和权限。");
        assert!(describe("HTTP 401 Unauthorized").1.is_some());
        assert_eq!(describe("已取消").0, "已取消");
        assert_eq!(tools().as_array().map(Vec::len), Some(9));
        assert!(instructions().contains("omniTerm Agent"));
    }
}

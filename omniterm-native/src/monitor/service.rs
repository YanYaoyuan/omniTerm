// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Persistent monitors. Sampling costs no model tokens and touches no terminal. An action is
//! fixed when the user approves the task: one exact command on one pinned terminal, run through
//! the broker with its usual checks, for at most the task's duration and `max_runs` times.
//!
//! Every lifecycle change moves a monitor's generation on, and whatever waited on the host
//! compares the generation it started with, so older work never outlives a pause, a stop or a
//! removal. A pause takes the target away in the broker, which drops the grant and voids
//! anything not yet written without interrupting a running command.

use super::probe::{self, Condition, Platform, State, Target};
use crate::remote::manager::Manager;
use omniterm_terminal_host::host::Host;
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_MONITORS: usize = 16;
const MAX_HISTORY: usize = 200;
const MAX_SERIES: usize = 120;
const MAX_FAILURES: u32 = 5;
const MAX_ACTIONS: u64 = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Pending,
    Running,
    Paused,
    Stopped,
    Expired,
    Failed,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Pending => "等待批准",
            Status::Running => "监控中",
            Status::Paused => "已暂停",
            Status::Stopped => "已停止",
            Status::Expired => "已到期",
            Status::Failed => "已失败",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    pub command: String,
    pub terminal: Value,
    pub max_runs: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct HistoryEntry {
    pub at: f64,
    pub kind: &'static str,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Monitor {
    pub id: String,
    /// "local", or the SSH session whose shared login reaches the host.
    pub host: Option<String>,
    pub label: String,
    pub target: Target,
    pub conditions: Vec<Condition>,
    pub interval_seconds: f64,
    pub duration_minutes: f64,
    pub action: Option<Action>,
    pub proposed_by: &'static str,
    pub status: Status,
    pub last_sample: Option<Map<String, Value>>,
    pub series: Vec<Map<String, Value>>,
    pub history: Vec<HistoryEntry>,
    pub triggers: u64,
    pub action_runs: u64,
    pub identity_unverified: bool,
    #[serde(skip)]
    expires: Option<Instant>,
    #[serde(skip)]
    sampled: Option<Instant>,
    #[serde(skip)]
    probe: State,
    #[serde(skip)]
    streaks: HashMap<usize, u32>,
    #[serde(skip)]
    generation: u64,
    #[serde(skip)]
    approving: bool,
    #[serde(skip)]
    failures: u32,
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Monitor {
    fn record(&mut self, kind: &'static str, message: impl Into<String>) {
        self.history.push(HistoryEntry { at: now(), kind, message: message.into() });
        if self.history.len() > MAX_HISTORY {
            self.history.remove(0);
        }
    }
}

pub struct Service {
    host: Arc<Host>,
    remotes: Arc<Manager>,
    monitors: Mutex<Vec<Monitor>>,
    operations: Mutex<HashMap<String, String>>,
    alerts: Mutex<Vec<(String, String)>>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

/// One request, validated as the desktop app's `validateRequest`.
fn request(input: &Value) -> Result<Monitor, String> {
    let host = match &input["host"] {
        Value::String(local) if local == "local" => None,
        value => Some(value["sessionId"].as_str().ok_or("请选择监控所在的主机。")?.to_string()),
    };
    let interval = input["intervalSeconds"].as_f64().unwrap_or(5.0);
    let duration = input["durationMinutes"].as_f64().unwrap_or(60.0);
    if !(1.0..=3600.0).contains(&interval) {
        return Err("采样间隔必须是 1–3600 秒。".into());
    }
    if !(duration > 0.0 && duration <= 7.0 * 24.0 * 60.0) {
        return Err("监控时长必须大于 0，且不超过 7 天。".into());
    }
    let conditions = input["conditions"].as_array().cloned().unwrap_or_default();
    if conditions.len() > 8 {
        return Err("最多可以设置 8 个条件。".into());
    }
    let conditions = conditions.iter().map(probe::condition).collect::<Result<Vec<_>, _>>()?;
    let action = match &input["action"] {
        Value::Null => None,
        action => {
            let command = action["command"].as_str().filter(|c| !c.trim().is_empty() && c.len() <= 4096 && !c.chars().any(|ch| ch.is_control() && ch != '\t')).ok_or("自动操作的命令必须是一行文本。")?;
            let terminal = &action["terminal"];
            if !terminal["sessionId"].is_string() || !terminal["epoch"].is_u64() {
                return Err("请选择自动操作的目标终端。".into());
            }
            let max_runs = action["maxRuns"].as_u64().unwrap_or(1);
            if !(1..=MAX_ACTIONS).contains(&max_runs) {
                return Err(format!("自动操作次数必须是 1–{MAX_ACTIONS}。"));
            }
            Some(Action { command: command.into(), terminal: json!({ "sessionId": terminal["sessionId"], "epoch": terminal["epoch"] }), max_runs })
        }
    };
    if action.is_some() && conditions.is_empty() {
        return Err("自动操作需要至少一个触发条件。".into());
    }
    let platform = if host.is_none() { Platform::local() } else { Platform::Linux };
    Ok(Monitor {
        id: uuid::Uuid::new_v4().to_string(),
        host,
        label: input["label"].as_str().map(|label| label.chars().take(80).collect()).unwrap_or_default(),
        target: probe::validate(&input["target"], platform)?,
        conditions,
        interval_seconds: interval,
        duration_minutes: duration,
        action,
        proposed_by: if input["proposedBy"].as_str() == Some("agent") { "agent" } else { "user" },
        status: Status::Pending,
        last_sample: None,
        series: Vec::new(),
        history: Vec::new(),
        triggers: 0,
        action_runs: 0,
        identity_unverified: false,
        expires: None,
        sampled: None,
        probe: State::default(),
        streaks: HashMap::new(),
        generation: 0,
        approving: false,
        failures: 0,
    })
}

impl Service {
    pub fn new(host: Arc<Host>, remotes: Arc<Manager>, wake: impl Fn() + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self { host, remotes, monitors: Mutex::default(), operations: Mutex::default(), alerts: Mutex::default(), wake: Arc::new(wake) })
    }

    pub fn list(&self) -> Vec<Monitor> {
        lock(&self.monitors).clone()
    }

    pub fn take_alerts(&self) -> Vec<(String, String)> {
        std::mem::take(&mut *lock(&self.alerts))
    }

    fn with<R>(&self, id: &str, change: impl FnOnce(&mut Monitor) -> R) -> Option<R> {
        let result = lock(&self.monitors).iter_mut().find(|monitor| monitor.id == id).map(change);
        (self.wake)();
        result
    }

    fn current(&self, id: &str, generation: u64) -> bool {
        lock(&self.monitors).iter().any(|monitor| monitor.id == id && monitor.generation == generation)
    }

    /// A proposal only records intent. Nothing is collected until the user approves it.
    pub fn propose(&self, input: &Value) -> Result<Monitor, String> {
        let mut monitor = request(input)?;
        let mut monitors = lock(&self.monitors);
        if monitors.len() >= MAX_MONITORS {
            return Err(format!("最多可以保留 {MAX_MONITORS} 个监控任务，请先删除不需要的任务。"));
        }
        monitor.record("state", if monitor.proposed_by == "agent" { "Agent 提议了这个任务" } else { "已创建" });
        monitors.push(monitor.clone());
        drop(monitors);
        (self.wake)();
        Ok(monitor)
    }

    fn end_task(&self, id: &str) {
        if let Err(error) = self.host.handle("task.end", json!({ "taskId": id })) {
            self.with(id, |monitor| monitor.record("error", format!("结束自动操作授权失败：{error}")));
        }
    }

    /// Requests for one task reach the host in the order they are sent, so a stop sent after
    /// `open` also lands after it; each later step only checks that it is still wanted.
    fn authorize(&self, id: &str, generation: u64, open: &str, params: Value) -> Result<(), String> {
        let Some((action, expires)) = lock(&self.monitors).iter().find(|monitor| monitor.id == id).and_then(|monitor| Some((monitor.action.clone()?, monitor.expires))) else { return Ok(()) };
        self.host.handle(open, params)?;
        let still = lock(&self.monitors).iter().any(|monitor| monitor.id == id && monitor.generation == generation && monitor.action.as_ref() == Some(&action));
        if !still {
            return Ok(());
        }
        let left = expires.map(|at| at.saturating_duration_since(Instant::now()).as_millis() as f64).unwrap_or(0.0);
        self.host.handle("task.grant", json!({ "taskId": id, "target": action.terminal, "expiresAt": now() + left })).map(|_| ())
    }

    pub fn approve(self: &Arc<Self>, id: &str) -> Result<(), String> {
        let generation = self
            .with(id, |monitor| {
                if monitor.status != Status::Pending || monitor.approving {
                    return Err("这个任务不在等待批准。".to_string());
                }
                monitor.generation += 1;
                monitor.approving = true;
                monitor.expires = Some(Instant::now() + Duration::from_secs_f64(monitor.duration_minutes * 60.0));
                Ok(monitor.generation)
            })
            .ok_or("这个监控任务不存在。")??;
        let target = lock(&self.monitors).iter().find(|monitor| monitor.id == id).and_then(|monitor| monitor.action.as_ref().map(|action| action.terminal.clone()));
        let result = self.authorize(id, generation, "task.begin", json!({ "taskId": id, "context": [], "targets": target.into_iter().collect::<Vec<_>>() }));
        self.with(id, |monitor| monitor.approving = false);
        result?;
        if !self.current(id, generation) {
            return Ok(());
        }
        self.with(id, |monitor| {
            monitor.status = Status::Running;
            monitor.record("state", "已批准并开始监控");
        });
        self.start_loop(id.to_string(), generation);
        Ok(())
    }

    pub fn deny(&self, id: &str) {
        let approving_with_action = lock(&self.monitors).iter().find(|monitor| monitor.id == id).is_some_and(|monitor| monitor.approving && monitor.action.is_some());
        lock(&self.monitors).retain(|monitor| monitor.id != id);
        if approving_with_action {
            self.end_task(id);
        }
        (self.wake)();
    }

    pub fn pause(&self, id: &str) {
        let has_action = self
            .with(id, |monitor| {
                if monitor.status != Status::Running {
                    return None;
                }
                monitor.generation += 1;
                monitor.status = Status::Paused;
                monitor.record("state", "已暂停：不再采样，也不会触发操作");
                Some(monitor.action.is_some())
            })
            .flatten();
        if has_action == Some(true)
            && let Err(error) = self.host.handle("task.update", json!({ "taskId": id, "context": [], "targets": [], "paused": true }))
        {
            self.with(id, |monitor| monitor.record("error", format!("暂停自动操作授权失败：{error}")));
        }
    }

    pub fn resume(self: &Arc<Self>, id: &str) {
        let Some(Some((generation, target))) = self.with(id, |monitor| {
            if monitor.status != Status::Paused {
                return None;
            }
            monitor.generation += 1;
            monitor.status = Status::Running;
            monitor.failures = 0;
            monitor.record("state", "已恢复");
            Some((monitor.generation, monitor.action.as_ref().map(|action| action.terminal.clone())))
        }) else {
            return;
        };
        let restored = self.authorize(id, generation, "task.update", json!({ "taskId": id, "context": [], "targets": target.into_iter().collect::<Vec<_>>(), "paused": false }));
        if let Err(error) = restored {
            if self.current(id, generation) {
                self.with(id, |monitor| {
                    monitor.generation += 1;
                    monitor.status = Status::Paused;
                    monitor.record("error", format!("无法恢复自动操作授权，任务保持暂停：{error}"));
                });
            }
            return;
        }
        if self.current(id, generation) {
            self.start_loop(id.to_string(), generation);
        }
    }

    /// Ending the broker task revokes the grant and cancels anything it had waiting.
    pub fn revoke(&self, id: &str) {
        let had = self.with(id, |monitor| monitor.action.take().is_some()).unwrap_or(false);
        if had {
            self.end_task(id);
            self.with(id, |monitor| monitor.record("state", "已撤销自动操作，继续只读监控"));
        }
    }

    fn finish(&self, id: &str, status: Status, message: &str) {
        let had = self
            .with(id, |monitor| {
                monitor.generation += 1;
                monitor.status = status;
                monitor.record("state", message.to_string());
                monitor.action.is_some()
            })
            .unwrap_or(false);
        if had {
            self.end_task(id);
        }
    }

    pub fn stop(&self, id: &str) {
        let live = lock(&self.monitors).iter().any(|monitor| monitor.id == id && matches!(monitor.status, Status::Pending | Status::Running | Status::Paused));
        if live {
            self.finish(id, Status::Stopped, "已停止");
        }
    }

    pub fn remove(&self, id: &str) -> Result<(), String> {
        let mut monitors = lock(&self.monitors);
        if monitors.iter().any(|monitor| monitor.id == id && (monitor.approving || matches!(monitor.status, Status::Running | Status::Paused))) {
            return Err("请先停止这个任务。".into());
        }
        monitors.retain(|monitor| monitor.id != id);
        drop(monitors);
        (self.wake)();
        Ok(())
    }

    fn start_loop(self: &Arc<Self>, id: String, generation: u64) {
        let service = self.clone();
        std::thread::spawn(move || {
            while service.current(&id, generation) {
                let interval = lock(&service.monitors).iter().find(|monitor| monitor.id == id).map(|monitor| monitor.interval_seconds).unwrap_or(5.0);
                service.sample(&id, generation);
                // Sleeps in short steps so a pause or stop ends the loop promptly.
                let until = Instant::now() + Duration::from_secs_f64(interval);
                while Instant::now() < until {
                    if !service.current(&id, generation) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100).min(until.saturating_duration_since(Instant::now())));
                }
            }
        });
    }

    fn run(&self, host: Option<&str>, script: &str) -> Result<String, String> {
        match host {
            None => probe::run_local(script),
            Some(session) => self.remotes.get(session).ok_or("这个 SSH 会话没有可用的共享连接。")?.exec(script, Duration::from_secs(10)),
        }
    }

    fn sample(self: &Arc<Self>, id: &str, generation: u64) {
        let Some((host, target, state, expired)) = lock(&self.monitors).iter().find(|monitor| monitor.id == id).map(|monitor| (monitor.host.clone(), monitor.target.clone(), monitor.probe.clone(), monitor.expires.is_some_and(|at| Instant::now() >= at))) else { return };
        if expired {
            self.finish(id, Status::Expired, "已到达设定时长");
            return;
        }
        let platform = if host.is_none() { Platform::local() } else { Platform::Linux };
        let output = probe::script(&target, &state, platform).and_then(|script| self.run(host.as_deref(), &script));
        if !self.current(id, generation) {
            return;
        }
        let mut failed = None;
        let fired = self.with(id, |monitor| {
            let output = match output {
                Ok(output) => output,
                Err(error) => {
                    monitor.failures += 1;
                    monitor.record("error", error);
                    if monitor.failures >= MAX_FAILURES {
                        failed = Some(monitor.label.clone());
                    }
                    return Vec::new();
                }
            };
            let mut sample = probe::parse(&monitor.target, &output);
            let elapsed = monitor.sampled.map(|at| at.elapsed().as_secs_f64()).unwrap_or(0.0);
            monitor.sampled = Some(Instant::now());
            probe::with_cpu_rate(&mut sample, monitor.last_sample.as_ref(), elapsed);
            if sample.get("identity").and_then(Value::as_str) == Some("none") && !monitor.identity_unverified {
                monitor.identity_unverified = true;
                monitor.record("error", "无法读取这个日志文件的身份，文件被替换时可能漏掉新内容。");
            }
            if matches!(monitor.target, Target::Log { .. }) {
                // A log that is absent now is new when it appears, so all of it counts.
                monitor.probe = match sample.get("nextOffset").and_then(Value::as_u64) {
                    Some(next) => State { offset: Some(next), mark: sample.get("mark").and_then(Value::as_str).map(String::from) },
                    None => State { offset: Some(0), mark: None },
                };
            }
            for key in ["mark", "identity", "next"] {
                sample.remove(key);
            }
            sample.insert("at".into(), json!(now()));
            monitor.failures = 0;
            let mut numbers = sample.clone();
            numbers.remove("text");
            monitor.series.push(numbers);
            if monitor.series.len() > MAX_SERIES {
                monitor.series.remove(0);
            }
            if let Some(Value::String(text)) = sample.get_mut("text") {
                let skip = text.chars().count().saturating_sub(2000);
                *text = text.chars().skip(skip).collect();
            }
            // Fires on the transition into a condition, not on every sample while it holds.
            let mut fired = Vec::new();
            for (index, condition) in monitor.conditions.iter().enumerate() {
                let streak = if probe::holds(condition, &sample) { monitor.streaks.get(&index).copied().unwrap_or(0) + 1 } else { 0 };
                monitor.streaks.insert(index, streak);
                if streak == condition.for_samples {
                    fired.push(format!("条件满足：{} {} {}", condition.field, condition.op, condition.value.as_str().map(String::from).unwrap_or_else(|| if condition.value.is_null() { String::new() } else { condition.value.to_string() })).trim().to_string());
                }
            }
            monitor.last_sample = Some(sample);
            for message in &fired {
                monitor.triggers += 1;
                monitor.record("trigger", message.clone());
            }
            fired
        });
        if let Some(label) = failed {
            self.finish(id, Status::Failed, &format!("连续 {MAX_FAILURES} 次采样失败，已停止"));
            lock(&self.alerts).push((label, "监控采样连续失败，任务已停止。".into()));
            return;
        }
        for message in fired.unwrap_or_default() {
            let label = lock(&self.monitors).iter().find(|monitor| monitor.id == id).map(|monitor| monitor.label.clone()).unwrap_or_default();
            lock(&self.alerts).push((label, message));
            if !self.current(id, generation) {
                return;
            }
            self.act(id, generation);
        }
    }

    /// A run proposed but stopped by a pause stays counted: `max_runs` is a ceiling, and the
    /// broker treats a repeated call ID as the same operation.
    fn act(&self, id: &str, generation: u64) {
        let Some(Some((action, run))) = self.with(id, |monitor| {
            let action = monitor.action.clone()?;
            if monitor.action_runs >= action.max_runs {
                monitor.record("action", "已达到自动操作次数上限，未执行");
                return None;
            }
            monitor.action_runs += 1;
            Some((action, monitor.action_runs))
        }) else {
            return;
        };
        let proposed = self.host.handle("operation.propose", json!({ "taskId": id, "callId": format!("monitor-{run}"), "terminal": action.terminal["sessionId"], "command": action.command }));
        let operation = match proposed {
            Ok(operation) => operation,
            Err(error) => {
                self.with(id, |monitor| monitor.record("action", format!("自动操作未执行：{error}")));
                return;
            }
        };
        let operation_id = operation["operationId"].as_str().unwrap_or_default().to_string();
        lock(&self.operations).insert(operation_id.clone(), id.to_string());
        let status = operation["status"].as_str().unwrap_or_default().to_string();
        if status != "approved" {
            self.with(id, |monitor| monitor.record("action", format!("自动操作未获授权（{status}），未执行")));
            return;
        }
        // The last check before anything is sent; a pause or stop already voided it in the broker.
        let still = lock(&self.monitors).iter().any(|monitor| monitor.id == id && monitor.generation == generation && monitor.action.as_ref() == Some(&action));
        if !still {
            self.with(id, |monitor| monitor.record("action", "任务已暂停、停止或撤销，自动操作未执行"));
            return;
        }
        let result = self.host.handle("operation.execute", json!({ "operationId": operation_id, "taskId": id }));
        self.with(id, |monitor| match result {
            Ok(_) => monitor.record("action", format!("已发出自动操作：{}", action.command)),
            Err(error) => monitor.record("action", format!("自动操作未执行：{error}")),
        });
    }

    pub fn operation_changed(&self, operation: &Value) {
        let Some(id) = operation["operationId"].as_str().and_then(|op| lock(&self.operations).get(op).cloned()) else { return };
        let status = operation["status"].as_str().unwrap_or_default();
        if matches!(status, "completed" | "blocked" | "invalidated" | "outcomeUnknown" | "cancelled" | "takenOver" | "staged" | "interrupted") {
            let code = operation["exitCode"].as_i64().map(|code| format!("，退出码 {code}")).unwrap_or_default();
            self.with(&id, |monitor| monitor.record("action", format!("自动操作结果：{status}{code}")));
        }
    }

    pub fn shutdown(&self) {
        for monitor in lock(&self.monitors).iter_mut() {
            monitor.generation += 1;
            if matches!(monitor.status, Status::Running | Status::Paused) {
                monitor.status = Status::Stopped;
            }
        }
    }
}

/// The Agent's view: proposals without actions, and status without log text.
impl crate::agent::service::MonitorTools for Service {
    fn propose(&self, mut input: Value) -> Result<Value, String> {
        if let Some(map) = input.as_object_mut() {
            map.remove("action");
        }
        let monitor = Service::propose(self, &input)?;
        Ok(json!({ "id": monitor.id, "status": monitor.status }))
    }

    fn status(&self) -> Value {
        Value::Array(
            self.list()
                .into_iter()
                .map(|monitor| {
                    let mut sample = monitor.last_sample.unwrap_or_default();
                    sample.remove("text");
                    let history: Vec<Value> = monitor.history.iter().rev().take(10).rev().map(|entry| json!({ "at": entry.at, "kind": entry.kind, "message": entry.message })).collect();
                    json!({ "id": monitor.id, "label": monitor.label, "status": monitor.status, "target": monitor.target, "host": monitor.host.unwrap_or_else(|| "local".into()), "triggers": monitor.triggers, "actionRuns": monitor.action_runs, "sample": sample, "history": history })
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_that_cannot_be_honoured_are_refused_up_front() {
        let base = json!({ "host": "local", "target": { "kind": "process", "pid": 1 }, "conditions": [{ "field": "cpu", "op": ">", "value": 50 }] });
        assert!(request(&base).is_ok());
        assert!(request(&json!({ "host": "local", "target": { "kind": "process", "pid": 1 }, "intervalSeconds": 0 })).is_err());
        let action = json!({ "command": "restart", "terminal": { "sessionId": "s", "epoch": 1 }, "maxRuns": 2 });
        assert!(request(&json!({ "host": "local", "target": { "kind": "process", "pid": 1 }, "action": action })).unwrap_err().contains("至少一个触发条件"));
        let mut multi = base.clone();
        multi["action"] = json!({ "command": "a\nb", "terminal": { "sessionId": "s", "epoch": 1 } });
        assert!(request(&multi).is_err());
    }
}

//! Request dispatch: sessions, workspace registry, context service and operation broker.

use crate::broker::ApprovalScope;
use crate::broker::Broker;
use crate::broker::Operation;
use crate::broker::SubmitMode;
use crate::broker::TargetView;
use crate::broker::TerminalRef;
use crate::broker::Terminals;
use crate::protocol::Outbox;
use crate::redact::redact;
use crate::session::Flusher;
use crate::session::Plan;
use crate::session::Session;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::thread;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

pub const MAX_SESSIONS: usize = 20;
const MAX_INPUT_BYTES: usize = 1024 * 1024;
const TICK: Duration = Duration::from_millis(100);
const DEFAULT_LINES: usize = 200;
const MAX_LINES: usize = 2000;
const DEFAULT_BUDGET_BYTES: usize = 64 * 1024;
const MAX_BUDGET_BYTES: usize = 512 * 1024;
const MAX_MATCHES: usize = 50;
const SEARCH_LINES: usize = 20_000;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

/// Layout as last reported by the renderer. Liveness always comes from the sessions here.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    #[serde(default)]
    pub window_id: String,
    pub focused_session_id: Option<String>,
    pub active_tab_id: Option<String>,
    #[serde(default)]
    pub tabs: Vec<WorkspaceTab>,
    #[serde(skip)]
    pub updated_at: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceTab {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub panes: Vec<WorkspacePane>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePane {
    pub session_id: String,
    #[serde(default)]
    pub title: String,
}

pub struct Host {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    broker: Mutex<Broker>,
    workspace: Mutex<Workspace>,
    flusher: Arc<Flusher>,
    outbox: Outbox,
}

impl Terminals for Host {
    fn view(&self, session_id: &str) -> Option<TargetView> {
        self.session(session_id).map(|session| session.view())
    }

    fn agent_write(&self, session_id: &str, bytes: &[u8]) -> Result<(), String> {
        self.session(session_id)
            .ok_or_else(|| "The terminal has closed.".to_string())?
            .write_agent(bytes)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdParams {
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WriteParams {
    id: String,
    data: String,
    #[serde(default)]
    reply: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResizeParams {
    id: String,
    cols: u16,
    rows: u16,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TaskParams {
    task_id: String,
    #[serde(default)]
    context: Vec<TerminalRef>,
    #[serde(default)]
    targets: Vec<TerminalRef>,
    #[serde(default)]
    paused: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProposeParams {
    task_id: String,
    call_id: String,
    terminal: String,
    command: String,
    timeout_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OperationParams {
    operation_id: String,
    /// Set for calls that originate from the model: a task only reaches its own operations.
    task_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApproveParams {
    operation_id: String,
    scope: ApprovalScope,
    mode: SubmitMode,
    ttl_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContextParams {
    task_id: String,
    /// Selection text the user made in a terminal, keyed by session.
    #[serde(default)]
    selections: HashMap<String, String>,
    max_lines_per_terminal: Option<usize>,
    max_bytes: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadParams {
    task_id: String,
    terminal: String,
    since_seq: Option<u64>,
    max_lines: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchParams {
    task_id: String,
    terminal: String,
    query: String,
    max_matches: Option<usize>,
}

fn parse<T: DeserializeOwned>(params: Value) -> Result<T, String> {
    serde_json::from_value(params).map_err(|error| format!("Invalid request: {error}"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalContext {
    #[serde(rename = "ref")]
    reference: TerminalRef,
    label: String,
    kind: String,
    seq_start: u64,
    seq_end: u64,
    observed_at: u64,
    alternate_screen: bool,
    truncated: bool,
    omitted_lines: usize,
    missing_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    selection: Option<String>,
    lines: Vec<String>,
}

impl Host {
    pub fn new(outbox: Outbox) -> Arc<Self> {
        let host = Arc::new(Self {
            sessions: Mutex::default(),
            broker: Mutex::default(),
            workspace: Mutex::default(),
            flusher: Arc::default(),
            outbox,
        });
        let flush_host = Arc::clone(&host);
        thread::spawn(move || {
            let outbox = flush_host.outbox.clone();
            flush_host.flusher.run(|id| flush_host.session(id), &outbox);
        });
        let tick_host = Arc::clone(&host);
        thread::spawn(move || {
            loop {
                thread::sleep(TICK);
                tick_host.tick();
            }
        });
        host
    }

    fn sessions(&self) -> MutexGuard<'_, HashMap<String, Arc<Session>>> {
        self.sessions.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    fn broker(&self) -> MutexGuard<'_, Broker> {
        self.broker.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    fn workspace(&self) -> MutexGuard<'_, Workspace> {
        self.workspace.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    fn session(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions().get(id).cloned()
    }

    fn require(&self, id: &str) -> Result<Arc<Session>, String> {
        self.session(id).ok_or_else(|| "Terminal session was not found.".to_string())
    }

    fn tick(&self) {
        let mut broker = self.broker();
        if !broker.has_active() {
            return;
        }
        let changed = broker.tick(self, now());
        drop(broker);
        self.publish(changed);
    }

    fn publish(&self, operations: Vec<Operation>) {
        for operation in operations {
            self.outbox.event("operation.update", json!(operation));
        }
    }

    fn create(&self, plan: Plan) -> Result<Value, String> {
        let mut sessions = self.sessions();
        if sessions.len() >= MAX_SESSIONS {
            return Err(format!("Close a tab before opening another (maximum {MAX_SESSIONS} sessions)."));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let session = Arc::new(Session::new(id.clone(), plan, nonce));
        let descriptor = session.descriptor();
        sessions.insert(id, session);
        Ok(descriptor)
    }

    /// Reports a user write to the broker right away, closing the window before the next tick.
    fn after_user_write(&self) {
        self.tick();
    }

    fn resolve(&self, terminal: &str) -> Result<TerminalRef, String> {
        let sessions = self.sessions();
        let mut matches = sessions.values().filter(|session| {
            session.id == terminal || (terminal.len() >= 8 && session.id.starts_with(terminal))
        });
        let session = matches.next().ok_or("Unknown terminal.")?;
        if matches.next().is_some() {
            return Err("That terminal ID is ambiguous.".to_string());
        }
        Ok(TerminalRef { session_id: session.id.clone(), epoch: session.lock().epoch })
    }

    fn list_terminals(&self, task_id: Option<&str>) -> Result<Value, String> {
        let workspace = self.workspace().clone();
        let (context, targets) = match task_id {
            Some(task_id) => {
                let broker = self.broker();
                let task = broker.task(task_id)?;
                (task.context.clone(), task.targets.clone())
            }
            None => (Vec::new(), Vec::new()),
        };
        let sessions: Vec<Arc<Session>> = self.sessions().values().cloned().collect();
        let mut terminals: Vec<Value> = Vec::with_capacity(sessions.len());
        let mut listed = HashSet::new();
        let describe = |session: &Arc<Session>, tab: Option<(usize, &WorkspaceTab)>, pane: Option<(usize, &WorkspacePane)>| {
            let mut state = session.lock();
            let interaction = session.interaction(&mut state);
            let reference = TerminalRef { session_id: session.id.clone(), epoch: state.epoch };
            let shared = context.contains(&reference);
            let remote = session.plan.kind != "shell";
            let mut unknown = vec!["currentCommand"];
            let connection = &session.plan.connection;
            if remote {
                // The spawn directory is local; a remote directory is only ever an observation.
                if state.observed_cwd.is_none() {
                    unknown.push("cwd");
                }
            }
            let label = pane
                .map(|(_, pane)| pane.title.clone())
                .filter(|title| !title.is_empty())
                .unwrap_or_else(|| session.plan.title.clone());
            json!({
                "ref": reference,
                "shortId": session.id.chars().take(8).collect::<String>(),
                "label": label,
                "kind": session.plan.kind,
                "status": state.status,
                "interaction": interaction,
                "tab": tab.map(|(index, tab)| json!({ "id": tab.id, "index": index + 1, "title": tab.title })),
                "pane": pane.map(|(index, _)| index + 1),
                "focused": workspace.focused_session_id.as_deref() == Some(session.id.as_str()),
                "connection": { "configured": connection, "source": "profile", "certainty": if interaction.state == crate::interaction::InteractionState::Ready { "initial-connection" } else { "unknown" } },
                "cwd": if remote {
                    json!({ "value": state.observed_cwd, "source": "osc7", "certainty": "unverified" })
                } else {
                    json!({ "value": state.observed_cwd.clone().unwrap_or_else(|| session.plan.cwd.clone()), "source": if state.observed_cwd.is_some() { "osc7" } else { "spawn" }, "certainty": if state.observed_cwd.is_some() { "unverified" } else { "initial" } })
                },
                // Window titles are program output, so only selected terminals expose them.
                "observedTitle": if shared { json!(state.screen.title()) } else { Value::Null },
                "lastCommand": state.commands.back(),
                "contextSelected": shared,
                "executionTarget": targets.contains(&reference),
                "unknown": unknown,
            })
        };
        for (tab_index, tab) in workspace.tabs.iter().enumerate() {
            for (pane_index, pane) in tab.panes.iter().enumerate() {
                if let Some(session) = sessions.iter().find(|session| session.id == pane.session_id) {
                    listed.insert(session.id.clone());
                    terminals.push(describe(session, Some((tab_index, tab)), Some((pane_index, pane))));
                }
            }
        }
        // A session the renderer has not reported yet is still real; its position is unknown.
        for session in sessions.iter().filter(|session| !listed.contains(&session.id)) {
            terminals.push(describe(session, None, None));
        }
        Ok(json!({
            "terminals": terminals,
            "windowId": workspace.window_id,
            "activeTabId": workspace.active_tab_id,
            "layoutObservedAt": workspace.updated_at,
        }))
    }

    /// Bounded context for the task's selected terminals only.
    fn build_context(&self, params: ContextParams) -> Result<Value, String> {
        let task = self.broker().task(&params.task_id)?.clone();
        if task.paused {
            return Err("Context sharing is paused by the user.".to_string());
        }
        let lines_each = params.max_lines_per_terminal.unwrap_or(DEFAULT_LINES).clamp(1, MAX_LINES);
        let budget = params.max_bytes.unwrap_or(DEFAULT_BUDGET_BYTES).clamp(1024, MAX_BUDGET_BYTES);
        let share = budget / task.context.len().max(1);
        let labels = self.labels();
        let mut terminals = Vec::with_capacity(task.context.len());
        let mut unavailable = Vec::new();
        for reference in &task.context {
            let session = self.session(&reference.session_id).filter(|session| session.lock().epoch == reference.epoch);
            let Some(session) = session else {
                unavailable.push(reference.clone());
                continue;
            };
            let snapshot = session.snapshot(lines_each);
            let rendered = snapshot.rendered;
            let selection = params.selections.get(&session.id).map(|text| {
                truncate_text(&text.lines().map(redact).collect::<Vec<_>>().join("\n"), share / 2)
            });
            // Selection is spent first, then the newest output fills what remains.
            let room = share.saturating_sub(selection.as_ref().map_or(0, String::len));
            let (lines, dropped) = fit_tail(rendered.lines.iter().map(|line| redact(line)).collect(), room);
            terminals.push(TerminalContext {
                reference: reference.clone(),
                label: labels.get(&session.id).cloned().unwrap_or_else(|| session.plan.title.clone()),
                kind: session.plan.kind.clone(),
                seq_start: 0,
                seq_end: snapshot.seq_end,
                observed_at: now(),
                alternate_screen: rendered.alternate_screen,
                truncated: rendered.omitted + dropped > 0 || snapshot.skipped_bytes > 0,
                omitted_lines: rendered.omitted + dropped,
                missing_bytes: snapshot.skipped_bytes,
                selection,
                lines,
            });
        }
        Ok(json!({
            "snapshotId": uuid::Uuid::new_v4().to_string(),
            "observedAt": now(),
            "terminals": terminals,
            "unavailable": unavailable,
            "budgetBytes": budget,
        }))
    }

    fn labels(&self) -> HashMap<String, String> {
        let workspace = self.workspace();
        workspace
            .tabs
            .iter()
            .flat_map(|tab| tab.panes.iter())
            .filter(|pane| !pane.title.is_empty())
            .map(|pane| (pane.session_id.clone(), pane.title.clone()))
            .collect()
    }

    fn readable(&self, task_id: &str, terminal: &str) -> Result<(Arc<Session>, TerminalRef), String> {
        let reference = self.resolve(terminal)?;
        self.broker().check_read(task_id, &reference)?;
        Ok((self.require(&reference.session_id)?, reference))
    }

    fn read_context(&self, params: ReadParams) -> Result<Value, String> {
        let (session, reference) = self.readable(&params.task_id, &params.terminal)?;
        let lines = params.max_lines.unwrap_or(DEFAULT_LINES).clamp(1, MAX_LINES);
        let context = match params.since_seq {
            Some(since) => {
                let delta = session.delta(since, None, lines);
                TerminalContext {
                    reference,
                    label: session.plan.title.clone(),
                    kind: session.plan.kind.clone(),
                    seq_start: delta.seq_start,
                    seq_end: delta.seq_end,
                    observed_at: now(),
                    alternate_screen: delta.rendered.alternate_screen,
                    truncated: delta.rendered.omitted > 0 || delta.missing_bytes > 0,
                    omitted_lines: delta.rendered.omitted,
                    missing_bytes: delta.missing_bytes,
                    selection: None,
                    lines: delta.rendered.lines.iter().map(|line| redact(line)).collect(),
                }
            }
            None => {
                let snapshot = session.snapshot(lines);
                let rendered = snapshot.rendered;
                TerminalContext {
                    reference,
                    label: session.plan.title.clone(),
                    kind: session.plan.kind.clone(),
                    seq_start: 0,
                    seq_end: snapshot.seq_end,
                    observed_at: now(),
                    alternate_screen: rendered.alternate_screen,
                    truncated: rendered.omitted > 0 || snapshot.skipped_bytes > 0,
                    omitted_lines: rendered.omitted,
                    missing_bytes: snapshot.skipped_bytes,
                    selection: None,
                    lines: rendered.lines.iter().map(|line| redact(line)).collect(),
                }
            }
        };
        Ok(json!(context))
    }

    fn search(&self, params: SearchParams) -> Result<Value, String> {
        if params.query.is_empty() {
            return Err("A search query is required.".to_string());
        }
        let (session, reference) = self.readable(&params.task_id, &params.terminal)?;
        let delta = session.delta(0, None, SEARCH_LINES);
        let limit = params.max_matches.unwrap_or(20).clamp(1, MAX_MATCHES);
        let query = params.query.to_lowercase();
        let first_line = delta.rendered.omitted;
        let found: Vec<Value> = delta
            .rendered
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.to_lowercase().contains(&query))
            .take(limit)
            .map(|(index, line)| json!({ "line": first_line + index + 1, "text": redact(line) }))
            .collect();
        Ok(json!({
            "ref": reference,
            "matches": found,
            "seqStart": delta.seq_start,
            "seqEnd": delta.seq_end,
            "missingBytes": delta.missing_bytes,
            "truncated": delta.missing_bytes > 0 || delta.rendered.omitted > 0,
        }))
    }

    fn check_owner(&self, operation_id: &str, task_id: Option<&str>) -> Result<(), String> {
        let broker = self.broker();
        let operation = broker.get(operation_id)?;
        if task_id.is_some_and(|task_id| task_id != operation.task_id) {
            return Err("That operation belongs to another task.".to_string());
        }
        Ok(())
    }

    fn operation_result(&self, operation_id: &str) -> Result<Value, String> {
        let operation = self.broker().get(operation_id)?.clone();
        let output = match (operation.seq_start, self.session(&operation.target.session_id)) {
            (Some(start), Some(session)) if session.lock().epoch == operation.target.epoch => {
                let delta = session.delta(start, operation.seq_end, DEFAULT_LINES);
                json!({
                    "lines": delta.rendered.lines.iter().map(|line| redact(line)).collect::<Vec<_>>(),
                    "omittedLines": delta.rendered.omitted,
                    "missingBytes": delta.missing_bytes,
                    "seqStart": delta.seq_start,
                    "seqEnd": delta.seq_end,
                })
            }
            _ => Value::Null,
        };
        Ok(json!({ "operation": operation, "output": output }))
    }

    pub fn shutdown(&self) {
        let sessions: Vec<Arc<Session>> = self.sessions().drain().map(|(_, session)| session).collect();
        for session in sessions {
            session.close();
        }
    }

    pub fn handle(&self, method: &str, params: Value) -> Result<Value, String> {
        match method {
            "session.create" => self.create(parse(params)?),
            "session.start" => {
                let IdParams { id } = parse(params)?;
                let session = self.require(&id)?;
                session.start(&self.flusher, &self.outbox)?;
                Ok(session.descriptor())
            }
            "session.write" => {
                let WriteParams { id, data, reply } = parse(params)?;
                if data.len() > MAX_INPUT_BYTES {
                    return Err("Terminal input must be a string of at most 1 MiB.".to_string());
                }
                let written = self.require(&id)?.write_user(&data, reply)?;
                if !reply {
                    self.after_user_write();
                }
                Ok(json!(written))
            }
            "session.resize" => {
                let ResizeParams { id, cols, rows } = parse(params)?;
                if !(2..=1000).contains(&cols) || !(1..=500).contains(&rows) {
                    return Err("Terminal size must be 2–1000 columns and 1–500 rows.".to_string());
                }
                self.require(&id)?.resize(cols, rows)?;
                Ok(json!(true))
            }
            "session.buffer" => {
                let IdParams { id } = parse(params)?;
                Ok(self.require(&id)?.replay())
            }
            "session.close" => {
                let IdParams { id } = parse(params)?;
                let session = self.sessions().remove(&id);
                let closed = session.is_some();
                if let Some(session) = session {
                    session.close();
                    self.tick();
                }
                Ok(json!(closed))
            }
            "workspace.update" => {
                let mut workspace: Workspace = parse(params)?;
                workspace.updated_at = now();
                *self.workspace() = workspace;
                Ok(json!(true))
            }
            "workspace.list" => {
                let task_id = params.get("taskId").and_then(Value::as_str).map(str::to_string);
                self.list_terminals(task_id.as_deref())
            }
            "task.begin" => {
                let TaskParams { task_id, context, targets, .. } = parse(params)?;
                self.broker().begin_task(&task_id, context, targets);
                Ok(json!(true))
            }
            "task.update" => {
                let TaskParams { task_id, context, targets, paused } = parse(params)?;
                self.broker().update_task(&task_id, context, targets, paused)?;
                Ok(json!(true))
            }
            "task.cancel" => {
                let TaskParams { task_id, .. } = parse(params)?;
                let mut broker = self.broker();
                let changed = broker.cancel_task(self, &task_id);
                drop(broker);
                self.publish(changed);
                Ok(json!(true))
            }
            "task.end" => {
                let TaskParams { task_id, .. } = parse(params)?;
                let mut broker = self.broker();
                let changed = broker.end_task(self, &task_id);
                drop(broker);
                self.publish(changed);
                Ok(json!(true))
            }
            "task.invalidateAll" => {
                let reason = params.get("reason").and_then(Value::as_str).unwrap_or("The Agent engine restarted.").to_string();
                let changed = self.broker().invalidate_all(&reason);
                self.publish(changed);
                Ok(json!(true))
            }
            "context.build" => self.build_context(parse(params)?),
            "context.read" => self.read_context(parse(params)?),
            "context.search" => self.search(parse(params)?),
            "operation.propose" => {
                let ProposeParams { task_id, call_id, terminal, command, timeout_ms } = parse(params)?;
                let target = self.resolve(&terminal)?;
                let mut broker = self.broker();
                let operation = broker.propose(self, &task_id, &call_id, &target, &command, timeout_ms, now())?;
                drop(broker);
                self.outbox.event("operation.update", json!(operation));
                Ok(json!(operation))
            }
            "operation.approve" => {
                let ApproveParams { operation_id, scope, mode, ttl_ms } = parse(params)?;
                let mut broker = self.broker();
                let operation = broker.approve(self, &operation_id, scope, mode, ttl_ms, now());
                drop(broker);
                let operation = operation?;
                self.outbox.event("operation.update", json!(operation));
                self.tick();
                Ok(json!(operation))
            }
            "operation.deny" => {
                let OperationParams { operation_id, .. } = parse(params)?;
                let operation = self.broker().deny(&operation_id)?;
                self.outbox.event("operation.update", json!(operation));
                Ok(json!(operation))
            }
            "operation.execute" => {
                let OperationParams { operation_id, task_id } = parse(params)?;
                self.check_owner(&operation_id, task_id.as_deref())?;
                let operation = self.broker().execute(&operation_id, now())?;
                self.tick();
                Ok(json!(operation))
            }
            "operation.cancel" => {
                let OperationParams { operation_id, task_id } = parse(params)?;
                self.check_owner(&operation_id, task_id.as_deref())?;
                let mut broker = self.broker();
                let operation = broker.cancel(self, &operation_id);
                drop(broker);
                let operation = operation?;
                self.outbox.event("operation.update", json!(operation));
                Ok(json!(operation))
            }
            "operation.result" => {
                let OperationParams { operation_id, task_id } = parse(params)?;
                self.check_owner(&operation_id, task_id.as_deref())?;
                self.operation_result(&operation_id)
            }
            _ => Err(format!("Unknown method: {method}")),
        }
    }

    #[cfg(test)]
    fn insert_for_test(&self, session: Arc<Session>) {
        self.sessions().insert(session.id.clone(), session);
    }
}

fn truncate_text(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[selection truncated]", &text[..end])
}

/// Keeps the newest lines that fit; returns how many were dropped.
fn fit_tail(lines: Vec<String>, max_bytes: usize) -> (Vec<String>, usize) {
    let mut used = 0;
    let mut start = lines.len();
    while start > 0 {
        let cost = lines[start - 1].len() + 1;
        if used + cost > max_bytes {
            break;
        }
        used += cost;
        start -= 1;
    }
    let dropped = start;
    (lines.into_iter().skip(start).collect(), dropped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Frame;
    use crate::session::Status;
    use crate::session::ingest;
    use std::sync::mpsc::Receiver;

    fn fixture() -> (Arc<Host>, Receiver<Frame>) {
        let (outbox, frames) = Outbox::channel();
        (Host::new(outbox), frames)
    }

    fn terminal(host: &Host, id: &str, kind: &str, text: &[u8]) -> TerminalRef {
        let plan = Plan { kind: kind.to_string(), title: id.to_uppercase(), cwd: "/work".to_string(), ..Plan::default() };
        let session = Arc::new(Session::new(id.to_string(), plan, "nonce".to_string()));
        {
            let mut state = session.lock();
            state.epoch = 1;
            state.status = Status::Running;
            ingest(&mut state, text);
        }
        host.insert_for_test(session);
        TerminalRef { session_id: id.to_string(), epoch: 1 }
    }

    #[test]
    fn context_contains_selected_terminals_and_never_others() {
        let (host, _frames) = fixture();
        let a = terminal(&host, "a", "shell", b"build error\r\n");
        let b = terminal(&host, "b", "ssh", b"log line\r\n");
        let _c = terminal(&host, "c", "ssh", b"private output\r\n");
        host.handle("task.begin", json!({ "taskId": "t", "context": [a, b], "targets": [] })).expect("begin");
        let context = host.handle("context.build", json!({ "taskId": "t" })).expect("context").to_string();
        assert!(context.contains("build error") && context.contains("log line"));
        assert!(!context.contains("private output"));
    }

    #[test]
    fn tools_cannot_read_outside_the_task_scope() {
        let (host, _frames) = fixture();
        let a = terminal(&host, "a", "shell", b"allowed\r\n");
        terminal(&host, "c", "ssh", b"private output\r\n");
        host.handle("task.begin", json!({ "taskId": "t", "context": [a], "targets": [] })).expect("begin");
        assert!(host.handle("context.read", json!({ "taskId": "t", "terminal": "c" })).is_err());
        assert!(host.handle("context.search", json!({ "taskId": "t", "terminal": "c", "query": "private" })).is_err());
        let read = host.handle("context.read", json!({ "taskId": "t", "terminal": "a" })).expect("read");
        assert_eq!(read["lines"][0], "allowed");
    }

    #[test]
    fn listing_shares_inventory_without_exposing_output() {
        let (host, _frames) = fixture();
        terminal(&host, "c", "ssh", b"\x1b]2;secret title\x07private output\r\n");
        let list = host.handle("workspace.list", json!({})).expect("list").to_string();
        assert!(list.contains("\"kind\":\"ssh\"") && list.contains("\"cwd\""));
        assert!(!list.contains("secret title") && !list.contains("private output"));
    }

    #[test]
    fn a_remote_terminal_never_reports_its_local_spawn_directory_as_its_location() {
        let (host, _frames) = fixture();
        terminal(&host, "b", "ssh", b"");
        let list = host.handle("workspace.list", json!({})).expect("list");
        assert_eq!(list["terminals"][0]["cwd"]["value"], Value::Null);
        assert!(list["terminals"][0]["unknown"].to_string().contains("cwd"));
    }

    #[test]
    fn budget_truncation_is_labeled_and_keeps_the_newest_output() {
        let (host, _frames) = fixture();
        let text: String = (0..500).map(|index| format!("line number {index}\r\n")).collect();
        let a = terminal(&host, "a", "shell", text.as_bytes());
        host.handle("task.begin", json!({ "taskId": "t", "context": [a], "targets": [] })).expect("begin");
        let context = host.handle("context.build", json!({ "taskId": "t", "maxBytes": 1024 })).expect("context");
        let entry = &context["terminals"][0];
        assert_eq!(entry["truncated"], true);
        assert!(entry["lines"].as_array().expect("lines").last().expect("line").as_str().expect("text").contains("499"));
        assert!(entry["omittedLines"].as_u64().expect("count") > 0);
    }

    #[test]
    fn secrets_are_redacted_from_shared_context() {
        let (host, _frames) = fixture();
        let a = terminal(&host, "a", "shell", b"export API_KEY=abc123\r\n");
        host.handle("task.begin", json!({ "taskId": "t", "context": [a], "targets": [] })).expect("begin");
        let context = host.handle("context.build", json!({ "taskId": "t" })).expect("context").to_string();
        assert!(!context.contains("abc123"));
    }

    #[test]
    fn terminal_ids_must_identify_one_session() {
        let (host, _frames) = fixture();
        terminal(&host, "abcdefgh-1", "shell", b"");
        terminal(&host, "abcdefgh-2", "shell", b"");
        assert!(host.resolve("abcdefgh").unwrap_err().contains("ambiguous"));
        assert!(host.resolve("abc").is_err());
        assert!(host.resolve("abcdefgh-1").is_ok());
    }

    #[test]
    fn the_session_limit_is_enforced_here() {
        let (host, _frames) = fixture();
        let plan = json!({ "executable": "/bin/sh", "cwd": "/", "kind": "shell", "title": "T" });
        for _ in 0..MAX_SESSIONS {
            host.handle("session.create", plan.clone()).expect("created");
        }
        assert!(host.handle("session.create", plan).unwrap_err().contains("maximum 20"));
    }

    #[test]
    fn invalid_sizes_and_unknown_methods_are_rejected() {
        let (host, _frames) = fixture();
        terminal(&host, "a", "shell", b"");
        assert!(host.handle("session.resize", json!({ "id": "a", "cols": 1, "rows": 1 })).is_err());
        assert!(host.handle("session.resize", json!({ "id": "a", "cols": 80, "rows": 501 })).is_err());
        assert!(host.handle("nope", json!({})).is_err());
        assert_eq!(host.handle("session.close", json!({ "id": "missing" })), Ok(json!(false)));
    }

    #[test]
    fn a_task_cannot_reach_another_tasks_operations() {
        let (host, _frames) = fixture();
        let b = terminal(&host, "b", "ssh", b"");
        host.handle("task.begin", json!({ "taskId": "one", "targets": [b.clone()] })).expect("begin");
        host.handle("task.begin", json!({ "taskId": "two", "targets": [b] })).expect("begin");
        let operation = host
            .handle("operation.propose", json!({ "taskId": "one", "callId": "c", "terminal": "b", "command": "ls" }))
            .expect("proposed");
        let id = operation["operationId"].as_str().expect("id");
        assert!(host.handle("operation.execute", json!({ "operationId": id, "taskId": "two" })).is_err());
        assert!(host.handle("operation.cancel", json!({ "operationId": id, "taskId": "two" })).is_err());
        assert!(host.handle("operation.execute", json!({ "operationId": id, "taskId": "one" })).is_ok());
    }

    #[test]
    fn exited_sessions_stay_listed_until_closed() {
        let (host, _frames) = fixture();
        terminal(&host, "a", "shell", b"bye\r\n");
        host.session("a").expect("session").lock().status = Status::Exited;
        let list = host.handle("workspace.list", json!({})).expect("list");
        assert_eq!(list["terminals"][0]["interaction"]["state"], "exited");
    }
}

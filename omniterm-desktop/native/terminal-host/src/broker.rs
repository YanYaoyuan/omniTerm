//! Agent operations on terminals.
//!
//! Every Agent write passes through here, away from the renderer's broadcast path. The model
//! can propose and request execution; only an approval from the desktop UI can authorize one.
//! Operations are pinned to `session + connection epoch`, independent of focus or layout, and
//! live only in this process, so a restart can never resume an approval. A command that was
//! written is never written again: when its outcome is unknown it stays unknown.

use crate::interaction::Interaction;
use crate::interaction::InteractionState;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use std::collections::HashMap;
use std::collections::VecDeque;

const QUEUE_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_COMMAND_TIMEOUT_MS: u64 = 120_000;
const MAX_COMMAND_TIMEOUT_MS: u64 = 900_000;
const MAX_COMMAND_BYTES: usize = 16 * 1024;
const MAX_FINISHED_OPERATIONS: usize = 200;
const INTERRUPT: &[u8] = b"\x03";

/// Stable identity of a terminal connection. Titles, tabs and aliases are display only.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalRef {
    pub session_id: String,
    pub epoch: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OperationStatus {
    PendingApproval,
    Approved,
    Queued,
    Running,
    Staged,
    Completed,
    Denied,
    Cancelled,
    Invalidated,
    Blocked,
    Interrupted,
    TakenOver,
    /// The command may have run. It is reported as such and never retried.
    OutcomeUnknown,
}

impl OperationStatus {
    fn is_final(self) -> bool {
        !matches!(
            self,
            Self::PendingApproval | Self::Approved | Self::Queued | Self::Running
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ApprovalScope {
    Once,
    /// Later commands of this task on this exact target run without another card until expiry.
    TaskTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SubmitMode {
    Run,
    /// Types the command without Enter, for a person to review and submit.
    Stage,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Authorization {
    pub scope: ApprovalScope,
    pub mode: SubmitMode,
    pub revision: u64,
    pub expires_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    pub operation_id: String,
    pub task_id: String,
    pub call_id: String,
    pub target: TerminalRef,
    pub command: String,
    pub payload_hash: String,
    pub proposed_revision: u64,
    pub status: OperationStatus,
    pub reason: Option<String>,
    pub authorization: Option<Authorization>,
    pub exit_code: Option<i32>,
    pub seq_start: Option<u64>,
    pub seq_end: Option<u64>,
    pub created_at: u64,
    pub timeout_ms: u64,
    #[serde(skip)]
    queued_at: u64,
    #[serde(skip)]
    started_at: u64,
    #[serde(skip)]
    commands_at_start: u64,
    #[serde(skip)]
    execute_requested: bool,
}

/// What the broker may read about a terminal when deciding.
#[derive(Debug, Clone)]
pub struct TargetView {
    pub epoch: u64,
    pub revision: u64,
    pub interaction: Interaction,
    pub output_seq: u64,
    pub commands_finished: u64,
    pub last_exit_code: Option<i32>,
    pub bracketed_paste: bool,
}

pub trait Terminals {
    fn view(&self, session_id: &str) -> Option<TargetView>;
    fn agent_write(&self, session_id: &str, bytes: &[u8]) -> Result<(), String>;
}

#[derive(Debug, Clone, Default)]
struct Grant {
    target: Option<TerminalRef>,
    mode: Option<SubmitMode>,
    expires_at: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Task {
    pub context: Vec<TerminalRef>,
    pub targets: Vec<TerminalRef>,
    pub paused: bool,
    grants: Vec<Grant>,
}

#[derive(Default)]
pub struct Broker {
    tasks: HashMap<String, Task>,
    operations: HashMap<String, Operation>,
    order: VecDeque<String>,
    // One FIFO per terminal: Agent writes to a terminal are strictly serialized.
    queues: HashMap<String, VecDeque<String>>,
    calls: HashMap<(String, String), String>,
}

fn hash(target: &TerminalRef, command: &str) -> String {
    let digest = Sha256::new()
        .chain_update(target.session_id.as_bytes())
        .chain_update(target.epoch.to_le_bytes())
        .chain_update(command.as_bytes())
        .finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl Broker {
    pub fn begin_task(&mut self, task_id: &str, context: Vec<TerminalRef>, targets: Vec<TerminalRef>) {
        self.tasks.insert(
            task_id.to_string(),
            Task {
                context,
                targets,
                ..Task::default()
            },
        );
    }

    /// Replaces the scope between turns. Changing a target drops its unattended grants.
    pub fn update_task(
        &mut self,
        task_id: &str,
        context: Vec<TerminalRef>,
        targets: Vec<TerminalRef>,
        paused: bool,
    ) -> Result<(), String> {
        let task = self.tasks.get_mut(task_id).ok_or("The Agent task has ended.")?;
        task.grants.retain(|grant| {
            grant
                .target
                .as_ref()
                .is_some_and(|target| targets.contains(target))
        });
        task.context = context;
        task.targets = targets;
        task.paused = paused;
        Ok(())
    }

    pub fn task(&self, task_id: &str) -> Result<&Task, String> {
        self.tasks
            .get(task_id)
            .ok_or_else(|| "The Agent task has ended.".to_string())
    }

    /// Reading is granted by the task's context scope, and never by its target list.
    pub fn check_read(&self, task_id: &str, target: &TerminalRef) -> Result<(), String> {
        let task = self.task(task_id)?;
        if task.paused {
            return Err("Context sharing is paused by the user.".to_string());
        }
        if !task.context.contains(target) {
            return Err("That terminal is not in the context selected for this task.".to_string());
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn propose(
        &mut self,
        terminals: &dyn Terminals,
        task_id: &str,
        call_id: &str,
        target: &TerminalRef,
        command: &str,
        timeout_ms: Option<u64>,
        now: u64,
    ) -> Result<Operation, String> {
        let call_key = (task_id.to_string(), call_id.to_string());
        // A repeated model or network delivery must not create a second executable operation.
        if let Some(operation) = self.calls.get(&call_key).and_then(|id| self.operations.get(id)) {
            return Ok(operation.clone());
        }
        let task = self.task(task_id)?;
        if !task.targets.contains(target) {
            return Err("That terminal is not an execution target of this task. Reading a terminal does not allow writing to it.".to_string());
        }
        if command.trim().is_empty() || command.len() > MAX_COMMAND_BYTES {
            return Err(format!("A command must be 1 to {MAX_COMMAND_BYTES} bytes."));
        }
        if command.chars().any(|ch| ch.is_control() && !matches!(ch, '\n' | '\t')) {
            return Err("A command cannot contain control characters.".to_string());
        }
        let view = current_view(terminals, target)?;
        if view.interaction.state == InteractionState::Unsupported {
            return Err("This terminal type does not accept shell commands.".to_string());
        }
        let operation = Operation {
            operation_id: uuid::Uuid::new_v4().to_string(),
            task_id: task_id.to_string(),
            call_id: call_id.to_string(),
            target: target.clone(),
            command: command.to_string(),
            payload_hash: hash(target, command),
            proposed_revision: view.revision,
            status: OperationStatus::PendingApproval,
            reason: None,
            authorization: None,
            exit_code: None,
            seq_start: None,
            seq_end: None,
            created_at: now,
            timeout_ms: timeout_ms
                .unwrap_or(DEFAULT_COMMAND_TIMEOUT_MS)
                .clamp(1000, MAX_COMMAND_TIMEOUT_MS),
            queued_at: 0,
            started_at: 0,
            commands_at_start: 0,
            execute_requested: false,
        };
        let id = operation.operation_id.clone();
        self.calls.insert(call_key, id.clone());
        self.order.push_back(id.clone());
        self.operations.insert(id.clone(), operation);
        self.prune();
        // A live grant from the user covers later commands on the same exact target.
        let grant = self.tasks.get(task_id).and_then(|task| {
            task.grants
                .iter()
                .find(|grant| grant.target.as_ref() == Some(target) && grant.expires_at > now)
                .cloned()
        });
        if let Some(grant) = grant {
            self.authorize(&id, ApprovalScope::TaskTarget, grant.mode.unwrap_or(SubmitMode::Run), view.revision, Some(grant.expires_at));
        }
        Ok(self.operations[&id].clone())
    }

    fn authorize(&mut self, id: &str, scope: ApprovalScope, mode: SubmitMode, revision: u64, expires_at: Option<u64>) {
        if let Some(operation) = self.operations.get_mut(id) {
            operation.authorization = Some(Authorization { scope, mode, revision, expires_at });
            operation.status = OperationStatus::Approved;
        }
    }

    /// Called only for a decision made in the desktop UI.
    pub fn approve(
        &mut self,
        terminals: &dyn Terminals,
        operation_id: &str,
        scope: ApprovalScope,
        mode: SubmitMode,
        ttl_ms: Option<u64>,
        now: u64,
    ) -> Result<Operation, String> {
        let operation = self.get(operation_id)?;
        if operation.status != OperationStatus::PendingApproval {
            return Err("This operation is no longer waiting for approval.".to_string());
        }
        let target = operation.target.clone();
        let task_id = operation.task_id.clone();
        let view = match current_view(terminals, &target) {
            Ok(view) => view,
            Err(reason) => {
                self.finish(operation_id, OperationStatus::Invalidated, Some(reason.clone()), None);
                return Err(reason);
            }
        };
        let expires_at = (scope == ApprovalScope::TaskTarget)
            .then(|| now + ttl_ms.unwrap_or(600_000).min(3_600_000));
        self.authorize(operation_id, scope, mode, view.revision, expires_at);
        if let (Some(expires_at), Some(task)) = (expires_at, self.tasks.get_mut(&task_id)) {
            task.grants.push(Grant { target: Some(target), mode: Some(mode), expires_at });
        }
        self.get(operation_id).cloned()
    }

    pub fn deny(&mut self, operation_id: &str) -> Result<Operation, String> {
        let status = self.get(operation_id)?.status;
        if status != OperationStatus::PendingApproval {
            return Err("This operation is no longer waiting for approval.".to_string());
        }
        self.finish(operation_id, OperationStatus::Denied, Some("Denied by the user.".to_string()), None);
        self.get(operation_id).cloned()
    }

    /// Queues an approved operation. Calling this twice cannot write the command twice.
    pub fn execute(&mut self, operation_id: &str, now: u64) -> Result<Operation, String> {
        let operation = self
            .operations
            .get_mut(operation_id)
            .ok_or("Unknown operation.")?;
        match operation.status {
            OperationStatus::Approved => {
                operation.status = OperationStatus::Queued;
                operation.queued_at = now;
                let session = operation.target.session_id.clone();
                self.queues.entry(session).or_default().push_back(operation_id.to_string());
            }
            OperationStatus::PendingApproval => operation.execute_requested = true,
            _ => {}
        }
        self.get(operation_id).cloned()
    }

    /// Stops an operation that has not been written, or interrupts its original target.
    pub fn cancel(&mut self, terminals: &dyn Terminals, operation_id: &str) -> Result<Operation, String> {
        let operation = self.get(operation_id)?;
        match operation.status {
            OperationStatus::PendingApproval | OperationStatus::Approved | OperationStatus::Queued => {
                self.finish(operation_id, OperationStatus::Cancelled, Some("Cancelled.".to_string()), None);
            }
            OperationStatus::Running => {
                let target = operation.target.clone();
                current_view(terminals, &target)?;
                terminals.agent_write(&target.session_id, INTERRUPT)?;
                let seq = terminals.view(&target.session_id).map(|view| view.output_seq);
                self.finish(
                    operation_id,
                    OperationStatus::Interrupted,
                    Some("Ctrl+C was sent to the original target; the command may have had partial effects.".to_string()),
                    seq,
                );
            }
            _ => {}
        }
        self.get(operation_id).cloned()
    }

    /// Cancels everything the task has pending and revokes its grants.
    pub fn cancel_task(&mut self, terminals: &dyn Terminals, task_id: &str) -> Vec<Operation> {
        if let Some(task) = self.tasks.get_mut(task_id) {
            task.grants.clear();
        }
        let ids: Vec<String> = self
            .order
            .iter()
            .filter(|id| {
                self.operations.get(*id).is_some_and(|operation| {
                    operation.task_id == task_id && !operation.status.is_final()
                })
            })
            .cloned()
            .collect();
        ids.iter()
            .filter_map(|id| self.cancel(terminals, id).ok())
            .collect()
    }

    pub fn end_task(&mut self, terminals: &dyn Terminals, task_id: &str) -> Vec<Operation> {
        let changed = self.cancel_task(terminals, task_id);
        self.tasks.remove(task_id);
        changed
    }

    /// An engine restart or crash voids every approval and pending operation.
    pub fn invalidate_all(&mut self, reason: &str) -> Vec<Operation> {
        self.tasks.values_mut().for_each(|task| task.grants.clear());
        let ids: Vec<String> = self.order.iter().cloned().collect();
        let mut changed = Vec::new();
        for id in ids {
            let Some(status) = self.operations.get(&id).map(|operation| operation.status) else {
                continue;
            };
            match status {
                OperationStatus::PendingApproval | OperationStatus::Approved | OperationStatus::Queued => {
                    self.finish(&id, OperationStatus::Invalidated, Some(reason.to_string()), None);
                }
                OperationStatus::Running => {
                    self.finish(&id, OperationStatus::OutcomeUnknown, Some(reason.to_string()), None);
                }
                _ => continue,
            }
            changed.push(self.operations[&id].clone());
        }
        changed
    }

    pub fn get(&self, operation_id: &str) -> Result<&Operation, String> {
        self.operations
            .get(operation_id)
            .ok_or_else(|| "Unknown operation.".to_string())
    }

    pub fn has_active(&self) -> bool {
        self.operations.values().any(|operation| !operation.status.is_final())
    }

    /// Advances queues and running commands. Returns every operation that changed.
    pub fn tick(&mut self, terminals: &dyn Terminals, now: u64) -> Vec<Operation> {
        let mut changed = Vec::new();
        self.sweep_stale(terminals, now, &mut changed);
        let sessions: Vec<String> = self.queues.keys().cloned().collect();
        for session in sessions {
            self.advance_queue(terminals, &session, now, &mut changed);
        }
        changed
    }

    fn sweep_stale(&mut self, terminals: &dyn Terminals, now: u64, changed: &mut Vec<Operation>) {
        let ids: Vec<String> = self.order.iter().cloned().collect();
        for id in ids {
            let Some(operation) = self.operations.get(&id) else {
                continue;
            };
            let before = operation.status;
            if before == OperationStatus::Running {
                self.track_running(terminals, &id, now);
            } else if !before.is_final() {
                let view = current_view(terminals, &operation.target);
                let promoted = before == OperationStatus::Approved && operation.execute_requested;
                match view {
                    Err(reason) => self.finish(&id, OperationStatus::Invalidated, Some(reason), None),
                    Ok(view) => {
                        let revision_moved = operation
                            .authorization
                            .as_ref()
                            .is_some_and(|authorization| authorization.revision != view.revision);
                        if revision_moved {
                            self.finish(
                                &id,
                                OperationStatus::Invalidated,
                                Some("The terminal changed after approval (user input or a new interaction mode).".to_string()),
                                None,
                            );
                        } else if promoted {
                            let _ = self.execute(&id, now);
                        }
                    }
                }
            }
            if let Some(operation) = self.operations.get(&id)
                && operation.status != before
            {
                changed.push(operation.clone());
            }
        }
    }

    fn track_running(&mut self, terminals: &dyn Terminals, id: &str, now: u64) {
        let Some(operation) = self.operations.get(id).cloned() else {
            return;
        };
        let Ok(view) = current_view(terminals, &operation.target) else {
            self.finish(
                id,
                OperationStatus::OutcomeUnknown,
                Some("The terminal closed or reconnected while the command was running. Its outcome is unknown and it will not be retried.".to_string()),
                None,
            );
            return;
        };
        let started_revision = operation.authorization.as_ref().map(|authorization| authorization.revision);
        if Some(view.revision) != started_revision {
            self.finish(
                id,
                OperationStatus::TakenOver,
                Some("The user took over this terminal; the Agent stopped tracking the command.".to_string()),
                Some(view.output_seq),
            );
            return;
        }
        if view.commands_finished > operation.commands_at_start {
            if let Some(entry) = self.operations.get_mut(id) {
                entry.exit_code = view.last_exit_code;
            }
            self.finish(id, OperationStatus::Completed, None, Some(view.output_seq));
            return;
        }
        if now.saturating_sub(operation.started_at) >= operation.timeout_ms {
            self.finish(
                id,
                OperationStatus::OutcomeUnknown,
                Some("No completion signal arrived before the timeout. The command may still be running and will not be retried.".to_string()),
                Some(view.output_seq),
            );
        }
    }

    fn advance_queue(&mut self, terminals: &dyn Terminals, session: &str, now: u64, changed: &mut Vec<Operation>) {
        loop {
            let Some(head) = self.queues.get(session).and_then(|queue| queue.front().cloned()) else {
                self.queues.remove(session);
                return;
            };
            let Some(operation) = self.operations.get(&head) else {
                self.pop(session);
                continue;
            };
            match operation.status {
                OperationStatus::Queued => {}
                // The running command stays at the head and holds back the rest.
                OperationStatus::Running => return,
                _ => {
                    self.pop(session);
                    continue;
                }
            }
            if !self.try_start(terminals, &head, now, changed) {
                return;
            }
        }
    }

    /// Returns false when the head must keep waiting for its terminal.
    fn try_start(&mut self, terminals: &dyn Terminals, id: &str, now: u64, changed: &mut Vec<Operation>) -> bool {
        let Some(operation) = self.operations.get(id).cloned() else {
            return true;
        };
        let reject = |broker: &mut Self, status, reason: String, changed: &mut Vec<Operation>| {
            broker.finish(id, status, Some(reason), None);
            changed.push(broker.operations[id].clone());
            true
        };
        let Some(authorization) = operation.authorization.clone() else {
            return reject(self, OperationStatus::Invalidated, "No valid authorization.".to_string(), changed);
        };
        if !self.tasks.contains_key(&operation.task_id) {
            return reject(self, OperationStatus::Invalidated, "The Agent task has ended.".to_string(), changed);
        }
        if authorization.expires_at.is_some_and(|expires_at| expires_at <= now) {
            return reject(self, OperationStatus::Invalidated, "The approval expired.".to_string(), changed);
        }
        if hash(&operation.target, &operation.command) != operation.payload_hash {
            return reject(self, OperationStatus::Invalidated, "The operation no longer matches what was approved.".to_string(), changed);
        }
        let view = match current_view(terminals, &operation.target) {
            Ok(view) => view,
            Err(reason) => return reject(self, OperationStatus::Invalidated, reason, changed),
        };
        if view.revision != authorization.revision {
            return reject(self, OperationStatus::Invalidated, "The terminal changed after approval.".to_string(), changed);
        }
        let multi_line = operation.command.contains('\n');
        if multi_line && !view.bracketed_paste {
            return reject(self, OperationStatus::Blocked, "A multi-line command needs bracketed paste, which this terminal has not enabled.".to_string(), changed);
        }
        let state = view.interaction.state;
        let submit = match (authorization.mode, state) {
            (SubmitMode::Run, InteractionState::Ready) => true,
            (SubmitMode::Stage, InteractionState::Ready | InteractionState::Unknown) => false,
            (SubmitMode::Run, InteractionState::Unknown) => {
                return reject(self, OperationStatus::Blocked, "This terminal's state cannot be confirmed, so the command was not submitted. Approve it as text to review, then press Enter yourself.".to_string(), changed);
            }
            (_, InteractionState::Busy) => {
                if now.saturating_sub(operation.queued_at) >= QUEUE_TIMEOUT_MS {
                    return reject(self, OperationStatus::Blocked, "The terminal stayed busy, so the command was not sent. If it is sitting at a shell prompt, press Enter there once and try again.".to_string(), changed);
                }
                return false;
            }
            (_, InteractionState::Fullscreen) => {
                return reject(self, OperationStatus::Blocked, "A full-screen program is active, so the command was not sent.".to_string(), changed);
            }
            (_, InteractionState::Password) => {
                return reject(self, OperationStatus::Blocked, "A password prompt is active, so the command was not sent.".to_string(), changed);
            }
            (_, InteractionState::Exited | InteractionState::Unsupported) => {
                return reject(self, OperationStatus::Invalidated, "The terminal cannot accept commands.".to_string(), changed);
            }
        };
        let mut bytes = Vec::with_capacity(operation.command.len() + 12);
        if multi_line {
            bytes.extend_from_slice(b"\x1b[200~");
            bytes.extend_from_slice(operation.command.as_bytes());
            bytes.extend_from_slice(b"\x1b[201~");
        } else {
            bytes.extend_from_slice(operation.command.as_bytes());
        }
        if submit {
            bytes.push(b'\r');
        }
        // Status moves first: from here the command counts as sent, even if the write fails.
        if let Some(entry) = self.operations.get_mut(id) {
            entry.status = if submit { OperationStatus::Running } else { OperationStatus::Staged };
            entry.started_at = now;
            entry.seq_start = Some(view.output_seq);
            entry.commands_at_start = view.commands_finished;
        }
        if let Err(error) = terminals.agent_write(&operation.target.session_id, &bytes) {
            self.finish(id, OperationStatus::OutcomeUnknown, Some(format!("The write failed and will not be retried: {error}")), None);
            changed.push(self.operations[id].clone());
            return true;
        }
        if !submit {
            self.finish(id, OperationStatus::Staged, Some("Typed into the terminal without Enter; review and submit it there.".to_string()), Some(view.output_seq));
        }
        changed.push(self.operations[id].clone());
        !submit
    }

    fn pop(&mut self, session: &str) {
        if let Some(queue) = self.queues.get_mut(session) {
            queue.pop_front();
        }
    }

    fn finish(&mut self, id: &str, status: OperationStatus, reason: Option<String>, seq_end: Option<u64>) {
        if let Some(operation) = self.operations.get_mut(id) {
            operation.status = status;
            operation.reason = reason;
            operation.seq_end = seq_end.or(operation.seq_end);
        }
    }

    fn prune(&mut self) {
        let finished = self
            .order
            .iter()
            .filter(|id| self.operations.get(*id).is_some_and(|operation| operation.status.is_final()))
            .count();
        let mut excess = finished.saturating_sub(MAX_FINISHED_OPERATIONS);
        let mut kept = VecDeque::with_capacity(self.order.len());
        for id in std::mem::take(&mut self.order) {
            let finished = self.operations.get(&id).is_some_and(|operation| operation.status.is_final());
            if excess > 0 && finished {
                excess -= 1;
                if let Some(operation) = self.operations.remove(&id) {
                    self.calls.remove(&(operation.task_id, operation.call_id));
                }
            } else {
                kept.push_back(id);
            }
        }
        self.order = kept;
    }
}

fn current_view(terminals: &dyn Terminals, target: &TerminalRef) -> Result<TargetView, String> {
    let view = terminals
        .view(&target.session_id)
        .ok_or("The target terminal has closed.")?;
    if view.epoch != target.epoch {
        return Err("The target terminal reconnected; this request belonged to the old connection.".to_string());
    }
    if view.interaction.state == InteractionState::Exited {
        return Err("The target terminal has exited.".to_string());
    }
    Ok(view)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interaction::StateSource;
    use std::cell::RefCell;

    struct Fake {
        views: RefCell<HashMap<String, TargetView>>,
        writes: RefCell<Vec<(String, String)>>,
    }

    fn interaction(state: InteractionState, source: StateSource) -> Interaction {
        Interaction { state, source }
    }

    fn ready() -> Interaction {
        interaction(InteractionState::Ready, StateSource::ShellIntegration)
    }

    impl Fake {
        fn new(sessions: &[&str]) -> Self {
            let view = TargetView {
                epoch: 1,
                revision: 0,
                interaction: ready(),
                output_seq: 10,
                commands_finished: 0,
                last_exit_code: None,
                bracketed_paste: false,
            };
            Self {
                views: RefCell::new(sessions.iter().map(|id| ((*id).to_string(), view.clone())).collect()),
                writes: RefCell::default(),
            }
        }

        fn update(&self, session: &str, update: impl FnOnce(&mut TargetView)) {
            update(self.views.borrow_mut().get_mut(session).expect("session exists"));
        }

        fn written(&self) -> Vec<(String, String)> {
            self.writes.borrow().clone()
        }
    }

    impl Terminals for Fake {
        fn view(&self, session_id: &str) -> Option<TargetView> {
            self.views.borrow().get(session_id).cloned()
        }

        fn agent_write(&self, session_id: &str, bytes: &[u8]) -> Result<(), String> {
            self.writes.borrow_mut().push((session_id.to_string(), String::from_utf8_lossy(bytes).into_owned()));
            Ok(())
        }
    }

    fn reference(session: &str) -> TerminalRef {
        TerminalRef { session_id: session.to_string(), epoch: 1 }
    }

    /// Task reads A+B and may only write to B.
    fn fixture() -> (Fake, Broker) {
        let fake = Fake::new(&["a", "b", "c"]);
        let mut broker = Broker::default();
        broker.begin_task("task", vec![reference("a"), reference("b")], vec![reference("b")]);
        (fake, broker)
    }

    fn propose(broker: &mut Broker, fake: &Fake, call: &str, command: &str) -> String {
        broker
            .propose(fake, "task", call, &reference("b"), command, None, 1000)
            .expect("proposal accepted")
            .operation_id
    }

    fn approve(broker: &mut Broker, fake: &Fake, id: &str) {
        broker.approve(fake, id, ApprovalScope::Once, SubmitMode::Run, None, 1000).expect("approved");
    }

    fn status(broker: &Broker, id: &str) -> OperationStatus {
        broker.get(id).expect("operation exists").status
    }

    #[test]
    fn reading_a_terminal_does_not_allow_writing_to_it() {
        let (fake, mut broker) = fixture();
        assert!(broker.check_read("task", &reference("a")).is_ok());
        let error = broker
            .propose(&fake, "task", "1", &reference("a"), "ls", None, 1000)
            .expect_err("A is read-only");
        assert!(error.contains("does not allow writing"));
        assert!(broker.check_read("task", &reference("c")).is_err());
        assert!(broker.propose(&fake, "task", "2", &reference("c"), "ls", None, 1000).is_err());
    }

    #[test]
    fn nothing_runs_without_a_user_approval() {
        let (fake, mut broker) = fixture();
        let id = propose(&mut broker, &fake, "1", "uptime");
        broker.execute(&id, 1000).expect("request recorded");
        broker.tick(&fake, 1100);
        assert_eq!(status(&broker, &id), OperationStatus::PendingApproval);
        assert!(fake.written().is_empty());
    }

    #[test]
    fn an_approved_command_reaches_only_its_target_once() {
        let (fake, mut broker) = fixture();
        let id = propose(&mut broker, &fake, "1", "uptime");
        approve(&mut broker, &fake, &id);
        broker.execute(&id, 1000).expect("queued");
        broker.execute(&id, 1000).expect("second request is harmless");
        broker.tick(&fake, 1100);
        broker.execute(&id, 1200).expect("third request is harmless");
        broker.tick(&fake, 1300);
        assert_eq!(fake.written(), [("b".to_string(), "uptime\r".to_string())]);
        assert_eq!(status(&broker, &id), OperationStatus::Running);
    }

    #[test]
    fn approval_given_while_execution_waits_starts_the_command() {
        let (fake, mut broker) = fixture();
        let id = propose(&mut broker, &fake, "1", "uptime");
        broker.execute(&id, 1000).expect("request recorded");
        approve(&mut broker, &fake, &id);
        broker.tick(&fake, 1100);
        broker.tick(&fake, 1200);
        assert_eq!(fake.written().len(), 1);
    }

    #[test]
    fn a_repeated_tool_call_returns_the_same_operation() {
        let (fake, mut broker) = fixture();
        let first = propose(&mut broker, &fake, "1", "uptime");
        assert_eq!(first, propose(&mut broker, &fake, "1", "uptime"));
    }

    #[test]
    fn a_replacement_terminal_does_not_inherit_requests() {
        let (fake, mut broker) = fixture();
        let id = propose(&mut broker, &fake, "1", "uptime");
        approve(&mut broker, &fake, &id);
        broker.execute(&id, 1000).expect("queued");
        let replacement = fake.view("a").expect("a exists");
        fake.views.borrow_mut().remove("b");
        fake.views.borrow_mut().insert("new-b".to_string(), replacement);
        broker.tick(&fake, 1100);
        assert_eq!(status(&broker, &id), OperationStatus::Invalidated);
        assert!(fake.written().is_empty());
    }

    #[test]
    fn a_reconnected_epoch_rejects_stale_requests_and_approvals() {
        let (fake, mut broker) = fixture();
        let queued = propose(&mut broker, &fake, "1", "uptime");
        approve(&mut broker, &fake, &queued);
        broker.execute(&queued, 1000).expect("queued");
        let waiting = propose(&mut broker, &fake, "2", "date");
        fake.update("b", |view| view.epoch = 2);
        let error = broker
            .approve(&fake, &waiting, ApprovalScope::Once, SubmitMode::Run, None, 1100)
            .expect_err("approval of a stale request");
        assert!(error.contains("reconnected"));
        broker.tick(&fake, 1200);
        assert_eq!(status(&broker, &queued), OperationStatus::Invalidated);
        assert!(fake.written().is_empty());
    }

    #[test]
    fn user_input_after_approval_invalidates_the_operation() {
        let (fake, mut broker) = fixture();
        let id = propose(&mut broker, &fake, "1", "uptime");
        approve(&mut broker, &fake, &id);
        fake.update("b", |view| view.revision += 1);
        broker.execute(&id, 1000).expect("queued");
        broker.tick(&fake, 1100);
        assert_eq!(status(&broker, &id), OperationStatus::Invalidated);
        assert!(fake.written().is_empty());
    }

    #[test]
    fn taking_over_a_running_command_stops_tracking_it_without_a_replay() {
        let (fake, mut broker) = fixture();
        let id = propose(&mut broker, &fake, "1", "sleep 100");
        approve(&mut broker, &fake, &id);
        broker.execute(&id, 1000).expect("queued");
        broker.tick(&fake, 1100);
        fake.update("b", |view| view.revision += 1);
        broker.tick(&fake, 1200);
        assert_eq!(status(&broker, &id), OperationStatus::TakenOver);
        assert_eq!(fake.written().len(), 1);
    }

    #[test]
    fn an_unknown_outcome_is_reported_and_never_retried() {
        let (fake, mut broker) = fixture();
        let id = propose(&mut broker, &fake, "1", "deploy");
        approve(&mut broker, &fake, &id);
        broker.execute(&id, 1000).expect("queued");
        broker.tick(&fake, 1100);
        fake.views.borrow_mut().remove("b");
        broker.tick(&fake, 1200);
        assert_eq!(status(&broker, &id), OperationStatus::OutcomeUnknown);
        broker.execute(&id, 1300).expect("no effect");
        broker.tick(&fake, 1400);
        assert_eq!(fake.written().len(), 1);

        let (fake, mut broker) = fixture();
        let id = broker
            .propose(&fake, "task", "1", &reference("b"), "deploy", Some(2000), 1000)
            .expect("proposed")
            .operation_id;
        approve(&mut broker, &fake, &id);
        broker.execute(&id, 1000).expect("queued");
        broker.tick(&fake, 1100);
        broker.tick(&fake, 3200);
        assert_eq!(status(&broker, &id), OperationStatus::OutcomeUnknown);
    }

    #[test]
    fn non_idle_terminals_never_receive_a_submitted_command() {
        for state in [InteractionState::Fullscreen, InteractionState::Password, InteractionState::Unknown] {
            let (fake, mut broker) = fixture();
            fake.update("b", |view| view.interaction = interaction(state, StateSource::None));
            let id = propose(&mut broker, &fake, "1", "uptime");
            approve(&mut broker, &fake, &id);
            broker.execute(&id, 1000).expect("queued");
            broker.tick(&fake, 1100);
            assert_eq!(status(&broker, &id), OperationStatus::Blocked);
            assert!(fake.written().is_empty());
        }
    }

    #[test]
    fn a_busy_terminal_keeps_the_command_queued_and_then_gives_up() {
        let (fake, mut broker) = fixture();
        fake.update("b", |view| view.interaction = interaction(InteractionState::Busy, StateSource::ShellIntegration));
        let id = propose(&mut broker, &fake, "1", "uptime");
        approve(&mut broker, &fake, &id);
        broker.execute(&id, 1000).expect("queued");
        broker.tick(&fake, 2000);
        assert_eq!(status(&broker, &id), OperationStatus::Queued);
        broker.tick(&fake, 1000 + QUEUE_TIMEOUT_MS);
        assert_eq!(status(&broker, &id), OperationStatus::Blocked);
        assert!(fake.written().is_empty());
    }

    #[test]
    fn unknown_state_can_only_be_staged_without_enter() {
        let (fake, mut broker) = fixture();
        fake.update("b", |view| view.interaction = interaction(InteractionState::Unknown, StateSource::None));
        let id = propose(&mut broker, &fake, "1", "uptime");
        broker.approve(&fake, &id, ApprovalScope::Once, SubmitMode::Stage, None, 1000).expect("approved");
        broker.execute(&id, 1000).expect("queued");
        broker.tick(&fake, 1100);
        assert_eq!(status(&broker, &id), OperationStatus::Staged);
        assert_eq!(fake.written(), [("b".to_string(), "uptime".to_string())]);
    }

    #[test]
    fn writes_to_one_terminal_are_serialized_and_each_is_revalidated() {
        let (fake, mut broker) = fixture();
        let first = propose(&mut broker, &fake, "1", "one");
        let second = propose(&mut broker, &fake, "2", "two");
        for id in [&first, &second] {
            approve(&mut broker, &fake, id);
            broker.execute(id, 1000).expect("queued");
        }
        broker.tick(&fake, 1100);
        assert_eq!(fake.written().len(), 1);
        fake.update("b", |view| {
            view.commands_finished = 1;
            view.last_exit_code = Some(3);
        });
        broker.tick(&fake, 1200);
        assert_eq!(broker.get(&first).expect("first").exit_code, Some(3));
        assert_eq!(status(&broker, &first), OperationStatus::Completed);
        assert_eq!(fake.written()[1], ("b".to_string(), "two\r".to_string()));
    }

    #[test]
    fn a_task_grant_covers_only_the_same_target_until_it_expires() {
        let (fake, mut broker) = fixture();
        let id = propose(&mut broker, &fake, "1", "one");
        broker.approve(&fake, &id, ApprovalScope::TaskTarget, SubmitMode::Run, Some(5000), 1000).expect("approved");
        let covered = broker.propose(&fake, "task", "2", &reference("b"), "two", None, 2000).expect("proposed");
        assert_eq!(covered.status, OperationStatus::Approved);
        let expired = broker.propose(&fake, "task", "3", &reference("b"), "three", None, 7000).expect("proposed");
        assert_eq!(expired.status, OperationStatus::PendingApproval);
    }

    #[test]
    fn cancelling_stops_queued_work_and_interrupts_the_original_target() {
        let (fake, mut broker) = fixture();
        let running = propose(&mut broker, &fake, "1", "sleep 100");
        let queued = propose(&mut broker, &fake, "2", "next");
        for id in [&running, &queued] {
            approve(&mut broker, &fake, id);
            broker.execute(id, 1000).expect("queued");
        }
        broker.tick(&fake, 1100);
        broker.cancel_task(&fake, "task");
        assert_eq!(status(&broker, &running), OperationStatus::Interrupted);
        assert_eq!(status(&broker, &queued), OperationStatus::Cancelled);
        assert_eq!(fake.written()[1], ("b".to_string(), "\u{3}".to_string()));
        broker.tick(&fake, 1200);
        assert_eq!(fake.written().len(), 2);
    }

    #[test]
    fn an_engine_restart_voids_approvals_and_marks_running_commands_unknown() {
        let (fake, mut broker) = fixture();
        let running = propose(&mut broker, &fake, "1", "sleep 100");
        approve(&mut broker, &fake, &running);
        broker.execute(&running, 1000).expect("queued");
        broker.tick(&fake, 1100);
        let waiting = propose(&mut broker, &fake, "2", "next");
        approve(&mut broker, &fake, &waiting);
        broker.invalidate_all("The Agent engine restarted.");
        assert_eq!(status(&broker, &running), OperationStatus::OutcomeUnknown);
        assert_eq!(status(&broker, &waiting), OperationStatus::Invalidated);
    }

    #[test]
    fn paused_or_withdrawn_context_stops_new_reads() {
        let (_, mut broker) = fixture();
        broker.update_task("task", vec![reference("a")], vec![reference("b")], false).expect("updated");
        assert!(broker.check_read("task", &reference("b")).is_err());
        broker.update_task("task", vec![reference("a")], vec![reference("b")], true).expect("updated");
        assert!(broker.check_read("task", &reference("a")).unwrap_err().contains("paused"));
    }

    #[test]
    fn commands_cannot_smuggle_control_characters() {
        let (fake, mut broker) = fixture();
        for command in ["ls\rrm -rf /", "ls\x1b[A", "", "   "] {
            assert!(broker.propose(&fake, "task", command, &reference("b"), command, None, 1000).is_err());
        }
    }

    #[test]
    fn multi_line_commands_require_bracketed_paste() {
        let (fake, mut broker) = fixture();
        let id = propose(&mut broker, &fake, "1", "a\nb");
        approve(&mut broker, &fake, &id);
        broker.execute(&id, 1000).expect("queued");
        broker.tick(&fake, 1100);
        assert_eq!(status(&broker, &id), OperationStatus::Blocked);
        fake.update("b", |view| view.bracketed_paste = true);
        let id = propose(&mut broker, &fake, "2", "c\nd");
        approve(&mut broker, &fake, &id);
        broker.execute(&id, 1200).expect("queued");
        broker.tick(&fake, 1300);
        assert_eq!(fake.written(), [("b".to_string(), "\u{1b}[200~c\nd\u{1b}[201~\r".to_string())]);
    }
}

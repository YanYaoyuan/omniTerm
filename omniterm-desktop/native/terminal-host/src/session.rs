//! One PTY session: process lifecycle, replay buffer, screen model and output delivery.

use crate::broker::TargetView;
use crate::interaction::Facts;
use crate::interaction::Foreground;
use crate::interaction::Interaction;
use crate::interaction::ShellPhase;
use crate::interaction::classify;
use crate::protocol::Frame;
use crate::protocol::Outbox;
use crate::screen::Rendered;
use crate::screen::ScreenModel;
use crate::screen::ShellEvent;
use crate::screen::render_bytes;
use portable_pty::ChildKiller;
use portable_pty::CommandBuilder;
use portable_pty::MasterPty;
use portable_pty::PtySize;
use portable_pty::native_pty_system;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::io::Read;
use std::io::Write;
use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use std::time::Instant;

pub const MAX_BUFFER: usize = 2 * 1024 * 1024;
/// Output is coalesced for this long, bounding IPC traffic per frame without visible lag.
const FLUSH_DELAY: Duration = Duration::from_millis(4);
const FLUSH_BYTES: usize = 256 * 1024;
/// A reader pauses above this, which stops the PTY and throttles the producing program.
const MAX_PENDING: usize = 1024 * 1024;
const READ_CHUNK: usize = 64 * 1024;
const EXIT_DRAIN: Duration = Duration::from_millis(250);
/// Output is parsed when someone asks about the screen, not as it arrives: parsing costs more
/// than the whole delivery path. This much of a burst's tail is parsed in full.
const MAX_UNPARSED: usize = 1024 * 1024;
/// Above this, older output is skimmed for mode changes so eviction can never lose one.
const SKIM_AT: usize = MAX_UNPARSED + 512 * 1024;
const MAX_CSI_LENGTH: usize = 32;
const MAX_OSC_LENGTH: usize = 4096;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: String,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub connection: Connection,
}

/// Facts from the saved profile. They describe how the session started, not where its shell
/// is now; nested `ssh`, `su` or tmux can move it without this process knowing.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub host: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pending,
    Running,
    Exited,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandRecord {
    pub exit_code: Option<i32>,
    pub seq_start: u64,
    pub seq_end: u64,
}

pub struct State {
    pub status: Status,
    pub closed: bool,
    pub epoch: u64,
    /// Moves on user input and interaction-mode changes, never on ordinary output.
    pub revision: u64,
    pub cols: u16,
    pub rows: u16,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub screen: ScreenModel,
    pub phase: ShellPhase,
    pub observed_cwd: Option<String>,
    pub commands_finished: u64,
    pub commands: VecDeque<CommandRecord>,
    pub last_exit_code: Option<i32>,
    command_started_at: Option<u64>,
    pending_input: bool,
    ring: VecDeque<u8>,
    seq: u64,
    parsed_seq: u64,
    /// Output that reached the screen model only as mode and mark sequences.
    skipped_bytes: u64,
    pending: Vec<u8>,
    alternate_screen: bool,
    reader_done: bool,
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Option<mpsc::Sender<Vec<u8>>>,
    killer: Option<Box<dyn ChildKiller + Send + Sync>>,
}

pub struct Session {
    pub id: String,
    pub plan: Plan,
    pub nonce: String,
    pub state: Mutex<State>,
    drained: Condvar,
}

impl Session {
    pub fn new(id: String, plan: Plan, nonce: String) -> Self {
        let (rows, cols) = (32, 120);
        let screen = ScreenModel::new(rows, cols, &nonce);
        Self {
            id,
            plan,
            nonce,
            state: Mutex::new(State {
                status: Status::Pending,
                closed: false,
                epoch: 0,
                revision: 0,
                cols,
                rows,
                pid: None,
                exit_code: None,
                screen,
                phase: ShellPhase::Unobserved,
                observed_cwd: None,
                commands_finished: 0,
                commands: VecDeque::new(),
                last_exit_code: None,
                command_started_at: None,
                pending_input: false,
                ring: VecDeque::new(),
                seq: 0,
                parsed_seq: 0,
                skipped_bytes: 0,
                pending: Vec::new(),
                alternate_screen: false,
                reader_done: false,
                master: None,
                writer: None,
                killer: None,
            }),
            drained: Condvar::new(),
        }
    }

    pub fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    pub fn executable(&self) -> bool {
        matches!(self.plan.kind.as_str(), "shell" | "ssh")
    }

    pub fn descriptor(&self) -> serde_json::Value {
        let state = self.lock();
        let mut value = json!({
            "id": self.id,
            "title": self.plan.title,
            "kind": self.plan.kind,
            "cwd": self.plan.cwd,
            "status": state.status,
            "epoch": state.epoch,
            "shellNonce": self.nonce,
        });
        if let Some(pid) = state.pid {
            value["pid"] = json!(pid);
        }
        value
    }

    pub fn start(self: &Arc<Self>, flusher: &Arc<Flusher>, outbox: &Outbox) -> Result<(), String> {
        let mut state = self.lock();
        if state.status != Status::Pending {
            return Ok(());
        }
        let result = self.spawn(&mut state, flusher, outbox);
        if let Err(message) = &result {
            state.status = Status::Exited;
            state.exit_code = Some(1);
            let text = format!("\r\nUnable to start session: {message}\r\n");
            ingest(&mut state, text.as_bytes());
            let bytes = std::mem::take(&mut state.pending);
            outbox.send(Frame::Data { session_id: self.id.clone(), seq_end: state.seq, bytes });
            outbox.event("session.exit", json!({ "id": self.id, "exitCode": 1, "signal": null }));
        }
        result
    }

    fn spawn(self: &Arc<Self>, state: &mut State, flusher: &Arc<Flusher>, outbox: &Outbox) -> Result<(), String> {
        let size = PtySize { rows: state.rows, cols: state.cols, pixel_width: 0, pixel_height: 0 };
        let pair = native_pty_system().openpty(size).map_err(|error| error.to_string())?;
        let mut command = CommandBuilder::new(&self.plan.executable);
        command.args(&self.plan.args);
        command.cwd(&self.plan.cwd);
        command.env_clear();
        for (key, value) in &self.plan.env {
            command.env(key, value);
        }
        let mut child = pair.slave.spawn_command(command).map_err(|error| error.to_string())?;
        drop(pair.slave);
        let reader = pair.master.try_clone_reader().map_err(|error| error.to_string())?;
        let mut writer = pair.master.take_writer().map_err(|error| error.to_string())?;
        state.pid = child.process_id();
        state.killer = Some(child.clone_killer());
        state.master = Some(pair.master);
        state.status = Status::Running;
        state.epoch += 1;

        // Input has its own thread: a child that stops reading must not stall the host.
        let (input, queue) = mpsc::channel::<Vec<u8>>();
        state.writer = Some(input);
        thread::spawn(move || {
            while let Ok(bytes) = queue.recv() {
                if writer.write_all(&bytes).is_err() || writer.flush().is_err() {
                    return;
                }
            }
        });

        let session = Arc::clone(self);
        let reader_flusher = Arc::clone(flusher);
        thread::spawn(move || session.read_output(reader, &reader_flusher));

        let session = Arc::clone(self);
        let outbox = outbox.clone();
        thread::spawn(move || {
            let status = child.wait();
            session.finish(status.ok(), &outbox);
        });
        Ok(())
    }

    fn read_output(&self, mut reader: Box<dyn Read + Send>, flusher: &Flusher) {
        let mut buffer = vec![0; READ_CHUNK];
        loop {
            let count = match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => count,
            };
            let mut state = self.lock();
            ingest(&mut state, &buffer[..count]);
            let urgent = state.pending.len() >= FLUSH_BYTES;
            drop(state);
            flusher.mark(self.id.clone(), urgent);
            let mut state = self.lock();
            while state.pending.len() > MAX_PENDING && !state.closed {
                state = self.drained.wait(state).unwrap_or_else(|poison| poison.into_inner());
            }
            if state.closed {
                return;
            }
        }
        self.lock().reader_done = true;
        self.drained.notify_all();
    }

    fn finish(&self, status: Option<portable_pty::ExitStatus>, outbox: &Outbox) {
        let deadline = Instant::now() + EXIT_DRAIN;
        let mut state = self.lock();
        // ConPTY only ends the reader once the pseudoconsole closes, so this wait is bounded.
        while !state.reader_done && Instant::now() < deadline {
            let (guard, _) = self
                .drained
                .wait_timeout(state, Duration::from_millis(20))
                .unwrap_or_else(|poison| poison.into_inner());
            state = guard;
        }
        self.flush_locked(&mut state, outbox);
        state.status = Status::Exited;
        state.writer = None;
        state.killer = None;
        state.master = None;
        let exit_code = status.as_ref().map(|status| status.exit_code() as i32);
        state.exit_code = exit_code.or(Some(0));
        let closed = state.closed;
        let exit_code = state.exit_code;
        drop(state);
        self.drained.notify_all();
        if !closed {
            outbox.event("session.exit", json!({ "id": self.id, "exitCode": exit_code, "signal": null }));
        }
    }

    /// Sent under the lock so output can never overtake or trail the exit event.
    fn flush_locked(&self, state: &mut State, outbox: &Outbox) {
        if state.pending.is_empty() || state.closed {
            return;
        }
        let bytes = take_complete_utf8(&mut state.pending, state.reader_done);
        if !bytes.is_empty() {
            outbox.send(Frame::Data { session_id: self.id.clone(), seq_end: state.seq, bytes });
        }
    }

    fn flush(&self, outbox: &Outbox) {
        let mut state = self.lock();
        self.flush_locked(&mut state, outbox);
        drop(state);
        self.drained.notify_all();
    }

    pub fn write_user(&self, data: &str, reply: bool) -> Result<bool, String> {
        let mut state = self.lock();
        if state.status != Status::Running {
            return Ok(false);
        }
        // Terminal-generated replies (cursor reports, focus events) are not user intent.
        if !reply {
            state.revision += 1;
            let submitted = data.contains(['\r', '\n', '\u{3}']);
            state.pending_input = !submitted && !data.is_empty() || (state.pending_input && !submitted);
        }
        send_input(&state, data.as_bytes())?;
        Ok(true)
    }

    pub fn write_agent(&self, bytes: &[u8]) -> Result<(), String> {
        let state = self.lock();
        if state.status != Status::Running {
            return Err("The terminal is not running.".to_string());
        }
        send_input(&state, bytes)
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        let mut state = self.lock();
        // Output written for the old size must be laid out at that size.
        catch_up(&mut state);
        state.cols = cols;
        state.rows = rows;
        state.screen.resize(rows, cols);
        if let Some(master) = &state.master {
            let size = PtySize { rows, cols, pixel_width: 0, pixel_height: 0 };
            master.resize(size).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// Replay data for a view that attached late. Starts on a character boundary.
    pub fn replay(&self) -> serde_json::Value {
        let mut state = self.lock();
        let bytes = state.ring.make_contiguous();
        let start = bytes.iter().position(|byte| byte & 0xc0 != 0x80).unwrap_or(0);
        json!({
            "data": String::from_utf8_lossy(&bytes[start..]),
            "exited": state.status == Status::Exited,
            "exitCode": state.exit_code,
        })
    }

    pub fn close(&self) {
        let mut state = self.lock();
        if state.closed {
            return;
        }
        state.closed = true;
        state.writer = None;
        let running = state.status == Status::Running;
        let pid = state.pid;
        let killer = state.killer.take();
        state.pending.clear();
        drop(state);
        self.drained.notify_all();
        if running {
            terminate(pid, killer);
        }
    }

    pub fn view(&self) -> TargetView {
        let mut state = self.lock();
        let interaction = self.interaction(&mut state);
        TargetView {
            epoch: state.epoch,
            revision: state.revision,
            interaction,
            output_seq: state.seq,
            commands_finished: state.commands_finished,
            last_exit_code: state.last_exit_code,
            bracketed_paste: state.screen.bracketed_paste(),
        }
    }

    pub fn interaction(&self, state: &mut State) -> Interaction {
        catch_up(state);
        let cursor_line = state.screen.cursor_line();
        let (foreground, password_mode) = self.terminal_mode(state);
        classify(Facts {
            running: state.status == Status::Running,
            executable: self.executable(),
            alternate_screen: state.screen.alternate_screen(),
            foreground,
            password_mode,
            phase: state.phase,
            pending_input: state.pending_input,
            cursor_line: &cursor_line,
        })
    }

    #[cfg(unix)]
    fn terminal_mode(&self, state: &State) -> (Foreground, bool) {
        let Some(master) = state.master.as_ref() else {
            return (Foreground::Unknown, false);
        };
        let mut termios: libc::termios = unsafe { std::mem::zeroed() };
        let password = master.as_raw_fd().is_some_and(|fd| {
            // SAFETY: `fd` stays open while the master is held under the state lock and
            // `termios` is a valid out pointer.
            let read = unsafe { libc::tcgetattr(fd, &mut termios) } == 0;
            read && (termios.c_lflag & libc::ECHO) == 0 && (termios.c_lflag & libc::ICANON) != 0
        });
        // An ssh client is always the foreground process locally, so this says nothing
        // about the remote shell.
        if self.plan.kind != "shell" {
            return (Foreground::Unknown, false);
        }
        let foreground = match (master.process_group_leader(), state.pid) {
            (Some(leader), Some(pid)) if leader as u32 == pid => Foreground::Shell,
            (Some(_), Some(_)) => Foreground::OtherProcess,
            _ => Foreground::Unknown,
        };
        (foreground, password)
    }

    #[cfg(not(unix))]
    fn terminal_mode(&self, _state: &State) -> (Foreground, bool) {
        (Foreground::Unknown, false)
    }

    #[cfg(test)]
    pub fn seq(&self) -> u64 {
        self.lock().seq
    }

    /// Screen plus scrollback, resolved through the terminal parser.
    pub fn snapshot(&self, max_lines: usize) -> Snapshot {
        let mut state = self.lock();
        catch_up(&mut state);
        Snapshot {
            rendered: state.screen.snapshot(max_lines),
            seq_end: state.seq,
            skipped_bytes: state.skipped_bytes,
        }
    }

    /// Renders output after `since`. Output evicted from the buffer is reported, never invented.
    pub fn delta(&self, since: u64, until: Option<u64>, max_lines: usize) -> Delta {
        let mut state = self.lock();
        let end = until.map_or(state.seq, |until| until.min(state.seq));
        let oldest = state.seq - state.ring.len() as u64;
        let start = since.max(oldest).min(end);
        let (rows, cols) = (state.rows, state.cols);
        let bytes = state.ring.make_contiguous();
        let slice = &bytes[(start - oldest) as usize..(end - oldest) as usize];
        Delta {
            rendered: render_bytes(slice, rows, cols, max_lines),
            seq_start: start,
            seq_end: end,
            missing_bytes: oldest.saturating_sub(since),
        }
    }
}

pub struct Snapshot {
    pub rendered: Rendered,
    pub seq_end: u64,
    pub skipped_bytes: u64,
}

pub struct Delta {
    pub rendered: Rendered,
    pub seq_start: u64,
    pub seq_end: u64,
    pub missing_bytes: u64,
}

fn send_input(state: &State, bytes: &[u8]) -> Result<(), String> {
    let writer = state.writer.as_ref().ok_or("The terminal is not accepting input.")?;
    writer.send(bytes.to_vec()).map_err(|_| "The terminal is not accepting input.".to_string())
}

pub fn ingest(state: &mut State, bytes: &[u8]) {
    state.ring.extend(bytes);
    state.seq += bytes.len() as u64;
    state.pending.extend_from_slice(bytes);
    let unparsed = (state.seq - state.parsed_seq) as usize;
    if unparsed > SKIM_AT {
        skim(state, unparsed - MAX_UNPARSED);
    }
    let excess = state.ring.len().saturating_sub(MAX_BUFFER);
    if excess > 0 {
        state.ring.drain(..excess);
    }
}

/// Brings the screen model up to date. Anything that reads screen state goes through here, so
/// a decision never rests on stale state, however far behind the parser was.
pub fn catch_up(state: &mut State) {
    let unparsed = (state.seq - state.parsed_seq) as usize;
    if unparsed == 0 {
        return;
    }
    if unparsed > MAX_UNPARSED {
        skim(state, unparsed - MAX_UNPARSED);
    }
    let unparsed = (state.seq - state.parsed_seq) as usize;
    let start = state.ring.len() - unparsed;
    let bytes = state.ring.make_contiguous();
    state.screen.process(&bytes[start..]);
    state.parsed_seq = state.seq;
    if state.screen.alternate_screen() != state.alternate_screen {
        state.alternate_screen = !state.alternate_screen;
        state.revision += 1;
    }
    for event in state.screen.take_events() {
        apply_shell_event(state, event);
    }
}

/// Passes the oldest unparsed bytes through without rendering them, keeping only the sequences
/// that change terminal modes or mark commands. Without this a burst could hide `vim` opening.
fn skim(state: &mut State, count: usize) {
    let unparsed = (state.seq - state.parsed_seq) as usize;
    let start = state.ring.len() - unparsed;
    let bytes = state.ring.make_contiguous();
    let kept = mode_sequences(&bytes[start..start + count]);
    state.screen.process(&kept);
    state.parsed_seq += count as u64;
    state.skipped_bytes += count as u64;
}

fn mode_sequences(bytes: &[u8]) -> Vec<u8> {
    let mut kept = Vec::new();
    let mut offset = 0;
    while let Some(found) = bytes[offset..].iter().position(|byte| *byte == 0x1b) {
        let rest = &bytes[offset + found..];
        let length = if rest.starts_with(b"\x1b[?") {
            rest.iter()
                .take(MAX_CSI_LENGTH)
                .position(u8::is_ascii_alphabetic)
                .map(|end| end + 1)
        } else if rest.starts_with(b"\x1b]133;") || rest.starts_with(b"\x1b]7;") {
            rest.iter()
                .take(MAX_OSC_LENGTH)
                .position(|byte| *byte == 0x07)
                .map(|end| end + 1)
        } else {
            None
        };
        match length {
            Some(length) => {
                kept.extend_from_slice(&rest[..length]);
                offset += found + length;
            }
            None => offset += found + 1,
        }
    }
    kept
}

fn apply_shell_event(state: &mut State, event: ShellEvent) {
    let seq = state.seq;
    match event {
        ShellEvent::WorkingDirectory(path) => state.observed_cwd = Some(path),
        // Unsigned marks may come from `cat` of a file; they never decide readiness.
        ShellEvent::PromptStart { trusted: true } | ShellEvent::CommandStart { trusted: true } => {
            state.phase = ShellPhase::AtPrompt;
        }
        ShellEvent::CommandExecuted { trusted: true } => {
            state.phase = ShellPhase::Running;
            state.command_started_at = Some(seq);
        }
        ShellEvent::CommandFinished { trusted: true, exit_code } => {
            state.phase = ShellPhase::AtPrompt;
            state.commands_finished += 1;
            state.last_exit_code = exit_code;
            let seq_start = state.command_started_at.take().unwrap_or(seq);
            state.commands.push_back(CommandRecord { exit_code, seq_start, seq_end: seq });
            if state.commands.len() > 32 {
                state.commands.pop_front();
            }
        }
        _ => {}
    }
}

/// Keeps an incomplete trailing UTF-8 sequence for the next frame, so every frame decodes.
pub fn take_complete_utf8(pending: &mut Vec<u8>, finished: bool) -> Vec<u8> {
    let keep = if finished { 0 } else { incomplete_suffix(pending) };
    let rest = pending.split_off(pending.len() - keep);
    std::mem::replace(pending, rest)
}

fn incomplete_suffix(bytes: &[u8]) -> usize {
    for back in 1..=bytes.len().min(3) {
        let byte = bytes[bytes.len() - back];
        if byte & 0xc0 == 0x80 {
            continue;
        }
        let needed = match byte {
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf7 => 4,
            _ => 1,
        };
        return if needed > back { back } else { 0 };
    }
    0
}

#[cfg(unix)]
fn terminate(pid: Option<u32>, killer: Option<Box<dyn ChildKiller + Send + Sync>>) {
    // forkpty creates a new session and process group. Signaling the group lets foreground
    // helpers hang up too; interactive shells manage their own job-control groups.
    if let Some(pid) = pid.and_then(|pid| i32::try_from(pid).ok()).filter(|pid| *pid > 0) {
        // SAFETY: kill(2) takes plain integers and reports failure through its return value.
        unsafe { libc::kill(-pid, libc::SIGHUP) };
    }
    if let Some(mut killer) = killer {
        let _ = killer.kill();
    }
}

#[cfg(not(unix))]
fn terminate(_pid: Option<u32>, killer: Option<Box<dyn ChildKiller + Send + Sync>>) {
    if let Some(mut killer) = killer {
        let _ = killer.kill();
    }
}

/// Batches output of all sessions onto the single frame writer.
#[derive(Default)]
pub struct Flusher {
    dirty: Mutex<(Vec<String>, bool)>,
    wake: Condvar,
}

impl Flusher {
    pub fn mark(&self, id: String, urgent: bool) {
        let mut dirty = self.dirty.lock().unwrap_or_else(|poison| poison.into_inner());
        if !dirty.0.contains(&id) {
            dirty.0.push(id);
        }
        dirty.1 |= urgent;
        drop(dirty);
        self.wake.notify_one();
    }

    pub fn run(&self, sessions: impl Fn(&str) -> Option<Arc<Session>>, outbox: &Outbox) {
        loop {
            let mut dirty = self.dirty.lock().unwrap_or_else(|poison| poison.into_inner());
            while dirty.0.is_empty() {
                dirty = self.wake.wait(dirty).unwrap_or_else(|poison| poison.into_inner());
            }
            if !dirty.1 {
                let (guard, _) = self
                    .wake
                    .wait_timeout_while(dirty, FLUSH_DELAY, |dirty| !dirty.1)
                    .unwrap_or_else(|poison| poison.into_inner());
                dirty = guard;
            }
            let ids = std::mem::take(&mut dirty.0);
            dirty.1 = false;
            drop(dirty);
            for id in ids {
                if let Some(session) = sessions(&id) {
                    session.flush(outbox);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session::new("s".to_string(), Plan { kind: "shell".to_string(), ..Plan::default() }, "n".to_string())
    }

    #[test]
    fn utf8_sequences_split_across_reads_stay_whole() {
        let text = "中🙂".as_bytes();
        for cut in 0..=text.len() {
            let mut pending = text[..cut].to_vec();
            let first = take_complete_utf8(&mut pending, false);
            pending.extend_from_slice(&text[cut..]);
            let second = take_complete_utf8(&mut pending, false);
            assert!(std::str::from_utf8(&first).is_ok());
            assert_eq!([first, second].concat(), text);
        }
    }

    #[test]
    fn the_last_frame_of_a_session_keeps_trailing_bytes() {
        let mut pending = vec![b'a', 0xe4];
        assert_eq!(take_complete_utf8(&mut pending, true), [b'a', 0xe4]);
    }

    #[test]
    fn replay_keeps_the_latest_two_mib_on_a_character_boundary() {
        let session = session();
        let mut state = session.lock();
        ingest(&mut state, b"discard this");
        ingest(&mut state, "中文🙂".repeat(300_000).as_bytes());
        drop(state);
        let replay = session.replay();
        let data = replay["data"].as_str().expect("text");
        assert!(data.len() <= MAX_BUFFER);
        assert!(!data.contains("discard this") && !data.contains('\u{fffd}'));
        assert!(data.ends_with("中文🙂"));
    }

    #[test]
    fn deltas_track_sequence_ranges_and_report_evicted_output() {
        let session = session();
        let mut state = session.lock();
        ingest(&mut state, b"first\r\n");
        drop(state);
        let mark = session.seq();
        let mut state = session.lock();
        ingest(&mut state, b"second\r\n");
        drop(state);
        let delta = session.delta(mark, None, 100);
        assert_eq!(delta.rendered.lines, ["second"]);
        assert_eq!((delta.seq_start, delta.missing_bytes), (mark, 0));

        let mut state = session.lock();
        ingest(&mut state, &vec![b'x'; MAX_BUFFER]);
        drop(state);
        let delta = session.delta(0, None, 1);
        assert_eq!(delta.missing_bytes, delta.seq_start);
        assert!(delta.missing_bytes > 0);
    }

    #[test]
    fn output_does_not_move_the_revision_but_screen_mode_changes_do() {
        let session = session();
        let mut state = session.lock();
        ingest(&mut state, b"plenty of logs\r\n");
        catch_up(&mut state);
        assert_eq!(state.revision, 0);
        ingest(&mut state, b"\x1b[?1049h");
        catch_up(&mut state);
        assert_eq!(state.revision, 1);
        ingest(&mut state, b"\x1b[?1049l");
        catch_up(&mut state);
        assert_eq!(state.revision, 2);
    }

    #[test]
    fn output_is_not_parsed_until_the_screen_is_read() {
        let session = session();
        let mut state = session.lock();
        ingest(&mut state, b"\x1b[?1049h");
        assert_eq!(state.parsed_seq, 0);
        let interaction = session.interaction(&mut state);
        assert_eq!(interaction.state, crate::interaction::InteractionState::Exited);
        assert!(state.screen.alternate_screen());
        assert_eq!(state.revision, 1);
    }

    #[test]
    fn a_burst_cannot_hide_a_full_screen_program() {
        let session = session();
        let mut state = session.lock();
        // Well past the replay buffer, with nobody reading the screen in between.
        ingest(&mut state, b"\x1b[?2004h");
        ingest(&mut state, b"\x1b[?1049h");
        for _ in 0..80 {
            ingest(&mut state, &[b'x'; 64 * 1024]);
        }
        catch_up(&mut state);
        assert!(state.screen.alternate_screen());
        assert!(state.screen.bracketed_paste());
        assert_eq!(state.revision, 1);
        assert_eq!(state.skipped_bytes + MAX_UNPARSED as u64, state.seq);
        let snapshot = {
            drop(state);
            session.snapshot(10)
        };
        assert!(snapshot.skipped_bytes > 0);
    }

    #[test]
    fn a_burst_keeps_command_marks_and_their_order() {
        let session = session();
        let mut state = session.lock();
        ingest(&mut state, b"\x1b]133;C;k=n\x07");
        ingest(&mut state, &[b'x'; 3 * 1024 * 1024]);
        ingest(&mut state, b"\x1b]133;D;9;k=n\x07");
        ingest(&mut state, &[b'y'; 3 * 1024 * 1024]);
        catch_up(&mut state);
        assert_eq!(state.commands_finished, 1);
        assert_eq!(state.last_exit_code, Some(9));
        assert_eq!(state.phase, ShellPhase::AtPrompt);
    }

    #[test]
    fn skimming_drops_text_and_ordinary_sequences() {
        let bytes = b"text\x1b[31m red\x1b[?25l more\x1b]0;title\x07\x1b]7;file:///a\x07 end\x1b[?1";
        assert_eq!(mode_sequences(bytes), b"\x1b[?25l\x1b]7;file:///a\x07");
    }

    #[test]
    fn trusted_marks_track_commands_and_forged_marks_are_ignored() {
        let session = session();
        let mut state = session.lock();
        ingest(&mut state, b"\x1b]133;A;k=n\x07$ \x1b]133;C;k=n\x07out\x1b]133;D;7;k=n\x07");
        catch_up(&mut state);
        assert_eq!(state.commands_finished, 1);
        assert_eq!(state.last_exit_code, Some(7));
        assert_eq!(state.phase, ShellPhase::AtPrompt);
        ingest(&mut state, b"\x1b]133;C;k=n\x07");
        ingest(&mut state, b"\x1b]133;A\x07\x1b]133;D;0\x07");
        catch_up(&mut state);
        assert_eq!(state.phase, ShellPhase::Running);
        assert_eq!(state.commands_finished, 1);
    }

    #[test]
    fn replies_do_not_count_as_user_input() {
        let session = session();
        session.lock().status = Status::Running;
        let (input, queue) = mpsc::channel();
        session.lock().writer = Some(input);
        session.write_user("\x1b[12;40R", true).expect("written");
        assert_eq!(session.lock().revision, 0);
        session.write_user("l", false).expect("written");
        assert_eq!(session.lock().revision, 1);
        assert!(session.lock().pending_input);
        session.write_user("s\r", false).expect("written");
        assert!(!session.lock().pending_input);
        assert_eq!(queue.try_iter().count(), 3);
    }

    #[test]
    fn writes_after_exit_are_refused_quietly() {
        let session = session();
        assert_eq!(session.write_user("x", false), Ok(false));
        assert!(session.write_agent(b"x").is_err());
    }
}

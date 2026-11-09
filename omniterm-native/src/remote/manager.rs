// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! One companion per SSH tab: it waits for the terminal's login to finish, opens an SFTP
//! channel and a metrics channel over the shared connection, and runs file operations and a
//! transfer queue on its own threads. The UI reads a snapshot each frame and never waits.

use super::metrics::{self, Sample, Tracker};
use super::sftp::{self, Client};
use crate::core::plan::Companion;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(500);
const HANDSHAKE: Duration = Duration::from_secs(20);
const PROGRESS: Duration = Duration::from_millis(150);
const CONCURRENT_TRANSFERS: usize = 2;
const MAX_STDERR: usize = 4096;
const MAX_METRICS_BUFFER: usize = 256 * 1024;
const MAX_DEPTH: usize = 64;
const MAX_FINISHED: usize = 20;
/// The most a dragged item may hold: the OS drag needs the whole thing on disk first.
const MAX_DRAG_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    NoSftp,
    ChannelRefused,
    HostKey,
    Auth,
    Lost,
    Failed,
}

impl Reason {
    pub fn message(self) -> &'static str {
        match self {
            Reason::NoSftp => "SSH 服务器未提供或不允许 SFTP，文件面板不可用。",
            Reason::ChannelRefused => "SSH 服务器拒绝新增会话通道。堡垒机菜单登录可能不支持共享 SFTP，请使用管理员提供的资产 SFTP 入口。",
            Reason::HostKey => "SSH 主机身份验证失败，请核对主机指纹和 known_hosts。",
            Reason::Auth => "文件连接的 SSH 认证未通过。请检查认证方式与权限；终端登录成功不代表文件通道可复用该登录。",
            Reason::Lost => "SFTP 连接已断开。",
            Reason::Failed => "无法打开 SFTP 连接。",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Waiting,
    Connecting,
    Ready,
    Unavailable { reason: Reason, detail: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct FileEntry {
    pub name: String,
    pub directory: bool,
    pub symlink: bool,
    pub size: u64,
    pub mtime: u32,
    pub mode: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Listing {
    pub path: Option<String>,
    pub entries: Vec<FileEntry>,
    pub error: Option<String>,
    pub loading: bool,
    ticket: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferStatus {
    Queued,
    Running,
    Done,
    Failed,
    Canceled,
}

#[derive(Clone, Debug)]
pub struct Transfer {
    pub id: u64,
    pub upload: bool,
    pub name: String,
    pub bytes: u64,
    pub total: u64,
    pub status: TransferStatus,
    pub skipped: usize,
    pub error: String,
    canceled: Arc<AtomicBool>,
}

impl Transfer {
    pub(crate) fn queued(id: u64, upload: bool, name: String) -> Self {
        Self { id, upload, name, bytes: 0, total: 0, status: TransferStatus::Queued, skipped: 0, error: String::new(), canceled: Arc::default() }
    }

    pub fn finished(&self) -> bool {
        matches!(self.status, TransferStatus::Done | TransferStatus::Failed | TransferStatus::Canceled)
    }
}

/// A remote item fetched for a drag out of the window: the OS can only drag a finished
/// local file, so the first drag downloads it and the next one hands it over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DragFile {
    Preparing,
    Ready(PathBuf),
}

/// What the UI shows for one remote.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub phase: Phase,
    pub home: Option<String>,
    pub listing: Listing,
    pub transfers: Vec<Transfer>,
    pub metrics: Option<Sample>,
    pub metrics_available: bool,
    pub uploads_done: u64,
    /// Dragged items by remote path.
    pub drags: HashMap<String, DragFile>,
}

/// A transfer's work: given the remote and the transfer ID, the number of items skipped.
type Work = Box<dyn FnOnce(&Remote, u64) -> Result<usize, String> + Send>;

struct Job {
    transfer: u64,
    run: Work,
    drag_source: Option<String>,
}

struct State {
    phase: Phase,
    home: Option<String>,
    client: Option<Arc<Client>>,
    sftp_child: Option<Arc<Mutex<Child>>>,
    metrics_child: Option<Arc<Mutex<Child>>>,
    metrics_wanted: bool,
    metrics: Option<Sample>,
    metrics_available: bool,
    linked: bool,
    listing: Listing,
    transfers: Vec<Transfer>,
    queue: VecDeque<Job>,
    running: usize,
    uploads_done: u64,
    drags: HashMap<String, DragFile>,
    /// A private directory for dragged items, removed when the remote closes.
    drag_directory: Option<PathBuf>,
    /// Exact temporary paths created by this remote, retried after a lost connection.
    partial_uploads: HashSet<String>,
}

pub struct Remote {
    id: String,
    target: Companion,
    state: Mutex<State>,
    closed: AtomicBool,
    wake: Arc<dyn Fn() + Send + Sync>,
    next: AtomicU64,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A process with no console window on Windows, where a GUI app's children would get one.
pub fn command(program: &str) -> Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
}

fn failure_reason(stderr: &str, fallback: Reason) -> Reason {
    let lower = stderr.to_lowercase();
    if lower.contains("session open refused") || lower.contains("session request failed") || lower.contains("administratively prohibited") {
        Reason::ChannelRefused
    } else if lower.contains("subsystem request failed") || (lower.contains("subsystem") && (lower.contains("not") || lower.contains("denied"))) {
        Reason::NoSftp
    } else if lower.contains("host key verification failed") || lower.contains("remote host identification has changed") {
        Reason::HostKey
    } else if lower.contains("permission denied") || lower.contains("no supported authentication") {
        Reason::Auth
    } else {
        fallback
    }
}

/// A server chooses the names it lists, so every name must stay inside the local directory.
pub fn safe_local_name(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." || name.len() > 255 || name.contains('/') || name.contains('\0') {
        return false;
    }
    if cfg!(windows) {
        let stem = name.split('.').next().unwrap_or_default().to_ascii_lowercase();
        let reserved = matches!(stem.as_str(), "con" | "prn" | "aux" | "nul") || ((stem.starts_with("com") || stem.starts_with("lpt")) && stem[3..].chars().all(|c| c.is_ascii_digit()) && stem.len() > 3);
        if name.chars().any(|c| "<>:\"\\|?*".contains(c) || c.is_control()) || name.ends_with('.') || name.ends_with(' ') || reserved {
            return false;
        }
    }
    true
}

/// A new directory under `parent` that only this user can read.
fn private_directory(parent: &Path, prefix: &str) -> Result<PathBuf, String> {
    let directory = parent.join(format!("{prefix}{}", uuid::Uuid::new_v4().simple()));
    #[cfg(unix)]
    let builder = {
        let mut builder = std::fs::DirBuilder::new();
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder
    };
    // Directories under the user's own temporary folder are private to them on Windows.
    #[cfg(not(unix))]
    let builder = std::fs::DirBuilder::new();
    builder.create(&directory).map_err(|error| format!("无法创建临时目录：{error}"))?;
    Ok(directory)
}

pub fn join(directory: &str, name: &str) -> String {
    if directory.ends_with('/') { format!("{directory}{name}") } else { format!("{directory}/{name}") }
}

pub fn parent(directory: &str) -> String {
    let trimmed = directory.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) | None => "/".into(),
        Some(index) => trimmed[..index].into(),
    }
}

fn basename(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path)
}

impl Remote {
    fn channel_args(&self) -> Vec<String> {
        let control = match &self.target.control_path {
            Some(path) => format!("ControlPath={path}"),
            None => "ControlPath=none".into(),
        };
        let mut args: Vec<String> = ["-o", "BatchMode=yes", "-o", "ControlMaster=no", "-o", &control, "-o", "ClearAllForwardings=yes", "-o", "RemoteCommand=none", "-o", "RequestTTY=no", "-o", "ConnectTimeout=15"].iter().map(|s| s.to_string()).collect();
        if self.target.control_path.is_some() {
            // Refused mux channels must not fall back to a fresh MFA login.
            args.extend(["-o".into(), "ProxyCommand=false".into()]);
        }
        args.extend(self.target.args.iter().cloned());
        args
    }

    fn publish(&self, change: impl FnOnce(&mut State)) {
        change(&mut lock(&self.state));
        (self.wake)();
    }

    pub fn snapshot(&self) -> Snapshot {
        let state = lock(&self.state);
        Snapshot {
            phase: state.phase.clone(),
            home: state.home.clone(),
            listing: state.listing.clone(),
            transfers: state.transfers.clone(),
            metrics: state.metrics.clone(),
            metrics_available: state.metrics_available,
            uploads_done: state.uploads_done,
            drags: state.drags.clone(),
        }
    }

    fn client(&self) -> Result<Arc<Client>, String> {
        let state = lock(&self.state);
        match (&state.phase, &state.client) {
            (Phase::Ready, Some(client)) => Ok(client.clone()),
            _ => Err("SFTP 连接还没有准备好。".into()),
        }
    }

    /// The control socket appears only after authentication finishes, however long the user
    /// spends on a host key prompt, password or second factor.
    fn wait_for_login(self: &Arc<Self>) {
        let remote = self.clone();
        std::thread::spawn(move || {
            if let Some(path) = remote.target.control_path.clone() {
                while !Path::new(&path).exists() {
                    if remote.closed.load(Ordering::Acquire) {
                        return;
                    }
                    std::thread::sleep(POLL);
                }
            }
            if remote.closed.load(Ordering::Acquire) { return; }
            lock(&remote.state).linked = true;
            if lock(&remote.state).metrics_wanted {
                remote.start_metrics();
            }
            remote.connect();
        });
    }

    fn connect(self: &Arc<Self>) {
        if self.closed.load(Ordering::Acquire) { return; }
        self.publish(|state| state.phase = Phase::Connecting);
        let mut args = self.channel_args();
        args.extend(["-s".to_string(), self.target.destination.clone(), "sftp".to_string()]);
        let spawned = command(&self.target.executable).args(&args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(error) => {
                self.publish(|state| state.phase = Phase::Unavailable { reason: Reason::Failed, detail: error.to_string() });
                return;
            }
        };
        let (Some(stdin), Some(stdout), Some(mut stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else { return };
        let child = Arc::new(Mutex::new(child));
        {
            let mut state = lock(&self.state);
            if self.closed.load(Ordering::Acquire) {
                drop(state);
                let _ = lock(&child).kill();
                let _ = lock(&child).wait();
                return;
            }
            state.sftp_child = Some(child.clone());
        }
        let errors = Arc::new(Mutex::new(String::new()));
        {
            let errors = errors.clone();
            std::thread::spawn(move || {
                let mut buffer = [0; 1024];
                while let Ok(read) = stderr.read(&mut buffer) {
                    if read == 0 {
                        break;
                    }
                    let mut text = lock(&errors);
                    text.push_str(&String::from_utf8_lossy(&buffer[..read]));
                    let excess = text.len().saturating_sub(MAX_STDERR);
                    if excess > 0 {
                        let cut = (excess..text.len()).find(|index| text.is_char_boundary(*index)).unwrap_or(text.len());
                        text.drain(..cut);
                    }
                }
            });
        }
        // The handshake runs on a watchdog: a server that never answers ends the attempt.
        let (done, waiting) = std::sync::mpsc::channel();
        {
            let child = child.clone();
            std::thread::spawn(move || {
                if waiting.recv_timeout(HANDSHAKE).is_err() {
                    let _ = lock(&child).kill();
                }
            });
        }
        let result = Client::start(Box::new(stdin), Box::new(stdout)).and_then(|client| client.realpath(".").map(|home| (client, home)));
        let _ = done.send(());
        let has_control = self.target.control_path.is_some();
        match result {
            Ok((client, home)) => {
                let client = Arc::new(client);
                if self.closed.load(Ordering::Acquire) {
                    let _ = lock(&child).kill();
                    let _ = lock(&child).wait();
                    return;
                }
                let partials: Vec<String> = lock(&self.state).partial_uploads.iter().cloned().collect();
                for path in partials {
                    self.remove_partial(&client, &path);
                }
                self.publish(|state| {
                    state.client = Some(client.clone());
                    state.home = Some(home.clone());
                    state.phase = Phase::Ready;
                });
                // When the channel ends, the panel says so and offers a retry.
                let remote = self.clone();
                std::thread::spawn(move || {
                    // Never hold the child mutex while waiting: close() needs it to kill
                    // a live SSH channel. A blocking wait here deadlocks tab shutdown.
                    loop {
                        let alive = lock(&child).try_wait().is_ok_and(|status| status.is_none());
                        if !alive { break; }
                        std::thread::sleep(POLL);
                    }
                    if remote.closed.load(Ordering::Acquire) || !lock(&remote.state).client.as_ref().is_some_and(|current| Arc::ptr_eq(current, &client)) {
                        return;
                    }
                    let detail = lock(&errors).trim().to_string();
                    remote.publish(|state| {
                        state.client = None;
                        state.phase = Phase::Unavailable { reason: Reason::Lost, detail };
                    });
                });
            }
            Err(error) => {
                let _ = lock(&child).kill();
                let _ = lock(&child).wait();
                std::thread::sleep(Duration::from_millis(50));
                let stderr = lock(&errors).trim().to_string();
                // Through a shared connection ssh prints nothing when the server refuses the
                // subsystem, because the refusal goes to the master process.
                let fallback = if has_control && stderr.is_empty() { Reason::NoSftp } else { Reason::Failed };
                let detail = if stderr.is_empty() { error.to_string() } else { stderr.clone() };
                let detail: String = detail.chars().rev().take(500).collect::<Vec<_>>().into_iter().rev().collect();
                self.publish(|state| state.phase = Phase::Unavailable { reason: failure_reason(&stderr, fallback), detail });
            }
        }
    }

    fn start_metrics(self: &Arc<Self>) {
        let mut args = self.channel_args();
        args.push(self.target.destination.clone());
        args.push(format!("sh -c '{}'", metrics::script()));
        let Ok(mut child) = command(&self.target.executable).args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn() else {
            self.publish(|state| state.metrics_available = false);
            return;
        };
        let Some(mut stdout) = child.stdout.take() else { return };
        let child = Arc::new(Mutex::new(child));
        lock(&self.state).metrics_child = Some(child.clone());
        let remote = self.clone();
        std::thread::spawn(move || {
            let mut tracker = Tracker::default();
            let mut buffer = String::new();
            let mut chunk = [0; 4096];
            while let Ok(read) = stdout.read(&mut chunk) {
                if read == 0 {
                    break;
                }
                buffer.push_str(&String::from_utf8_lossy(&chunk[..read]));
                if buffer.len() > MAX_METRICS_BUFFER {
                    let _ = lock(&child).kill();
                    break;
                }
                for text in metrics::take_samples(&mut buffer) {
                    let sample = tracker.next(metrics::parse(&text));
                    remote.publish(|state| {
                        state.metrics = Some(sample);
                        state.metrics_available = true;
                    });
                }
            }
            let _ = lock(&child).wait();
            remote.publish(|state| {
                if state.metrics_child.as_ref().is_some_and(|current| Arc::ptr_eq(current, &child)) {
                    state.metrics_child = None;
                    state.metrics_available = false;
                }
            });
        });
    }

    pub fn watch_metrics(self: &Arc<Self>, enabled: bool) {
        let start = {
            let mut state = lock(&self.state);
            state.metrics_wanted = enabled;
            if !enabled {
                if let Some(child) = state.metrics_child.take() {
                    let _ = lock(&child).kill();
                }
                false
            } else {
                state.linked && state.metrics_child.is_none()
            }
        };
        if start {
            self.start_metrics();
        }
    }

    pub fn retry(self: &Arc<Self>) {
        let (linked, unavailable, metrics) = {
            let state = lock(&self.state);
            (state.linked, matches!(state.phase, Phase::Unavailable { .. }), state.metrics_wanted && state.metrics_child.is_none())
        };
        if !linked {
            return;
        }
        if unavailable {
            let remote = self.clone();
            std::thread::spawn(move || remote.connect());
        }
        if metrics {
            self.start_metrics();
        }
    }

    /// Lists a directory in the background; the newest request wins.
    pub fn list(self: &Arc<Self>, path: String) {
        let ticket = {
            let mut state = lock(&self.state);
            state.listing.ticket += 1;
            state.listing.loading = true;
            state.listing.ticket
        };
        (self.wake)();
        let remote = self.clone();
        std::thread::spawn(move || {
            let result = remote.client().and_then(|client| {
                let directory = client.realpath(&path).map_err(|error| error.to_string())?;
                let entries = client.readdir(&directory).map_err(|error| error.to_string())?;
                let entries = entries
                    .into_iter()
                    .map(|entry| {
                        // Symlinks report their own metadata; resolve them so linked folders open.
                        let target = if entry.attributes.symlink() { client.stat(&join(&directory, &entry.name)).ok() } else { None };
                        let shown = target.unwrap_or(entry.attributes);
                        FileEntry { directory: shown.directory(), symlink: entry.attributes.symlink(), size: shown.size.unwrap_or(0), mtime: shown.mtime.unwrap_or(0), mode: entry.attributes.mode.unwrap_or(0), name: entry.name }
                    })
                    .collect();
                Ok((directory, entries))
            });
            remote.publish(|state| {
                if state.listing.ticket != ticket {
                    return;
                }
                state.listing.loading = false;
                match result {
                    Ok((directory, entries)) => {
                        state.listing.path = Some(directory);
                        state.listing.entries = entries;
                        state.listing.error = None;
                    }
                    Err(error) => state.listing.error = Some(error),
                }
            });
        });
    }

    /// Runs a file operation in the background and reports a failure through `done`.
    pub fn operate(self: &Arc<Self>, operation: impl FnOnce(&Client) -> sftp::Result<()> + Send + 'static, done: impl FnOnce(Result<(), String>) + Send + 'static) {
        let remote = self.clone();
        std::thread::spawn(move || {
            let result = remote.client().and_then(|client| operation(&client).map_err(|error| error.to_string()));
            done(result);
            (remote.wake)();
        });
    }

    /// Paths of `names` that already exist where they would land, so the user can decide.
    pub fn upload_conflicts(&self, paths: &[PathBuf], directory: &str) -> Result<Vec<String>, String> {
        let client = self.client()?;
        let mut conflicts = Vec::new();
        for path in paths {
            let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            if client.exists(&join(directory, &name)).map_err(|error| error.to_string())? {
                conflicts.push(name);
            }
        }
        Ok(conflicts)
    }

    pub fn upload(self: &Arc<Self>, paths: Vec<PathBuf>, directory: String) {
        for source in paths {
            let name = source.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            let target = join(&directory, &name);
            self.enqueue(true, name, None, Box::new(move |remote, id| remote.run_upload(id, &source, &target)));
        }
    }

    pub fn download(self: &Arc<Self>, paths: Vec<String>, directory: PathBuf) -> Result<(), String> {
        for source in &paths {
            if !safe_local_name(basename(source)) {
                return Err(format!("拒绝下载不安全的文件名：{:?}", basename(source)));
            }
        }
        for source in paths {
            let name = basename(&source).to_string();
            let target = directory.join(&name);
            self.enqueue(false, name, None, Box::new(move |remote, id| remote.run_download(id, &source, &target, None)));
        }
        Ok(())
    }

    /// Fetches a remote item into a private temporary directory for a drag out of the
    /// window; the snapshot's `drags` follows it from preparing to ready.
    pub fn prepare_drag(self: &Arc<Self>, source: String) -> Result<(), String> {
        let name = basename(&source).to_string();
        if !safe_local_name(&name) {
            return Err(format!("拒绝下载不安全的文件名：{name:?}"));
        }
        let target = {
            let mut state = lock(&self.state);
            // Already fetched or on its way; a fetched file someone has since removed is fetched again.
            match state.drags.get(&source) {
                Some(DragFile::Preparing) => return Ok(()),
                Some(DragFile::Ready(file)) if file.is_file() || file.is_dir() => return Ok(()),
                _ => {}
            }
            let root = match &state.drag_directory {
                Some(root) => root.clone(),
                None => {
                    let root = private_directory(&std::env::temp_dir(), "omniterm-drag-")?;
                    state.drag_directory = Some(root.clone());
                    root
                }
            };
            // One directory per item, so two files of the same name never collide.
            let target = private_directory(&root, "")?.join(&name);
            state.drags.insert(source.clone(), DragFile::Preparing);
            target
        };
        self.enqueue(
            false,
            name,
            Some(source.clone()),
            Box::new(move |remote, id| {
                let result = remote.run_download(id, &source, &target, Some(MAX_DRAG_BYTES));
                remote.publish(|state| {
                    if result.is_ok() {
                        state.drags.insert(source, DragFile::Ready(target));
                    } else {
                        state.drags.remove(&source);
                    }
                });
                result
            }),
        );
        Ok(())
    }

    fn enqueue(self: &Arc<Self>, upload: bool, name: String, drag_source: Option<String>, run: Work) {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.publish(|state| {
            state.transfers.push(Transfer::queued(id, upload, name));
            state.queue.push_back(Job { transfer: id, run, drag_source });
        });
        self.pump();
    }

    fn pump(self: &Arc<Self>) {
        loop {
            let job = {
                let mut state = lock(&self.state);
                if self.closed.load(Ordering::Acquire) || state.running >= CONCURRENT_TRANSFERS {
                    return;
                }
                let Some(job) = state.queue.pop_front() else { return };
                state.running += 1;
                if let Some(transfer) = state.transfers.iter_mut().find(|transfer| transfer.id == job.transfer) {
                    transfer.status = TransferStatus::Running;
                }
                job
            };
            (self.wake)();
            let remote = self.clone();
            std::thread::spawn(move || {
                let result = (job.run)(&remote, job.transfer);
                remote.publish(|state| {
                    state.running -= 1;
                    if let Some(transfer) = state.transfers.iter_mut().find(|transfer| transfer.id == job.transfer) {
                        match result {
                            Ok(skipped) => {
                                transfer.status = TransferStatus::Done;
                                transfer.skipped = skipped;
                                if transfer.upload {
                                    state.uploads_done += 1;
                                }
                            }
                            Err(_) if transfer.canceled.load(Ordering::Relaxed) => transfer.status = TransferStatus::Canceled,
                            Err(error) => {
                                transfer.status = TransferStatus::Failed;
                                transfer.error = error;
                            }
                        }
                    }
                    let finished: Vec<u64> = state.transfers.iter().filter(|transfer| transfer.finished()).map(|transfer| transfer.id).collect();
                    let stale = finished.len().saturating_sub(MAX_FINISHED);
                    let stale: Vec<u64> = finished.into_iter().take(stale).collect();
                    state.transfers.retain(|transfer| !stale.contains(&transfer.id));
                });
                remote.pump();
            });
        }
    }

    pub fn cancel(&self, transfer: u64) {
        self.publish(|state| {
            if let Some(item) = state.transfers.iter_mut().find(|item| item.id == transfer && !item.finished()) {
                item.canceled.store(true, Ordering::Relaxed);
                if item.status == TransferStatus::Queued {
                    item.status = TransferStatus::Canceled;
                    if let Some(source) = state.queue.iter().find(|job| job.transfer == transfer).and_then(|job| job.drag_source.as_ref()) {
                        state.drags.remove(source);
                    }
                    state.queue.retain(|job| job.transfer != transfer);
                }
            }
        });
    }

    pub fn clear_finished(&self) {
        lock(&self.state).transfers.retain(|transfer| !transfer.finished());
    }

    fn progress(&self, id: u64) -> impl FnMut(u64) + '_ {
        let mut last = Instant::now();
        move |bytes| {
            let mut state = lock(&self.state);
            if let Some(transfer) = state.transfers.iter_mut().find(|transfer| transfer.id == id) {
                transfer.bytes += bytes;
            }
            drop(state);
            if last.elapsed() >= PROGRESS {
                last = Instant::now();
                (self.wake)();
            }
        }
    }

    fn add_total(&self, id: u64, bytes: u64) {
        if let Some(transfer) = lock(&self.state).transfers.iter_mut().find(|transfer| transfer.id == id) {
            transfer.total += bytes;
        }
    }

    fn canceled(&self, id: u64) -> Arc<AtomicBool> {
        lock(&self.state).transfers.iter().find(|transfer| transfer.id == id).map(|transfer| transfer.canceled.clone()).unwrap_or_default()
    }

    fn remove_partial(&self, client: &Client, path: &str) {
        if client.remove(path).is_ok() || client.exists(path).is_ok_and(|exists| !exists) {
            lock(&self.state).partial_uploads.remove(path);
        }
    }

    /// Links are skipped below the top level: following them can loop forever or send
    /// files from outside the folder the user chose. Files go up as `.part` and are moved
    /// into place only when complete.
    fn run_upload(&self, id: u64, source: &Path, target: &str) -> Result<usize, String> {
        let client = self.client()?;
        let canceled = self.canceled(id);
        enum Step {
            Directory(String),
            File(PathBuf, String, u32),
        }
        let mut plan = Vec::new();
        let mut skipped = 0;
        let mut stack = vec![(source.to_path_buf(), target.to_string(), 0usize, true)];
        while let Some((path, remote, depth, top)) = stack.pop() {
            if canceled.load(Ordering::Relaxed) {
                return Err("传输已取消。".into());
            }
            let meta = if top { std::fs::metadata(&path) } else { std::fs::symlink_metadata(&path) }.map_err(|error| error.to_string())?;
            if depth > MAX_DEPTH {
                skipped += 1;
            } else if meta.is_dir() {
                plan.push(Step::Directory(remote.clone()));
                let mut children: Vec<_> = std::fs::read_dir(&path).map_err(|error| error.to_string())?.flatten().collect();
                children.sort_by_key(|entry| std::cmp::Reverse(entry.file_name()));
                for entry in children {
                    stack.push((entry.path(), join(&remote, &entry.file_name().to_string_lossy()), depth + 1, false));
                }
            } else if meta.is_file() {
                self.add_total(id, meta.len());
                #[cfg(unix)]
                let mode = { use std::os::unix::fs::PermissionsExt; meta.permissions().mode() & 0o777 };
                #[cfg(not(unix))]
                let mode = 0o644;
                plan.push(Step::File(path, remote, mode));
            } else {
                skipped += 1;
            }
        }
        (self.wake)();
        let mut progress = self.progress(id);
        for step in plan {
            if canceled.load(Ordering::Relaxed) {
                return Err("传输已取消。".into());
            }
            match step {
                Step::Directory(path) => match client.stat(&path) {
                    Ok(existing) if !existing.directory() => return Err(format!("{path} 已经是一个文件。")),
                    Ok(_) => {}
                    Err(_) => client.mkdir(&path).map_err(|error| error.to_string())?,
                },
                Step::File(local, remote, mode) => {
                    let partial = format!("{remote}.omniterm-{}.part", &uuid::Uuid::new_v4().simple().to_string()[..8]);
                    lock(&self.state).partial_uploads.insert(partial.clone());
                    let result = client.upload(&local, &partial, Some(mode), &canceled, &mut progress).and_then(|_| client.replace(&partial, &remote));
                    if let Err(error) = result {
                        self.remove_partial(&client, &partial);
                        return Err(error.to_string());
                    }
                    lock(&self.state).partial_uploads.remove(&partial);
                }
            }
        }
        Ok(skipped)
    }

    fn run_download(&self, id: u64, source: &str, target: &Path, limit: Option<u64>) -> Result<usize, String> {
        let client = self.client()?;
        let canceled = self.canceled(id);
        enum Step {
            Directory(PathBuf),
            File(String, PathBuf, u32),
        }
        let mut plan = Vec::new();
        let mut skipped = 0;
        let mut total = 0u64;
        let top = client.stat(source).map_err(|error| error.to_string())?;
        let mut stack = vec![(source.to_string(), target.to_path_buf(), top, 0usize)];
        while let Some((remote, local, attributes, depth)) = stack.pop() {
            if canceled.load(Ordering::Relaxed) {
                return Err("传输已取消。".into());
            }
            if depth > MAX_DEPTH {
                skipped += 1;
            } else if attributes.directory() {
                plan.push(Step::Directory(local.clone()));
                let mut entries = client.readdir(&remote).map_err(|error| error.to_string())?;
                entries.sort_by(|a, b| b.name.cmp(&a.name));
                for entry in entries {
                    if !safe_local_name(&entry.name) || !(entry.attributes.directory() || entry.attributes.file()) {
                        skipped += 1;
                        continue;
                    }
                    stack.push((join(&remote, &entry.name), local.join(&entry.name), entry.attributes, depth + 1));
                }
            } else if attributes.file() {
                let size = attributes.size.unwrap_or(0);
                total = total.saturating_add(size);
                if let Some(limit) = limit
                    && total > limit
                {
                    return Err(format!("超过 {} GiB 的项目不能拖出，请用“下载”。", limit >> 30));
                }
                self.add_total(id, size);
                plan.push(Step::File(remote, local, attributes.mode.unwrap_or(0o644) & 0o777));
            } else {
                skipped += 1;
            }
        }
        (self.wake)();
        let mut progress = self.progress(id);
        for step in plan {
            if canceled.load(Ordering::Relaxed) {
                return Err("传输已取消。".into());
            }
            match step {
                Step::Directory(path) => std::fs::create_dir_all(&path).map_err(|error| error.to_string())?,
                Step::File(remote, local, mode) => {
                    let directory = local.parent().map(Path::to_path_buf).unwrap_or_default();
                    let partial = directory.join(format!(".{}.omniterm-part", &uuid::Uuid::new_v4().simple().to_string()[..8]));
                    // The user confirmed replacing an existing file; Windows renames refuse one.
                    let result = client.download(&remote, &partial, Some(mode), &canceled, &mut progress).map_err(|error| error.to_string()).and_then(|_| {
                        if cfg!(windows) && local.is_file() {
                            std::fs::remove_file(&local).map_err(|error| error.to_string())?;
                        }
                        std::fs::rename(&partial, &local).map_err(|error| error.to_string())
                    });
                    if let Err(error) = result {
                        let _ = std::fs::remove_file(&partial);
                        return Err(error);
                    }
                }
            }
        }
        Ok(skipped)
    }

    /// Ends the channels and the shared login, which otherwise outlives the terminal for
    /// ControlPersist seconds.
    fn close(self: &Arc<Self>) -> Option<std::thread::JoinHandle<()>> {
        if self.closed.swap(true, Ordering::AcqRel) { return None; }
        {
            let mut state = lock(&self.state);
            for transfer in &mut state.transfers {
                transfer.canceled.store(true, Ordering::Relaxed);
                if transfer.status == TransferStatus::Queued { transfer.status = TransferStatus::Canceled; }
            }
            state.queue.clear();
            state.drags.clear();
        }
        let remote = self.clone();
        Some(std::thread::spawn(move || {
            // Give canceled workers a bounded chance to remove their temporary files
            // while the channel still works. A tab close never blocks the UI thread.
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                let running = lock(&remote.state).running;
                if running == 0 || Instant::now() >= deadline { break; }
                std::thread::sleep(Duration::from_millis(20));
            }
            let (children, directory) = {
                let mut state = lock(&remote.state);
                state.client = None;
                ([state.sftp_child.take(), state.metrics_child.take()], state.drag_directory.take())
            };
            for child in children.into_iter().flatten() { let _ = lock(&child).kill(); }
            if let Some(directory) = directory { let _ = std::fs::remove_dir_all(directory); }
            if let Some(path) = &remote.target.control_path
                && Path::new(path).exists()
            {
                let _ = command(&remote.target.executable).args(["-o", &format!("ControlPath={path}"), "-O", "exit", &remote.target.destination]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
            }
        }))
    }
}

pub struct Manager {
    remotes: Mutex<HashMap<String, Arc<Remote>>>,
    wake: Arc<dyn Fn() + Send + Sync>,
    closing: Mutex<Vec<std::thread::JoinHandle<()>>>,
}

impl Manager {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self { remotes: Mutex::default(), wake: Arc::new(wake), closing: Mutex::default() }
    }

    pub fn attach(&self, id: &str, target: Companion) {
        let remote = Arc::new(Remote {
            id: id.to_string(),
            target,
            state: Mutex::new(State {
                phase: Phase::Waiting,
                home: None,
                client: None,
                sftp_child: None,
                metrics_child: None,
                metrics_wanted: false,
                metrics: None,
                metrics_available: false,
                linked: false,
                listing: Listing::default(),
                transfers: Vec::new(),
                queue: VecDeque::new(),
                running: 0,
                uploads_done: 0,
                drags: HashMap::new(),
                drag_directory: None,
                partial_uploads: HashSet::new(),
            }),
            closed: AtomicBool::new(false),
            wake: self.wake.clone(),
            next: AtomicU64::new(1),
        });
        lock(&self.remotes).insert(id.to_string(), remote.clone());
        remote.wait_for_login();
    }

    pub fn get(&self, id: &str) -> Option<Arc<Remote>> {
        lock(&self.remotes).get(id).cloned()
    }

    pub fn detach(&self, id: &str) {
        if let Some(remote) = lock(&self.remotes).remove(id)
            && let Some(worker) = remote.close()
        {
            let mut closing = lock(&self.closing);
            closing.retain(|worker| !worker.is_finished());
            closing.push(worker);
        }
    }

    pub fn shutdown(&self) {
        let remotes: Vec<_> = lock(&self.remotes).drain().map(|(_, remote)| remote).collect();
        let mut workers: Vec<_> = lock(&self.closing).drain(..).collect();
        workers.extend(remotes.iter().filter_map(|remote| remote.close()));
        // The window has exited; finish cleanup before main returns and stops all threads.
        for worker in workers { let _ = worker.join(); }
    }
}

impl Remote {
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Runs one read-only probe on the session's shared login and returns its output.
    pub fn exec(&self, script: &str, timeout: Duration) -> Result<String, String> {
        if !lock(&self.state).linked {
            return Err("这个 SSH 会话还没有可用的共享连接。".into());
        }
        let mut args = self.channel_args();
        args.push(self.target.destination.clone());
        args.push(format!("sh -c '{script}'"));
        let mut command = command(&self.target.executable);
        command.args(&args);
        crate::monitor::probe::run_with_timeout(&mut command, timeout).ok_or_else(|| "远程采样失败或超时。".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dragged_items_land_in_fresh_directories_only_this_user_reads() {
        let parent = std::env::temp_dir();
        let first = private_directory(&parent, "omniterm-test-").expect("create");
        let second = private_directory(&first, "").expect("create nested");
        assert_ne!(first, second);
        assert!(first.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with("omniterm-test-")));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&second).expect("stat").permissions().mode() & 0o777, 0o700);
        }
        std::fs::remove_dir_all(&first).expect("clean up");
    }

    #[test]
    fn server_names_never_escape_the_destination() {
        for name in ["", ".", "..", "a/b", "nul\0"] {
            assert!(!safe_local_name(name), "{name:?}");
        }
        assert!(safe_local_name("report.txt"));
        assert!(safe_local_name("日志 2026.log"));
    }

    #[test]
    fn remote_paths_join_and_climb() {
        assert_eq!(join("/", "a"), "/a");
        assert_eq!(join("/home/me", "a"), "/home/me/a");
        assert_eq!(parent("/home/me/"), "/home");
        assert_eq!(parent("/home"), "/");
        assert_eq!(parent("/"), "/");
    }

    #[test]
    fn ssh_errors_explain_why_the_panel_is_unavailable() {
        assert_eq!(failure_reason("subsystem request failed on channel 0", Reason::Failed), Reason::NoSftp);
        assert_eq!(failure_reason("Permission denied (publickey).", Reason::Failed), Reason::Auth);
        assert_eq!(failure_reason("mux_client_request_session: session request failed: Session open refused by peer\nPermission denied (password,keyboard-interactive,publickey).", Reason::Failed), Reason::ChannelRefused);
        assert_eq!(failure_reason("Host key verification failed.", Reason::Failed), Reason::HostKey);
        assert_eq!(failure_reason("", Reason::Lost), Reason::Lost);
    }
}

#[cfg(all(test, unix))]
#[path = "manager_tests.rs"]
mod integration_tests;

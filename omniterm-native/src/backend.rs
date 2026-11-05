// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! The terminal host, linked in. It is the same `Host` the desktop app runs as a child process,
//! driven through the same requests, so sessions, the screen model and the Agent broker behave
//! the same here. Output arrives as frames on one thread and goes straight into the emulators;
//! sessions open on a worker thread; the UI thread never waits on a PTY or a process.

use crate::core::plan::{self, Companion};
use crate::core::profile::{Kind, Profile};
use crate::core::settings::LocalShell;
use crate::terminal::{Options, Request, Size, Terminal};
use omniterm_terminal_host::host::Host;
use omniterm_terminal_host::protocol::{Frame, Outbox};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;

/// A session the UI asked for.
#[derive(Clone, Debug)]
pub struct Open {
    pub token: u64,
    pub kind: Kind,
    pub cwd: String,
    pub profile: Option<Profile>,
    pub companion: bool,
    pub local_shell: LocalShell,
    pub size: Size,
    pub options: Options,
}

/// What a session is, beyond its screen.
#[derive(Clone, Debug)]
pub struct Session {
    pub terminal: Arc<Terminal>,
    pub kind: Kind,
    pub title: String,
    pub cwd: String,
    pub powershell: bool,
    pub profile: Option<Profile>,
    pub companion: Option<Companion>,
    /// Marks the host trusts from this shell carry this nonce; see `shell_integration`.
    pub nonce: String,
    /// Shown once the session is open; see `plan::Plan::warning`.
    pub warning: Option<String>,
}

/// The readiness marks script for bash 4.4+ and zsh, from the desktop app's
/// `shell-integration.mjs`, for the user to submit in a shell they chose.
pub fn shell_integration(nonce: &str) -> Option<String> {
    const PLACEHOLDER: &str = "f00df00df00df00df00df00df00df00d";
    let valid = nonce.len() == 32 && nonce.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    valid.then(|| include_str!("../assets/shell-integration.txt").replace(PLACEHOLDER, nonce))
}

/// What the UI hears about, apart from output.
pub enum Notice {
    Opened { token: u64, result: Result<Box<Session>, String> },
    Exited { id: String, code: Option<i64> },
    Title { id: String, title: Option<String> },
    Directory { id: String, url: String },
    /// An event from the host that other parts of the app subscribe to (operations, monitors).
    Host(Value),
}

enum Write {
    Input { id: String, data: String, reply: bool },
    Resize { id: String, size: Size },
    Close(String),
}

pub struct Backend {
    host: Arc<Host>,
    terminals: Arc<Mutex<HashMap<String, Arc<Terminal>>>>,
    writes: Sender<Write>,
    opens: Sender<Open>,
    requests: Sender<Request>,
    control_path: fn() -> Option<String>,
    pub notices: Receiver<Notice>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Backend {
    /// `wake` is called whenever something on screen may have changed. `control_path` names a
    /// fresh socket for an SSH login shared with the file panel.
    pub fn new(wake: impl Fn() + Send + Sync + 'static, control_path: fn() -> Option<String>) -> Self {
        let wake = Arc::new(wake);
        let (outbox, frames) = Outbox::channel();
        let host = Host::new(outbox);
        let terminals: Arc<Mutex<HashMap<String, Arc<Terminal>>>> = Arc::default();
        let (notify, notices) = channel();
        let (requests, request_queue) = channel::<Request>();
        let (writes, write_queue) = channel::<Write>();
        let (opens, open_queue) = channel::<Open>();

        {
            let terminals = terminals.clone();
            let notify = notify.clone();
            let wake = wake.clone();
            thread::Builder::new()
                .name("omniterm-output".into())
                .spawn(move || {
                    for frame in frames {
                        match frame {
                            Frame::Data { session_id, bytes, .. } => {
                                let terminal = lock(&terminals).get(&session_id).cloned();
                                if let Some(terminal) = terminal {
                                    terminal.feed(&bytes);
                                }
                            }
                            Frame::Json(value) if value["event"] == "session.exit" => {
                                let id = value["params"]["id"].as_str().unwrap_or_default().to_string();
                                let code = value["params"]["exitCode"].as_i64();
                                if let Some(terminal) = lock(&terminals).get(&id).cloned() {
                                    terminal.finish(code);
                                }
                                let _ = notify.send(Notice::Exited { id, code });
                            }
                            Frame::Json(value) if value.get("event").is_some() => {
                                let _ = notify.send(Notice::Host(value));
                            }
                            Frame::Json(_) => {}
                        }
                        wake();
                    }
                })
                .expect("output thread");
        }

        // Every write goes through one thread, in order, so the UI never waits on a PTY.
        {
            let host = host.clone();
            thread::Builder::new()
                .name("omniterm-input".into())
                .spawn(move || {
                    for write in write_queue {
                        let _ = match write {
                            Write::Input { id, data, reply } => host.handle("session.write", json!({ "id": id, "data": data, "reply": reply })),
                            Write::Resize { id, size } => host.handle("session.resize", json!({ "id": id, "cols": size.cols, "rows": size.rows })),
                            Write::Close(id) => host.handle("session.close", json!({ "id": id })),
                        };
                    }
                })
                .expect("input thread");
        }

        // Replies the emulators owe their programs, and what they announce.
        {
            let writes = writes.clone();
            let notify = notify.clone();
            let wake = wake.clone();
            thread::Builder::new()
                .name("omniterm-requests".into())
                .spawn(move || {
                    for request in request_queue {
                        let notice = match request {
                            Request::Reply { id, text } => {
                                let _ = writes.send(Write::Input { id, data: text, reply: true });
                                continue;
                            }
                            Request::Title { id, title } => Notice::Title { id, title },
                            Request::Directory { id, url } => Notice::Directory { id, url },
                            Request::Bell { .. } => continue,
                        };
                        let _ = notify.send(notice);
                        wake();
                    }
                })
                .expect("request thread");
        }

        // Planning may wait on a process (the pyserial check), so sessions open here.
        {
            let host = host.clone();
            let terminals = terminals.clone();
            let requests = requests.clone();
            thread::Builder::new()
                .name("omniterm-open".into())
                .spawn(move || {
                    for open in open_queue {
                        let result = open_session(&host, &terminals, &requests, &open, &control_path);
                        let _ = notify.send(Notice::Opened { token: open.token, result: result.map(Box::new) });
                        wake();
                    }
                })
                .expect("open thread");
        }

        Self { host, terminals, writes, opens, requests, control_path, notices }
    }

    /// Opens a session right away. Only for local shells at launch, whose plans never wait.
    pub fn open_blocking(&self, open: Open) -> Result<Session, String> {
        open_session(&self.host, &self.terminals, &self.requests, &open, &self.control_path)
    }

    pub fn host(&self) -> &Arc<Host> {
        &self.host
    }

    /// Opens a session in the background; the result arrives as `Notice::Opened`.
    pub fn open(&self, open: Open) {
        let _ = self.opens.send(open);
    }

    pub fn write(&self, id: &str, data: String) {
        if !data.is_empty() {
            let _ = self.writes.send(Write::Input { id: id.to_string(), data, reply: false });
        }
    }

    /// Bytes the terminal itself generated (mouse and focus reports): never broadcast, and
    /// marked as replies so the broker does not count them as typing.
    pub fn write_reply(&self, id: &str, data: String) {
        if !data.is_empty() {
            let _ = self.writes.send(Write::Input { id: id.to_string(), data, reply: true });
        }
    }

    pub fn resize(&self, terminal: &Terminal, size: Size) {
        let size = Size { cols: size.cols.clamp(2, 1000), rows: size.rows.clamp(1, 500) };
        if terminal.resize(size) {
            let _ = self.writes.send(Write::Resize { id: terminal.id.clone(), size });
        }
    }

    pub fn close(&self, id: &str) {
        lock(&self.terminals).remove(id);
        let _ = self.writes.send(Write::Close(id.to_string()));
    }

    /// A host request that is not about a session's bytes: workspace, operations, context.
    pub fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        self.host.handle(method, params)
    }

    pub fn shutdown(&self) {
        self.host.shutdown();
    }
}

fn open_session(
    host: &Host,
    terminals: &Mutex<HashMap<String, Arc<Terminal>>>,
    requests: &Sender<Request>,
    open: &Open,
    control_path: &dyn Fn() -> Option<String>,
) -> Result<Session, String> {
    let request = plan::Request { kind: open.kind, cwd: &open.cwd, profile: open.profile.as_ref(), companion: open.companion, local_shell: &open.local_shell };
    let plan = plan::build(&request, control_path)?;
    let created = host.handle("session.create", plan.host)?;
    let id = created["id"].as_str().ok_or("终端主机没有返回会话 id。")?.to_string();
    let nonce = created["shellNonce"].as_str().unwrap_or_default().to_string();
    let terminal = Terminal::new(id.clone(), open.size, open.options, requests.clone());
    lock(terminals).insert(id.clone(), terminal.clone());
    let started = host
        .handle("session.resize", json!({ "id": id, "cols": open.size.cols, "rows": open.size.rows }))
        .and_then(|_| host.handle("session.start", json!({ "id": id })));
    if let Err(error) = started {
        lock(terminals).remove(&id);
        let _ = host.handle("session.close", json!({ "id": id }));
        return Err(error);
    }
    Ok(Session { terminal, kind: open.kind, title: plan.title, cwd: open.cwd.clone(), powershell: plan.powershell, profile: open.profile.clone(), companion: plan.companion, nonce, warning: plan.warning })
}

// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! The terminal host, linked in. It is the same `Host` the desktop app runs as a child process,
//! driven through the same requests, so sessions, the screen model and the Agent broker behave
//! the same here. Output arrives as frames on one thread and goes straight into the emulators.

use crate::terminal::{Request, Size, Terminal};
use omniterm_terminal_host::host::Host;
use omniterm_terminal_host::protocol::{Frame, Outbox};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;

/// What the UI hears about, apart from output.
#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    Exited { id: String, code: i64 },
    Title { id: String, title: Option<String> },
}

enum Write {
    Input { id: String, data: String, reply: bool },
    Resize { id: String, size: Size },
    Close(String),
}

pub struct Backend {
    host: Arc<Host>,
    terminals: Arc<Mutex<HashMap<String, Arc<Terminal>>>>,
    requests: Sender<Request>,
    writes: Sender<Write>,
    pub notices: Receiver<Notice>,
}

/// How a local shell starts: the user's login shell, or PowerShell on Windows.
pub fn local_shell_plan(cwd: &str) -> Value {
    let (executable, args): (String, Vec<String>) = if cfg!(windows) {
        let pwsh = std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).any(|dir| dir.join("pwsh.exe").is_file()))
            .unwrap_or(false);
        (if pwsh { "pwsh.exe" } else { "powershell.exe" }.to_string(), vec!["-NoLogo".to_string()])
    } else {
        let shell = std::env::var("SHELL").ok().filter(|shell| std::path::Path::new(shell).is_file());
        (shell.unwrap_or_else(|| "/bin/sh".to_string()), vec!["-l".to_string()])
    };
    let mut env = BTreeMap::new();
    env.insert("TERM", "xterm-256color");
    env.insert("COLORTERM", "truecolor");
    env.insert("TERM_PROGRAM", "omniTerm");
    json!({ "executable": executable, "args": args, "cwd": cwd, "env": env, "kind": "shell", "title": "本地 Shell" })
}

pub fn home_dir() -> String {
    std::env::var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_else(|_| ".".to_string())
}

impl Backend {
    /// `wake` is called whenever something on screen may have changed.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        let wake = Arc::new(wake);
        let (outbox, frames) = Outbox::channel();
        let host = Host::new(outbox);
        let terminals: Arc<Mutex<HashMap<String, Arc<Terminal>>>> = Arc::default();
        let (notify, notices) = channel();
        let (requests, request_queue) = channel::<Request>();
        let (writes, write_queue) = channel::<Write>();

        let routes = terminals.clone();
        let exits = notify.clone();
        let woken = wake.clone();
        thread::Builder::new().name("omniterm-output".into()).spawn(move || {
            for frame in frames {
                match frame {
                    Frame::Data { session_id, bytes, .. } => {
                        let terminal = routes.lock().unwrap_or_else(|p| p.into_inner()).get(&session_id).cloned();
                        if let Some(terminal) = terminal {
                            terminal.feed(&bytes);
                        }
                    }
                    Frame::Json(value) if value["event"] == "session.exit" => {
                        let id = value["params"]["id"].as_str().unwrap_or_default().to_string();
                        if let Some(terminal) = routes.lock().unwrap_or_else(|p| p.into_inner()).get(&id) {
                            terminal.exited.store(true, std::sync::atomic::Ordering::Release);
                            terminal.touch();
                        }
                        let _ = exits.send(Notice::Exited { id, code: value["params"]["exitCode"].as_i64().unwrap_or(1) });
                    }
                    Frame::Json(_) => {}
                }
                woken();
            }
        }).expect("output thread");

        // Every write goes through one thread, in order, so the UI never waits on a PTY.
        let writer_host = host.clone();
        thread::Builder::new().name("omniterm-input".into()).spawn(move || {
            for write in write_queue {
                let _ = match write {
                    Write::Input { id, data, reply } => writer_host.handle("session.write", json!({ "id": id, "data": data, "reply": reply })),
                    Write::Resize { id, size } => writer_host.handle("session.resize", json!({ "id": id, "cols": size.cols, "rows": size.rows })),
                    Write::Close(id) => writer_host.handle("session.close", json!({ "id": id })),
                };
            }
        }).expect("input thread");

        // Replies the emulators owe their programs, and titles they announce.
        let replies = writes.clone();
        thread::Builder::new().name("omniterm-requests".into()).spawn(move || {
            for request in request_queue {
                match request {
                    Request::Reply { id, text } => { let _ = replies.send(Write::Input { id, data: text, reply: true }); }
                    Request::Title { id, title } => { let _ = notify.send(Notice::Title { id, title }); wake(); }
                }
            }
        }).expect("request thread");

        Self { host, terminals, requests, writes, notices }
    }

    /// Opens a session and starts it at `size`, so the first prompt is drawn at the right width.
    pub fn open(&self, plan: Value, size: Size) -> Result<Arc<Terminal>, String> {
        let created = self.host.handle("session.create", plan)?;
        let id = created["id"].as_str().ok_or("The terminal host returned no session id.")?.to_string();
        let terminal = Terminal::new(id.clone(), size, self.requests.clone());
        self.terminals.lock().unwrap_or_else(|p| p.into_inner()).insert(id.clone(), terminal.clone());
        self.host.handle("session.resize", json!({ "id": id, "cols": size.cols, "rows": size.rows }))?;
        if let Err(error) = self.host.handle("session.start", json!({ "id": id })) {
            self.terminals.lock().unwrap_or_else(|p| p.into_inner()).remove(&id);
            let _ = self.host.handle("session.close", json!({ "id": id }));
            return Err(error);
        }
        Ok(terminal)
    }

    pub fn write(&self, id: &str, data: String) {
        if !data.is_empty() {
            let _ = self.writes.send(Write::Input { id: id.to_string(), data, reply: false });
        }
    }

    pub fn resize(&self, terminal: &Terminal, size: Size) {
        if size.cols >= 2 && size.rows >= 1 && terminal.resize(size) {
            let _ = self.writes.send(Write::Resize { id: terminal.id.clone(), size: Size { cols: size.cols.min(1000), rows: size.rows.min(500) } });
        }
    }

    pub fn close(&self, id: &str) {
        self.terminals.lock().unwrap_or_else(|p| p.into_inner()).remove(id);
        let _ = self.writes.send(Write::Close(id.to_string()));
    }

    pub fn shutdown(&self) {
        self.host.shutdown();
    }
}

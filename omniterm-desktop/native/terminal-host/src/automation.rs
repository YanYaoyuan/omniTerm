//! Authenticated, loopback-only external-agent bridge. One JSON line per connection.
//! This is a local control surface: possession of the private discovery file grants access.
use crate::host::Host;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MAX_REQUEST: u64 = 1024 * 1024;
const MAX_CALLS: usize = 4096;

struct Call {
    payload: Value,
    operation: String,
    task: String,
}
struct Service {
    host: Arc<Host>,
    calls: Mutex<HashMap<String, Call>>,
}

impl Service {
    fn result(&self, operation: &str, task: &str) -> Result<Value, String> {
        let result = self.host.handle(
            "operation.result",
            json!({"operationId": operation, "taskId": task}),
        )?;
        if !matches!(
            result["operation"]["status"].as_str(),
            Some("pendingApproval" | "approved" | "queued" | "running")
        ) {
            let _ = self.host.handle("task.end", json!({"taskId": task}));
        }
        Ok(result)
    }
    fn terminal(&self, params: &Value) -> Result<Value, String> {
        let id = params["terminal"].as_str().ok_or("terminal is required")?;
        let list = self.host.handle("workspace.list", json!({}))?;
        let matches: Vec<_> = list["terminals"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|t| {
                let full = t["ref"]["sessionId"].as_str().unwrap_or_default();
                full == id || (id.len() >= 8 && full.starts_with(id))
            })
            .cloned()
            .collect();
        if matches.len() != 1 {
            return Err(
                "Terminal ID must identify exactly one session (minimum 8 characters).".into(),
            );
        }
        let terminal = matches[0].clone();
        let epoch = params["epoch"]
            .as_u64()
            .ok_or("epoch is required; use session list first")?;
        if terminal["ref"]["epoch"] != epoch {
            return Err("Connection epoch changed; list sessions again. Nothing was sent.".into());
        }
        Ok(terminal)
    }

    fn handle(&self, method: &str, params: Value) -> Result<Value, String> {
        match method {
            "ping" => Ok(json!({"protocol": 1, "pid": std::process::id()})),
            "list" => self.host.handle("workspace.list", json!({})),
            "read" | "setup" | "write" => {
                let terminal = self.terminal(&params)?;
                let id = terminal["ref"]["sessionId"]
                    .as_str()
                    .ok_or("No session ID")?;
                if method == "write" {
                    let mut text = params["text"]
                        .as_str()
                        .ok_or("text is required")?
                        .to_string();
                    if params["enter"].as_bool() == Some(true) {
                        text.push('\r');
                    }
                    self.host
                        .handle("session.write", json!({"id": id, "data": text}))?;
                    return Ok(
                        json!({"status": "sent", "ref": terminal["ref"], "completionKnown": false}),
                    );
                }
                if method == "setup" {
                    if params["shell"].as_str() != Some("bash")
                        && params["shell"].as_str() != Some("zsh")
                    {
                        return Err(
                            "setup requires --shell bash or --shell zsh at an empty shell prompt"
                                .into(),
                        );
                    }
                    if !matches!(terminal["kind"].as_str(), Some("ssh" | "shell")) {
                        return Err("Only shell and SSH sessions support setup.".into());
                    }
                    let script = self.host.handle("session.integration", json!({"id": id}))?;
                    self.host.handle("session.write", json!({"id": id, "data": format!("{}\r", script.as_str().ok_or("No integration script")?)}))?;
                    if terminal["kind"] == "ssh" {
                        self.host.handle(
                            "session.allowAutoSubmit",
                            json!({"id": id, "enabled": true}),
                        )?;
                    }
                    return Ok(
                        json!({"status": "sent", "ref": terminal["ref"], "note": "Poll session list until interaction.state is ready and autoSubmit is true."}),
                    );
                }
                let task = format!("external-read:{}", uuid::Uuid::new_v4());
                self.host.handle(
                    "task.begin",
                    json!({"taskId": task, "context": [terminal["ref"]]}),
                )?;
                let result = self.host.handle("context.read", json!({"taskId": task, "terminal": id, "sinceSeq": params["sinceSeq"], "maxLines": params["maxLines"]}));
                let _ = self.host.handle("task.end", json!({"taskId": task}));
                result
            }
            "exec" => {
                let request = params["requestId"]
                    .as_str()
                    .filter(|s| !s.is_empty() && s.len() <= 128)
                    .ok_or("requestId (1–128 bytes) is required")?;
                let mut calls = self.calls.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(call) = calls.get(request) {
                    if call.payload != params {
                        return Err("requestId was already used with different parameters".into());
                    }
                    return self.result(&call.operation, &call.task);
                }
                if calls.len() >= MAX_CALLS {
                    return Err("External operation limit reached; restart omniTerm before starting more operations.".into());
                }
                let terminal = self.terminal(&params)?;
                if terminal["interaction"]["state"] != "ready" || terminal["autoSubmit"] != true {
                    return Err("Terminal is not ready for tracked execution. At an empty bash 4.4+ / zsh prompt run session setup, then list again. Use session write only for explicit untracked input.".into());
                }
                let task = format!("external-exec:{}", uuid::Uuid::new_v4());
                self.host.handle("task.begin", json!({"taskId": task, "context": [terminal["ref"]], "targets": [terminal["ref"]]}))?;
                let operation = match self.host.handle("operation.propose", json!({"taskId": task, "callId": request, "terminal": terminal["ref"]["sessionId"], "command": params["command"], "timeoutMs": params["timeoutMs"]})) {
                    Ok(value) => value,
                    Err(error) => { let _ = self.host.handle("task.end", json!({"taskId": task})); return Err(error); }
                };
                let op = operation["operationId"]
                    .as_str()
                    .ok_or("Missing operation ID")?
                    .to_string();
                // Record before authorization/dispatch: an uncertain response must never replay a write.
                calls.insert(
                    request.to_string(),
                    Call {
                        payload: params.clone(),
                        operation: op.clone(),
                        task: task.clone(),
                    },
                );
                let approved = self.host.handle(
                    "operation.approve",
                    json!({"operationId": op, "scope": "once", "mode": "run"}),
                );
                if approved.is_ok() {
                    self.host.handle(
                        "operation.execute",
                        json!({"operationId": op, "taskId": task}),
                    )?;
                } else {
                    let _ = self
                        .host
                        .handle("operation.deny", json!({"operationId": op}));
                }
                self.result(&op, &task)
            }
            "result" | "cancel" => {
                let calls = self.calls.lock().unwrap_or_else(|p| p.into_inner());
                let call = calls.get(params["requestId"].as_str().ok_or("requestId is required")?).ok_or("Unknown requestId in this omniTerm instance; do not rerun an uncertain command")?;
                if method == "cancel" {
                    self.host.handle(
                        "operation.cancel",
                        json!({"operationId": call.operation, "taskId": call.task}),
                    )?;
                }
                self.result(&call.operation, &call.task)
            }
            _ => Err(format!("Unknown external method: {method}")),
        }
    }
}

pub struct Automation {
    stopped: Arc<AtomicBool>,
    descriptor: PathBuf,
}
impl Automation {
    pub fn start(host: Arc<Host>, directory: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
        }
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let instance = uuid::Uuid::new_v4().to_string();
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let descriptor = directory.join(format!("{instance}.json"));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&descriptor)?;
        serde_json::to_writer(
            &mut file,
            &json!({"protocol": 1, "instance": instance, "pid": std::process::id(), "port": listener.local_addr()?.port(), "token": token}),
        )?;
        file.sync_all()?;
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let service = Arc::new(Service {
            host,
            calls: Mutex::default(),
        });
        let workers = Arc::new(AtomicUsize::new(0));
        std::thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if workers.load(Ordering::Acquire) >= 16 {
                            continue;
                        }
                        workers.fetch_add(1, Ordering::AcqRel);
                        let (service, token, workers, stop) = (
                            service.clone(),
                            token.clone(),
                            workers.clone(),
                            stop.clone(),
                        );
                        std::thread::spawn(move || {
                            let _ = serve(stream, &token, &service, &stop);
                            workers.fetch_sub(1, Ordering::AcqRel);
                        });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(20))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            stopped,
            descriptor,
        })
    }
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        let _ = std::fs::remove_file(&self.descriptor);
    }
}
impl Drop for Automation {
    fn drop(&mut self) {
        self.stop();
    }
}

fn serve(
    mut stream: TcpStream,
    token: &str,
    service: &Service,
    stopped: &AtomicBool,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut line = Vec::new();
    BufReader::new((&stream).take(MAX_REQUEST + 1)).read_until(b'\n', &mut line)?;
    let response = (|| -> Result<Value, String> {
        if stopped.load(Ordering::Acquire) {
            return Err("omniTerm is shutting down".into());
        }
        if line.len() as u64 > MAX_REQUEST || line.last() != Some(&b'\n') {
            return Err("Expected one JSON line of at most 1 MiB".into());
        }
        let request: Value = serde_json::from_slice(&line).map_err(|_| "Invalid JSON request")?;
        if request["token"].as_str() != Some(token) {
            return Err("Authentication failed".into());
        }
        service.handle(
            request["method"].as_str().ok_or("method is required")?,
            request["params"].clone(),
        )
    })();
    let result = match response {
        Ok(value) => json!({"ok": true, "result": value}),
        Err(error) => json!({"ok": false, "error": error}),
    };
    serde_json::to_writer(&mut stream, &result)?;
    stream.write_all(b"\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Outbox;
    #[cfg(unix)]
    use std::time::Instant;

    struct Fixture {
        host: Arc<Host>,
        bridge: Automation,
        descriptor: Value,
        directory: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let (outbox, frames) = Outbox::channel();
            std::thread::spawn(move || for _ in frames {});
            let host = Host::new(outbox);
            let directory =
                std::env::temp_dir().join(format!("omniterm-bridge-test-{}", uuid::Uuid::new_v4()));
            let bridge = Automation::start(host.clone(), &directory).expect("bridge");
            let descriptor =
                serde_json::from_slice(&std::fs::read(&bridge.descriptor).expect("descriptor"))
                    .expect("json");
            Self {
                host,
                bridge,
                descriptor,
                directory,
            }
        }
        fn request(&self, token: &str, method: &str, params: Value) -> Value {
            let mut stream = TcpStream::connect((
                "127.0.0.1",
                self.descriptor["port"].as_u64().unwrap() as u16,
            ))
            .expect("connect");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            writeln!(
                stream,
                "{}",
                json!({"token": token, "method": method, "params": params})
            )
            .unwrap();
            let mut line = String::new();
            BufReader::new(stream).read_line(&mut line).unwrap();
            serde_json::from_str(&line).unwrap()
        }
        fn call(&self, method: &str, params: Value) -> Value {
            let response = self.request(self.descriptor["token"].as_str().unwrap(), method, params);
            assert_eq!(response["ok"], true, "{response}");
            response["result"].clone()
        }
        #[cfg(unix)]
        fn shell(&self, kind: &str) -> Value {
            // macOS ships bash 3.2; its system zsh supports the readiness integration.
            let (executable, args) = if cfg!(target_os = "macos") {
                ("/bin/zsh", vec!["-f", "-i"])
            } else {
                ("/bin/bash", vec!["--noprofile", "--norc", "-i"])
            };
            let created = self.host.handle("session.create", json!({"executable": executable, "args": args, "cwd": "/tmp", "kind": kind, "title": "fixture", "env": {"HISTFILE": "/dev/null", "PS1": "fixture> ", "EDITOR": "emacs"}})).unwrap();
            let started = self
                .host
                .handle("session.start", json!({"id": created["id"]}))
                .unwrap();
            std::thread::sleep(Duration::from_millis(150));
            json!({"terminal": started["id"], "epoch": started["epoch"]})
        }
        #[cfg(unix)]
        fn ready(&self, terminal: &Value) {
            let shell = if cfg!(target_os = "macos") {
                "zsh"
            } else {
                "bash"
            };
            self.call("setup", json!({"terminal": terminal["terminal"], "epoch": terminal["epoch"], "shell": shell}));
            let deadline = Instant::now() + Duration::from_secs(6);
            loop {
                let list = self.call("list", json!({}));
                let t = list["terminals"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|t| t["ref"]["sessionId"] == terminal["terminal"])
                    .unwrap();
                if t["interaction"]["state"] == "ready" && t["autoSubmit"] == true {
                    break;
                }
                assert!(Instant::now() < deadline, "{list}");
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        #[cfg(unix)]
        fn completed(&self, request: &str) -> Value {
            let deadline = Instant::now() + Duration::from_secs(8);
            loop {
                let result = self.call("result", json!({"requestId": request}));
                if result["operation"]["status"] == "completed" {
                    return result;
                }
                assert!(Instant::now() < deadline, "{result}");
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.bridge.stop();
            self.host.shutdown();
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn authentication_allowlist_and_private_discovery() {
        let fixture = Fixture::new();
        assert_eq!(fixture.request("wrong", "list", json!({}))["ok"], false);
        assert_eq!(
            fixture.request(
                fixture.descriptor["token"].as_str().unwrap(),
                "session.create",
                json!({})
            )["ok"],
            false
        );
        assert_eq!(fixture.call("ping", json!({}))["protocol"], 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&fixture.bridge.descriptor)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(&fixture.directory)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        fixture.bridge.stop();
        assert!(!fixture.bridge.descriptor.exists());
    }

    #[cfg(unix)]
    #[test]
    fn tracked_ssh_handshake_output_exit_code_and_idempotency() {
        let fixture = Fixture::new();
        // A real bash PTY marked SSH exercises the remote line-editor handshake.
        let terminal = fixture.shell("ssh");
        fixture.ready(&terminal);
        let params = json!({"terminal": terminal["terminal"], "epoch": terminal["epoch"], "requestId": "once", "command": "printf 'BRIDGE_RESULT_42\\n'; false", "timeoutMs": 5000});
        let first = fixture.call("exec", params.clone());
        let result = fixture.completed("once");
        assert_eq!(result["operation"]["exitCode"], 1);
        assert!(
            result["output"]["lines"]
                .to_string()
                .contains("BRIDGE_RESULT_42"),
            "{result}"
        );
        let again = fixture.call("exec", params.clone());
        assert_eq!(
            again["operation"]["operationId"],
            first["operation"]["operationId"]
        );
        assert_eq!(again["operation"]["status"], "completed");
        let mut changed = params;
        changed["command"] = json!("echo different");
        assert_eq!(
            fixture.request(
                fixture.descriptor["token"].as_str().unwrap(),
                "exec",
                changed
            )["ok"],
            false
        );
        let read = fixture.call("read", terminal.clone());
        let mut delta = terminal;
        delta["sinceSeq"] = read["seqEnd"].clone();
        let read = fixture.call("read", delta);
        assert!(read["lines"].as_array().unwrap().is_empty(), "{read}");
    }

    #[cfg(unix)]
    #[test]
    fn stale_epoch_and_unprepared_shell_never_execute() {
        let fixture = Fixture::new();
        let terminal = fixture.shell("ssh");
        let mut params = terminal.clone();
        params["command"] = json!("echo SHOULD_NOT_RUN");
        params["requestId"] = json!("unready");
        assert_eq!(
            fixture.request(
                fixture.descriptor["token"].as_str().unwrap(),
                "exec",
                params
            )["ok"],
            false
        );
        let mut stale = terminal.clone();
        stale["epoch"] = json!(999);
        stale["text"] = json!("echo SHOULD_NOT_RUN");
        stale["enter"] = json!(true);
        assert_eq!(
            fixture.request(
                fixture.descriptor["token"].as_str().unwrap(),
                "write",
                stale
            )["ok"],
            false
        );
        let read = fixture.call("read", terminal);
        assert!(!read["lines"].to_string().contains("SHOULD_NOT_RUN"));
    }

    #[cfg(unix)]
    #[test]
    fn explicit_input_is_untracked_and_does_not_touch_other_sessions() {
        let fixture = Fixture::new();
        let first = fixture.shell("shell");
        let second = fixture.shell("shell");
        let mut params = first.clone();
        params["text"] = json!("printf 'ONLY_FIRST_SESSION\\n'");
        params["enter"] = json!(true);
        let sent = fixture.call("write", params);
        assert_eq!(sent["completionKnown"], false);
        std::thread::sleep(Duration::from_millis(150));
        assert!(
            fixture.call("read", first)["lines"]
                .to_string()
                .contains("ONLY_FIRST_SESSION")
        );
        assert!(
            !fixture.call("read", second)["lines"]
                .to_string()
                .contains("ONLY_FIRST_SESSION")
        );
    }
}

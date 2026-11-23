// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! External operations on the same SFTP clients, directory view and transfer queue as the UI.
use crate::remote::manager::{
    ExternalTransfer, FileEntry, Listing, Manager, Phase, Remote, Transfer, TransferStatus,
};
use crate::remote::sftp::{Attributes, Client, Error};
use omniterm_terminal_host::host::Host;
use omniterm_terminal_host::redact::redact;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const MAX_CALLS: usize = 4096;
struct Job {
    payload: Value,
    remote: Arc<Remote>,
    client: Arc<Client>,
    transfer: Option<u64>,
    state: Mutex<Value>,
}
pub struct Files {
    host: Arc<Host>,
    manager: Arc<Manager>,
    jobs: Mutex<HashMap<String, Arc<Job>>>,
}
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
fn text<'a>(p: &'a Value, key: &str) -> Result<&'a str, String> {
    p[key]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 8192 && !s.contains('\0'))
        .ok_or_else(|| format!("{key} must be a nonempty string without NUL"))
}
fn remote_path(p: &Value, key: &str) -> Result<String, String> {
    let value = text(p, key)?;
    if !value.starts_with('/') {
        return Err(format!(
            "{key} must be an absolute remote path; inspect files info first"
        ));
    }
    Ok(value.to_string())
}
fn local_path(p: &Value) -> Result<PathBuf, String> {
    let path = PathBuf::from(text(p, "local")?);
    if !path.is_absolute() {
        return Err("local must be an absolute path on the omniTerm machine".into());
    }
    Ok(path)
}
fn attributes(a: Attributes) -> Value {
    json!({"size": a.size, "mode": a.mode, "mtime": a.mtime, "directory": a.directory(), "file": a.file(), "symlink": a.symlink()})
}
fn entry(e: &FileEntry) -> Value {
    json!({"name":e.name,"directory":e.directory,"symlink":e.symlink,"size":e.size,"mtime":e.mtime,"mode":e.mode})
}
fn listing(listing: &Listing, p: &Value) -> Value {
    let offset = p["offset"].as_u64().unwrap_or(0).min(100_000) as usize;
    let limit = p["limit"].as_u64().unwrap_or(200).clamp(1, 2000) as usize;
    let mut entries = listing.entries.clone();
    entries.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| a.name.cmp(&b.name))
    });
    let page: Vec<_> = entries.iter().skip(offset).take(limit).map(entry).collect();
    let next = offset.saturating_add(page.len());
    json!({"path":listing.path,"loading":listing.loading,"error":listing.error.as_deref().map(redact),"entries":page,"totalEntries":entries.len(),"offset":offset,"truncated":next < entries.len(),"nextOffset":if next < entries.len(){Some(next)}else{None}})
}
fn transfer(t: &Transfer, disconnected: bool) -> Value {
    let status = match t.status {
        TransferStatus::Queued => "queued",
        TransferStatus::Running => "running",
        TransferStatus::Done => "completed",
        TransferStatus::Canceled => "cancelled",
        TransferStatus::Failed if disconnected => "outcomeUnknown",
        TransferStatus::Failed => "failed",
    };
    json!({"status":status,"transferId":t.id,"direction":if t.upload{"upload"}else{"download"},"name":t.name,"bytes":t.bytes,"total":t.total,"skipped":t.skipped,"error":if t.error.is_empty(){None}else{Some(redact(&t.error))},"completionKnown":matches!(t.status, TransferStatus::Done | TransferStatus::Canceled)})
}
impl Files {
    pub fn new(host: Arc<Host>, manager: Arc<Manager>) -> Arc<Self> {
        Arc::new(Self {
            host,
            manager,
            jobs: Mutex::default(),
        })
    }
    fn describe(&self, remote: &Remote) -> Value {
        let state = remote.snapshot();
        let (phase, error) = match &state.phase {
            Phase::Waiting => ("waiting", None),
            Phase::Connecting => ("connecting", None),
            Phase::Ready => ("ready", None),
            Phase::Unavailable { reason, .. } => ("unavailable", Some(reason.message())),
        };
        let inventory = self
            .host
            .handle("workspace.list", json!({}))
            .unwrap_or_default();
        let terminal = inventory["terminals"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|t| t["ref"]["sessionId"] == remote.id());
        let endpoint = remote.public_endpoint();
        json!({"ref":{"fileSessionId":remote.id(),"generation":remote.generation()},"label":terminal.map(|t|t["label"].clone()).unwrap_or_else(||endpoint["title"].clone()),"terminalRef":terminal.map(|t|t["ref"].clone()),"endpoint":endpoint,"phase":phase,"error":error,"home":state.home,"path":state.listing.path,"loading":state.listing.loading,"independent":remote.is_independent()})
    }
    fn resolve(&self, p: &Value) -> Result<Arc<Remote>, String> {
        let id = text(p, "fileSession")?;
        let matches: Vec<_> = self
            .manager
            .all()
            .into_iter()
            .filter(|r| r.id() == id || (id.len() >= 8 && r.id().starts_with(id)))
            .collect();
        if matches.len() != 1 {
            return Err("File session ID must identify exactly one existing SFTP page".into());
        }
        let remote = matches[0].clone();
        if remote.generation() != text(p, "generation")? {
            return Err(
                "SFTP connection changed; list file sessions and bind again. Nothing was sent."
                    .into(),
            );
        }
        Ok(remote)
    }
    fn result(&self, request: &str, cancel: bool) -> Result<Value, String> {
        let job = lock(&self.jobs).get(request).cloned().ok_or(
            "Unknown file requestId in this instance; do not blindly repeat a file operation",
        )?;
        if let Some(id) = job.transfer {
            if cancel {
                job.remote.cancel(id);
            }
            if let Some(t) = job.remote.tracked_transfer(id) {
                *lock(&job.state) = transfer(&t, job.client.closed());
            }
        } else if cancel && lock(&job.state)["status"] == "running" {
            return Err("This SFTP metadata operation has already been dispatched and cannot be interrupted safely. Query its result.".into());
        }
        Ok(
            json!({"requestId":request,"ref":{"fileSessionId":job.remote.id(),"generation":job.payload["generation"]},"operation":lock(&job.state).clone()}),
        )
    }
    pub fn handle(self: &Arc<Self>, method: &str, p: Value) -> Result<Value, String> {
        let action = method.strip_prefix("files.").ok_or("Invalid file method")?;
        if action == "list" {
            let mut sessions: Vec<_> = self
                .manager
                .all()
                .iter()
                .map(|r| self.describe(r))
                .collect();
            sessions.sort_by(|a, b| {
                a["ref"]["fileSessionId"]
                    .as_str()
                    .cmp(&b["ref"]["fileSessionId"].as_str())
            });
            return Ok(json!({"fileSessions":sessions}));
        }
        if action == "result" || action == "cancel" {
            return self.result(text(&p, "requestId")?, action == "cancel");
        }
        if matches!(action, "info" | "transfers" | "cancel-transfer") {
            let remote = self.resolve(&p)?;
            if action == "cancel-transfer" {
                let id = p["transferId"].as_u64().ok_or("transferId is required")?;
                if remote.tracked_transfer(id).is_none() {
                    return Err("Unknown transfer ID in this file session".into());
                }
                remote.cancel(id);
                let item = remote
                    .tracked_transfer(id)
                    .ok_or("Transfer no longer available")?;
                return Ok(
                    json!({"ref":{"fileSessionId":remote.id(),"generation":remote.generation()},"operation":transfer(&item, remote.snapshot().phase != Phase::Ready)}),
                );
            }
            let state = remote.snapshot();
            let items: Vec<_> = state
                .transfers
                .iter()
                .rev()
                .take(200)
                .map(|t| transfer(t, state.phase != Phase::Ready))
                .collect();
            let mut result = self.describe(&remote);
            result["transfers"] = json!(items);
            if action == "info" {
                result["listing"] = listing(&state.listing, &p);
            }
            return Ok(result);
        }
        if !matches!(
            action,
            "ls" | "stat" | "mkdir" | "rename" | "remove" | "upload" | "download"
        ) {
            return Err(format!("Unknown file action: {action}"));
        }
        let mut p = p;
        p["action"] = json!(action);
        let request = text(&p, "requestId")?.to_string();
        if request.len() > 128 {
            return Err("requestId must be at most 128 bytes".into());
        }
        let mut jobs = lock(&self.jobs);
        if let Some(previous) = jobs.get(&request) {
            if previous.payload != p {
                return Err("File requestId already used with different parameters".into());
            }
            drop(jobs);
            return self.result(&request, false);
        }
        if jobs.len() >= MAX_CALLS {
            return Err("File operation limit reached; finish work and restart omniTerm".into());
        }
        let remote = self.resolve(&p)?;
        let client = remote.checked_client(text(&p, "generation")?)?;
        let path = if matches!(action, "upload" | "download") {
            remote_path(&p, "remote")?
        } else {
            remote_path(&p, "path")?
        };
        let concrete = |value: &str| {
            !value.trim_matches('/').is_empty() && !value.split('/').any(|s| s == "." || s == "..")
        };
        if action == "rename" && !concrete(&remote_path(&p, "to")?) {
            return Err("Use an explicit non-root destination for rename".into());
        }
        if matches!(action, "remove" | "rename") && !concrete(&path) {
            return Err("Use an explicit non-root path for remove/rename".into());
        }
        let navigation = if action == "ls" {
            Some(remote.reserve_browse(&client)?)
        } else {
            None
        };
        let transfer_id = if matches!(action, "upload" | "download") {
            Some(remote.start_external_transfer(
                client.clone(),
                ExternalTransfer {
                    upload: action == "upload",
                    local: local_path(&p)?,
                    remote: path.clone(),
                    recursive: p["recursive"].as_bool() == Some(true),
                    overwrite: p["overwrite"].as_bool() == Some(true),
                },
            ))
        } else {
            None
        };
        let job = Arc::new(Job {
            payload: p.clone(),
            remote: remote.clone(),
            client: client.clone(),
            transfer: transfer_id,
            state: Mutex::new(json!({"status":"running","completionKnown":false})),
        });
        jobs.insert(request.clone(), job.clone());
        drop(jobs);
        if transfer_id.is_none() {
            let action = action.to_string();
            std::thread::spawn(move || {
                let result: Result<Value, Error> = (|| match action.as_str() {
                    "ls" => remote
                        .browse_at(
                            &client,
                            &path,
                            navigation.as_ref().expect("reserved navigation").0,
                            &navigation.as_ref().expect("reserved navigation").1,
                        )
                        .map(|l| listing(&l, &p))
                        .map_err(Error::Protocol),
                    "stat" => client.lstat(&path).map(attributes),
                    "mkdir" => client.mkdir(&path).map(|_| json!({"path":path})),
                    "remove" => (if p["directory"].as_bool() == Some(true) {
                        client.rmdir(&path)
                    } else {
                        client.remove(&path)
                    })
                    .map(|_| json!({"path":path})),
                    "rename" => {
                        let to = remote_path(&p, "to").map_err(Error::Protocol)?;
                        if p["overwrite"].as_bool() != Some(true) && client.exists(&to)? {
                            return Err(Error::Status(
                                4,
                                "Destination exists; use --overwrite only when intended".into(),
                            ));
                        }
                        (if p["overwrite"].as_bool() == Some(true) {
                            client.replace(&path, &to)
                        } else {
                            client.rename(&path, &to)
                        })
                        .map(|_| json!({"from":path,"to":to}))
                    }
                    _ => unreachable!(),
                })();
                let succeeded = result.is_ok();
                *lock(&job.state) = match result {
                    Ok(value) => {
                        json!({"status":"completed","result":value,"completionKnown":true})
                    }
                    Err(error) => {
                        let uncertain =
                            client.closed() || matches!(error, Error::Closed | Error::Protocol(_));
                        json!({"status":if uncertain{"outcomeUnknown"}else{"failed"},"error":redact(&error.to_string()),"completionKnown":!uncertain})
                    }
                };
                if succeeded
                    && matches!(action.as_str(), "mkdir" | "remove" | "rename")
                    && remote.generation() == p["generation"].as_str().unwrap_or_default()
                    && let Some(current) = remote.snapshot().listing.path
                {
                    let source = path.trim_end_matches('/');
                    let within_source =
                        current == source || current.starts_with(&format!("{source}/"));
                    let next = if action == "remove"
                        && p["directory"].as_bool() == Some(true)
                        && within_source
                    {
                        crate::remote::manager::parent(source)
                    } else if action == "rename" && within_source {
                        format!(
                            "{}{}",
                            p["to"].as_str().unwrap_or_default().trim_end_matches('/'),
                            &current[source.len()..]
                        )
                    } else {
                        current
                    };
                    remote.list(next);
                }
            });
        }
        self.result(&request, false)
    }
}

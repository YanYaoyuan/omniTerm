// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Read-only diagnostics through exactly the same transport used by the file panel.
//! No remote shell, metrics, writes, downloads, or automatic reconnects are performed.

use super::manager::{Manager, Phase};
use std::path::Path;
use std::time::{Duration, Instant};

pub fn run(path: &Path) -> i32 {
    let manager = Manager::new(|| {});
    let result = verify(&manager, path);
    manager.shutdown();
    match result {
        Ok((directory, entries)) => {
            println!("{}", serde_json::json!({"ok": true, "readOnly": true, "directory": directory, "entries": entries}));
            0
        }
        Err(reason) => {
            // Fixed diagnostic categories only: no source JSON, credentials, server
            // stderr, or untrusted error text is written into command logs.
            println!("{}", serde_json::json!({"ok": false, "readOnly": true, "reason": reason}));
            1
        }
    }
}

fn verify(manager: &Manager, path: &Path) -> Result<(String, usize), &'static str> {
    let connection = super::qizhi::load(path).map_err(|_| "invalid_connection_parameters")?;
    manager.attach_qizhi("file-connection-verification", connection).map_err(|_| "cannot_prepare_connection")?;
    let remote = manager.get("file-connection-verification").ok_or("connection_not_created")?;
    let deadline = Instant::now() + Duration::from_secs(120);
    let home = loop {
        let snapshot = remote.snapshot();
        match snapshot.phase {
            Phase::Ready => break snapshot.home.ok_or("missing_initial_directory")?,
            Phase::Unavailable { .. } => return Err("connection_unavailable"),
            _ if Instant::now() >= deadline => return Err("authentication_or_handshake_timeout"),
            _ => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    remote.list(home.clone());
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let snapshot = remote.snapshot();
        if !snapshot.listing.loading {
            if snapshot.listing.error.is_some() {
                return Err("directory_listing_failed");
            }
            if let Some(directory) = snapshot.listing.path {
                return Ok((directory, snapshot.listing.entries.len()));
            }
        }
        if matches!(snapshot.phase, Phase::Unavailable { .. }) {
            return Err("connection_closed_during_listing");
        }
        if Instant::now() >= deadline {
            return Err("directory_listing_timeout");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

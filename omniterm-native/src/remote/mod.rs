// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! The companion connection behind an SSH tab: an SFTP channel for the file panel and a
//! metrics channel for the host bar. Both share the terminal's authenticated OpenSSH login
//! through its control socket, so they never ask for credentials and passwords never reach
//! this process. Windows OpenSSH has no control sockets; there the channels open their own
//! non-interactive connection instead.

pub mod manager;
pub mod metrics;
pub mod sftp;

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// sockaddr_un is 104 bytes on macOS and ssh appends a 17-byte temporary suffix.
const MAX_CONTROL_PATH: usize = 86;

struct Sockets {
    directory: Option<PathBuf>,
    next: u64,
}

fn sockets() -> &'static Mutex<Sockets> {
    static SOCKETS: OnceLock<Mutex<Sockets>> = OnceLock::new();
    SOCKETS.get_or_init(|| Mutex::new(Sockets { directory: None, next: 1 }))
}

/// A fresh control socket path in a private directory, or `None` when ssh could not use one:
/// an unusable ControlPath is a fatal ssh error, not a soft failure.
pub fn control_path() -> Option<String> {
    if cfg!(windows) {
        return None;
    }
    let mut sockets = sockets().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if sockets.directory.is_none() {
        // macOS TMPDIR can leave too little room for the socket name; its default root is /tmp.
        let root = if cfg!(target_os = "macos") { PathBuf::from("/tmp") } else { std::env::temp_dir() };
        let directory = root.join(format!("omniterm-ssh-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]));
        // `create_dir` fails if the name exists, so nobody else's directory is ever reused.
        std::fs::create_dir(&directory).ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).is_err() {
                let _ = std::fs::remove_dir_all(&directory);
                return None;
            }
        }
        sockets.directory = Some(directory);
    }
    let candidate = sockets.directory.as_ref()?.join(sockets.next.to_string());
    sockets.next += 1;
    let text = candidate.to_string_lossy().into_owned();
    (text.len() <= MAX_CONTROL_PATH && text.chars().all(|c| c.is_ascii_alphanumeric() || "_./-".contains(c))).then_some(text)
}

/// Removes the socket directory when the app quits; live masters already hold their sockets.
pub fn cleanup() {
    if let Some(directory) = sockets().lock().unwrap_or_else(|poisoned| poisoned.into_inner()).directory.take() {
        let _ = std::fs::remove_dir_all(directory);
    }
}

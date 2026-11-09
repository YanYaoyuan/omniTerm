// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! The data directory is the one Electron's `app.getPath("userData")` uses for "omniTerm", so
//! saved connections, model providers and backgrounds are shared with the desktop app.

use std::path::{Path, PathBuf};

pub fn home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn data_dir() -> PathBuf {
    // A separate native preview/test instance must not overwrite the workspace or
    // connection settings of an already-running application. Never repurpose HOME:
    // OpenSSH still needs the user's real known_hosts and OS credential store.
    if let Some(directory) = std::env::var_os("OMNITERM_DATA_DIR").map(PathBuf::from).filter(|path| path.is_absolute()) {
        return directory;
    }
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home().join("AppData").join("Roaming"))
    } else if cfg!(target_os = "macos") {
        home().join("Library").join("Application Support")
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|path| path.is_absolute()).unwrap_or_else(|| home().join(".config"))
    };
    base.join("omniTerm")
}

pub fn file(name: &str) -> PathBuf {
    data_dir().join(name)
}

/// Writes through a temporary file and a rename, so a crash never leaves half a file.
/// The directory and file are private to the user.
pub fn write_atomically(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let directory = path.parent().ok_or_else(|| std::io::Error::other("no parent directory"))?;
    std::fs::create_dir_all(directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700));
    }
    let temporary = directory.join(format!(
        ".{}.{}.tmp",
        path.file_name().and_then(|name| name.to_str()).unwrap_or("data"),
        std::process::id()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        use std::io::Write;
        let mut file = options.open(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// A command on PATH, or the path itself when it names a file. On Windows, `.exe` and `.com`
/// are tried for names without an extension.
pub fn find_executable(name: &str) -> Option<PathBuf> {
    let usable = |candidate: &Path| {
        candidate.metadata().is_ok_and(|meta| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                meta.is_file() && meta.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                meta.is_file()
            }
        })
    };
    let extensions: &[&str] = if cfg!(windows) && Path::new(name).extension().is_none() { &[".exe", ".com"] } else { &[""] };
    if name.contains('/') || name.contains('\\') {
        return extensions.iter().map(|extension| PathBuf::from(format!("{name}{extension}"))).find(|path| usable(path));
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .flat_map(|directory| extensions.iter().map(move |extension| directory.join(format!("{name}{extension}"))))
        .find(|candidate| usable(candidate))
}

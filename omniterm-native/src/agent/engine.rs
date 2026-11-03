// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Finds the Agent engine: the copy shipped next to the app, an explicit override, a build in
//! the repository, then `codex` on PATH, in the desktop app's order.

use crate::core::paths::find_executable;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Engine {
    pub executable: String,
    pub args: Vec<String>,
}

fn binary_name() -> &'static str {
    if cfg!(windows) { "codex.exe" } else { "codex" }
}

fn usable(path: &Path) -> Option<Engine> {
    find_executable(&path.to_string_lossy()).map(|path| Engine { executable: path.to_string_lossy().into_owned(), args: Vec::new() })
}

/// Directories the app ships its resources in: beside the executable, and macOS bundles'
/// `Contents/Resources`.
fn resource_dirs() -> Vec<PathBuf> {
    let Ok(executable) = std::env::current_exe() else { return Vec::new() };
    let Some(directory) = executable.parent() else { return Vec::new() };
    let mut dirs = vec![directory.to_path_buf(), directory.join("resources")];
    if let Some(contents) = directory.parent() {
        dirs.push(contents.join("Resources"));
    }
    dirs
}

pub fn resolve() -> Result<Engine, String> {
    for directory in resource_dirs() {
        if let Some(engine) = usable(&directory.join("omniterm-agent").join("bin").join(binary_name())) {
            return Ok(engine);
        }
    }
    if let Some(value) = std::env::var_os("OMNITERM_CODEX_BIN") {
        let path = PathBuf::from(&value);
        if path.extension().is_some_and(|extension| extension == "js" || extension == "mjs" || extension == "cjs") && path.is_file() {
            let node = find_executable("node").ok_or("JavaScript 形式的 Codex 启动器需要 PATH 中有 Node。请改用原生 Agent，或把 OMNITERM_CODEX_BIN 设为原生可执行文件。")?;
            return Ok(Engine { executable: node.to_string_lossy().into_owned(), args: vec![path.to_string_lossy().into_owned()] });
        }
        return usable(&path).ok_or_else(|| format!("OMNITERM_CODEX_BIN 不可执行：{}", path.display()));
    }
    // A development checkout: <repo>/omniterm-native/target/<profile>/omniterm-app.
    if let Ok(executable) = std::env::current_exe() {
        for root in executable.ancestors().filter(|dir| dir.join("codex-rs").is_dir()) {
            for candidate in [root.join("dist").join("omniterm-agent").join("bin").join(binary_name()), root.join("codex-rs/target/debug").join(binary_name()), root.join("codex-rs/target/release").join(binary_name())] {
                if let Some(engine) = usable(&candidate) {
                    return Ok(engine);
                }
            }
        }
    }
    find_executable("codex")
        .map(|path| Engine { executable: path.to_string_lossy().into_owned(), args: Vec::new() })
        .ok_or_else(|| "找不到 Agent 引擎。请重新安装带 omniterm-agent 资源的 omniTerm，或设置 OMNITERM_CODEX_BIN。".into())
}

// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Opening sessions belong to the tab/pane that requested them. A late completion must not
//! resurrect a closed tab or leave an unowned PTY running.

use super::Tab;
use crate::backend::{Backend, Session};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Placement {
    NewTab,
    Split { tab: u64, target: String, row: bool },
}

#[derive(Default)]
pub(super) struct Pending {
    entries: HashMap<u64, Placement>,
}

impl Pending {
    pub fn insert(&mut self, token: u64, placement: Placement) {
        self.entries.insert(token, placement);
    }

    pub fn cancel_pane(&mut self, id: &str) {
        self.entries.retain(|_, placement| !matches!(placement, Placement::Split { target, .. } if target == id));
    }

    pub fn cancel_tab(&mut self, id: u64) {
        self.entries.retain(|_, placement| !matches!(placement, Placement::Split { tab, .. } if *tab == id));
    }

    fn take(&mut self, token: u64, tabs: &[Tab]) -> Option<Placement> {
        self.entries.remove(&token).filter(|placement| match placement {
            Placement::NewTab => true,
            Placement::Split { tab, target, .. } => tabs.iter().any(|item| item.id == *tab && item.root.panes().contains(target)),
        })
    }

    pub fn complete(&mut self, token: u64, result: Result<Box<Session>, String>, tabs: &[Tab], backend: &Backend) -> Result<Option<(Placement, Box<Session>)>, String> {
        let Some(placement) = self.take(token, tabs) else {
            if let Ok(session) = result {
                backend.close(&session.terminal.id);
            }
            return Ok(None);
        };
        result.map(|session| Some((placement, session)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Node;

    fn tab(id: u64, pane: &str) -> Tab {
        Tab { id, root: Node::Pane(pane.into()), focus: pane.into(), title: String::new(), broadcast: false, zoomed: false }
    }

    #[test]
    fn split_completions_require_the_original_tab_and_pane() {
        let mut pending = Pending::default();
        let split = Placement::Split { tab: 1, target: "a".into(), row: true };
        pending.insert(1, split.clone());
        assert_eq!(pending.take(1, &[tab(1, "a")]), Some(split.clone()));
        assert_eq!(pending.take(1, &[tab(1, "a")]), None, "completion is consumed once");
        for tabs in [vec![], vec![tab(1, "b")], vec![tab(2, "a")]] {
            pending.insert(2, split.clone());
            assert_eq!(pending.take(2, &tabs), None, "closed, moved or docked targets must not create a new tab");
        }
    }

    #[test]
    fn closing_one_target_keeps_unrelated_requests() {
        let mut pending = Pending::default();
        pending.insert(1, Placement::Split { tab: 1, target: "a".into(), row: true });
        pending.insert(2, Placement::Split { tab: 1, target: "b".into(), row: false });
        pending.insert(3, Placement::NewTab);
        pending.cancel_pane("a");
        assert_eq!(pending.take(1, &[tab(1, "a")]), None);
        assert!(pending.take(2, &[tab(1, "b")]).is_some());
        pending.cancel_tab(1);
        assert_eq!(pending.take(3, &[]), Some(Placement::NewTab));
        pending.insert(4, Placement::Split { tab: 1, target: "b".into(), row: true });
        pending.cancel_tab(1);
        assert_eq!(pending.take(4, &[tab(1, "b")]), None);
    }

    #[test]
    fn errors_from_cancelled_requests_do_not_surface() {
        let backend = Backend::new(|| {}, || None);
        let mut pending = Pending::default();
        pending.insert(1, Placement::Split { tab: 1, target: "a".into(), row: true });
        pending.cancel_tab(1);
        assert!(matches!(pending.complete(1, Err("late error".into()), &[], &backend), Ok(None)));
        pending.insert(2, Placement::NewTab);
        assert_eq!(pending.complete(2, Err("live error".into()), &[], &backend).unwrap_err(), "live error");
        backend.shutdown();
    }

    #[cfg(unix)]
    #[test]
    fn a_late_custom_shell_session_is_reclaimed_after_its_target_closes() {
        use crate::backend::Open;
        use crate::core::profile::Kind;
        use crate::core::settings::LocalShell;
        use crate::terminal::{Options, Size};
        use alacritty_terminal::vte::ansi::CursorShape;
        use serde_json::json;
        use std::os::unix::fs::PermissionsExt;
        use std::time::{Duration, Instant};

        let directory = std::env::temp_dir().join(format!("omniterm-shell-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let backend = Backend::new(|| {}, || None);
        struct Cleanup<'a>(&'a Backend, std::path::PathBuf);
        impl Drop for Cleanup<'_> {
            fn drop(&mut self) {
                self.0.shutdown();
                let _ = std::fs::remove_dir_all(&self.1);
            }
        }
        let _cleanup = Cleanup(&backend, directory.clone());
        let program = directory.join("custom shell with spaces");
        std::fs::write(&program, "#!/bin/sh\nprintf 'custom-shell:%s\\n' \"$1\"\nwhile IFS= read -r line; do :; done\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut relative_program = std::path::PathBuf::new();
        for component in std::env::current_dir().unwrap().components() {
            if matches!(component, std::path::Component::Normal(_)) {
                relative_program.push("..");
            }
        }
        relative_program.push(program.strip_prefix("/").unwrap());
        let argument = "literal argument; $(not executed)";
        let session = backend.open_blocking(Open {
            token: 1,
            kind: Kind::Shell,
            cwd: directory.to_string_lossy().into_owned(),
            profile: None,
            companion: false,
            local_shell: LocalShell { program: relative_program.to_string_lossy().into_owned(), args: vec![argument.into()] },
            size: Size { cols: 120, rows: 24 },
            options: Options { scrollback: 1000, cursor: CursorShape::Block, blinking: false },
        }).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !session.terminal.screen_text().contains(&format!("custom-shell:{argument}")) {
            assert!(Instant::now() < deadline, "custom executable and arguments did not reach the real PTY");
            std::thread::sleep(Duration::from_millis(10));
        }
        let id = session.terminal.id.clone();
        assert!(backend.request("session.buffer", json!({ "id": id })).is_ok());
        let mut pending = Pending::default();
        pending.insert(1, Placement::Split { tab: 1, target: "closed".into(), row: true });
        pending.cancel_pane("closed");
        assert!(matches!(pending.complete(1, Ok(Box::new(session)), &[], &backend), Ok(None)));
        let deadline = Instant::now() + Duration::from_secs(5);
        while backend.request("session.buffer", json!({ "id": id })).is_ok() {
            assert!(Instant::now() < deadline, "late PTY session was not closed");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

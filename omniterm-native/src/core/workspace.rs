// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! The workspace the next launch reopens: local tabs, their split layout and each pane's last
//! directory. Only the interface is restored; every pane becomes a fresh shell and no command
//! runs again. Remote, serial and tunnel panes are dropped rather than reconnected unasked.

use crate::layout::{MIN_RATIO, Node};
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;
/// The terminal host allows 20 sessions; leave room for new tabs.
pub const MAX_PANES: usize = 16;
const MAX_TITLE: usize = 60;
const MAX_PATH: usize = 4096;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Saved {
    Pane { cwd: String },
    Split { direction: String, ratio: f32, children: Box<[Saved; 2]> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedTab {
    pub title: String,
    pub root: Saved,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub version: u32,
    pub active: usize,
    pub tabs: Vec<SavedTab>,
}

/// `cwd_of` gives a local pane's directory, or `None` for a pane that is not restored.
pub fn snapshot(node: &Node, cwd_of: &dyn Fn(&str) -> Option<String>) -> Option<Saved> {
    match node {
        Node::Pane(id) => cwd_of(id).map(|cwd| Saved::Pane { cwd }),
        Node::Split { row, ratio, children } => match (snapshot(&children[0], cwd_of), snapshot(&children[1], cwd_of)) {
            (Some(first), Some(second)) => Some(Saved::Split { direction: if *row { "row" } else { "column" }.into(), ratio: *ratio, children: Box::new([first, second]) }),
            (first, second) => first.or(second),
        },
    }
}

impl Workspace {
    fn path() -> std::path::PathBuf {
        super::paths::file("native-workspace.json")
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_vec(self) {
            // Restoring is best effort; the current session is unaffected either way.
            let _ = super::paths::write_atomically(&Self::path(), &json);
        }
    }

    pub fn clear() {
        let _ = std::fs::remove_file(Self::path());
    }

    pub fn load() -> Option<Self> {
        let bytes = std::fs::read(Self::path()).ok()?;
        Self::parse(&serde_json::from_slice(&bytes).ok()?)
    }

    /// Validates untrusted stored data; `None` when nothing usable remains.
    pub fn parse(value: &serde_json::Value) -> Option<Self> {
        if value["version"].as_u64() != Some(VERSION as u64) {
            return None;
        }
        let mut budget = MAX_PANES;
        fn node(value: &serde_json::Value, depth: usize, budget: &mut usize) -> Option<Saved> {
            if depth > MAX_PANES {
                return None;
            }
            match value["type"].as_str()? {
                "pane" => {
                    if *budget == 0 {
                        return None;
                    }
                    *budget -= 1;
                    let cwd = value["cwd"].as_str().filter(|cwd| cwd.len() <= MAX_PATH).unwrap_or_default();
                    Some(Saved::Pane { cwd: cwd.to_string() })
                }
                "split" => {
                    let direction = value["direction"].as_str().filter(|d| *d == "row" || *d == "column")?;
                    let children = value["children"].as_array().filter(|children| children.len() == 2)?;
                    let first = node(&children[0], depth + 1, budget);
                    let second = node(&children[1], depth + 1, budget);
                    match (first, second) {
                        (Some(first), Some(second)) => {
                            let ratio = value["ratio"].as_f64().map(|r| (r as f32).clamp(MIN_RATIO, 1.0 - MIN_RATIO)).unwrap_or(0.5);
                            Some(Saved::Split { direction: direction.into(), ratio, children: Box::new([first, second]) })
                        }
                        (first, second) => first.or(second),
                    }
                }
                _ => None,
            }
        }
        let tabs: Vec<SavedTab> = value["tabs"]
            .as_array()?
            .iter()
            .filter_map(|tab| {
                let root = node(&tab["root"], 0, &mut budget)?;
                let title = tab["title"].as_str().unwrap_or_default().chars().take(MAX_TITLE).collect();
                Some(SavedTab { title, root })
            })
            .collect();
        if tabs.is_empty() {
            return None;
        }
        let active = value["active"].as_u64().map(|a| a as usize).filter(|a| *a < tabs.len()).unwrap_or(0);
        Some(Self { version: VERSION, active, tabs })
    }
}

/// Replaces each saved pane with a live one, left to right. `open` returns a session id, or
/// `None` when the pane could not start; such panes collapse away.
pub fn restore(saved: &Saved, open: &mut dyn FnMut(&str) -> Option<String>) -> Option<Node> {
    match saved {
        Saved::Pane { cwd } => open(cwd).map(Node::Pane),
        Saved::Split { direction, ratio, children } => match (restore(&children[0], open), restore(&children[1], open)) {
            (Some(first), Some(second)) => Some(Node::Split { row: direction == "row", ratio: *ratio, children: Box::new([first, second]) }),
            (first, second) => first.or(second),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn untrusted_snapshots_keep_only_what_is_valid_and_within_budget() {
        let pane = |cwd: &str| json!({ "type": "pane", "cwd": cwd });
        let value = json!({ "version": 1, "active": 9, "tabs": [
            { "title": "x".repeat(99), "root": { "type": "split", "direction": "row", "ratio": 5, "children": [pane("/a"), pane("/b")] } },
            { "title": "bad", "root": { "type": "split", "direction": "sideways", "children": [pane("/c"), pane("/d")] } },
        ] });
        let workspace = Workspace::parse(&value).unwrap();
        assert_eq!(workspace.tabs.len(), 1);
        assert_eq!(workspace.active, 0);
        assert_eq!(workspace.tabs[0].title.chars().count(), MAX_TITLE);
        let Saved::Split { ratio, .. } = &workspace.tabs[0].root else { panic!() };
        assert_eq!(*ratio, 1.0 - MIN_RATIO);
        assert!(Workspace::parse(&json!({ "version": 2, "tabs": [] })).is_none());
    }

    #[test]
    fn panes_that_fail_to_open_collapse_away() {
        let saved = Saved::Split { direction: "column".into(), ratio: 0.3, children: Box::new([Saved::Pane { cwd: "/gone".into() }, Saved::Pane { cwd: "/ok".into() }]) };
        let restored = restore(&saved, &mut |cwd| (cwd == "/ok").then(|| "s1".to_string()));
        assert_eq!(restored, Some(Node::Pane("s1".into())));
        let tree = Node::Pane("a".into()).split("a", "b", true);
        let snap = snapshot(&tree, &|id| (id == "a").then(|| "/home".to_string()));
        assert_eq!(snap, Some(Saved::Pane { cwd: "/home".into() }));
    }
}

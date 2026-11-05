// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! The hosts in `~/.ssh/config`, offered for import. Imported profiles keep only the alias and
//! let ssh resolve it, so what is read here only labels the import list.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

const MAX_DEPTH: usize = 16;
const MAX_BYTES: u64 = 1 << 20;
const MAX_HOSTS: usize = 1000;

#[derive(Clone, Debug, PartialEq)]
pub struct Host {
    pub alias: String,
    pub hostname: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub proxy_jump: Option<String>,
}

impl Host {
    pub fn summary(&self) -> String {
        let user = self.user.as_deref().map(|user| format!("{user}@")).unwrap_or_default();
        let port = self.port.map(|port| format!(":{port}")).unwrap_or_default();
        let target = format!("{user}{}{port}", self.hostname);
        match &self.proxy_jump {
            Some(jump) => format!("{target} · 经 {jump}"),
            None => target,
        }
    }
}

pub struct Config {
    pub path: PathBuf,
    pub error: Option<String>,
    pub hosts: Vec<Host>,
}

/// Splits a directive like ssh does: `Key value`, `Key=value`, quotes and `#` comments.
pub fn tokenize(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current: Option<String> = None;
    let mut quote = false;
    for character in line.trim().chars() {
        if character == '"' {
            quote = !quote;
            current.get_or_insert_with(String::new);
        } else if !quote && character == '#' {
            break;
        } else if !quote && (character.is_whitespace() || (character == '=' && tokens.is_empty())) {
            tokens.extend(current.take());
        } else {
            current.get_or_insert_with(String::new).push(character);
        }
    }
    tokens.extend(current);
    tokens
}

fn glob(pattern: &str, text: &str) -> bool {
    fn matches(pattern: &[char], text: &[char]) -> bool {
        match (pattern.first(), text.first()) {
            (None, None) => true,
            (Some('*'), _) => matches(&pattern[1..], text) || (!text.is_empty() && matches(pattern, &text[1..])),
            (Some('?'), Some(_)) => matches(&pattern[1..], &text[1..]),
            (Some(p), Some(t)) if p == t => matches(&pattern[1..], &text[1..]),
            _ => false,
        }
    }
    let (pattern, text): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    matches(&pattern, &text)
}

struct Section {
    patterns: Vec<String>,
    options: BTreeMap<String, String>,
}

struct Reader<'a> {
    home: &'a Path,
    visited: HashSet<PathBuf>,
    sections: Vec<Section>,
    error: Option<String>,
    root: PathBuf,
}

impl Reader<'_> {
    fn expand(&self, value: &str) -> PathBuf {
        let home = |rest: &str| self.home.join(rest);
        let expanded = if value == "~" { self.home.to_path_buf() } else if let Some(rest) = value.strip_prefix("~/").or_else(|| value.strip_prefix("~\\")) { home(rest) } else { PathBuf::from(value) };
        if expanded.is_absolute() { expanded } else { self.home.join(".ssh").join(expanded) }
    }

    /// Expands one path segment at a time, sorted like glob(3).
    fn glob_paths(pattern: &Path) -> Vec<PathBuf> {
        let mut matches = vec![PathBuf::new()];
        for component in pattern.components() {
            let segment = component.as_os_str().to_string_lossy();
            if !segment.contains(['*', '?']) {
                for path in &mut matches {
                    path.push(component);
                }
                continue;
            }
            matches = matches
                .iter()
                .flat_map(|base| {
                    let mut names: Vec<String> = std::fs::read_dir(base)
                        .map(|entries| entries.flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect())
                        .unwrap_or_default();
                    names.retain(|name| glob(&segment, name) && (segment.starts_with('.') || !name.starts_with('.')));
                    names.sort();
                    names.into_iter().map(move |name| base.join(name))
                })
                .collect();
        }
        matches
    }

    fn read(&mut self, path: &Path, depth: usize) {
        if depth > MAX_DEPTH || !self.visited.insert(path.to_path_buf()) {
            return;
        }
        let contents = match std::fs::metadata(path) {
            Ok(meta) if meta.is_file() && meta.len() > MAX_BYTES => {
                self.error = Some(format!("{} 太大，无法读取。", path.display()));
                return;
            }
            Ok(meta) if meta.is_file() => std::fs::read_to_string(path),
            Ok(_) => return,
            Err(error) => {
                if error.kind() != std::io::ErrorKind::NotFound && path == self.root {
                    self.error = Some(error.to_string());
                }
                return;
            }
        };
        let Ok(contents) = contents else { return };
        for line in contents.trim_start_matches('\u{feff}').lines() {
            let tokens = tokenize(line);
            let Some((keyword, values)) = tokens.split_first() else { continue };
            match keyword.to_ascii_lowercase().as_str() {
                "host" => self.sections.push(Section { patterns: values.to_vec(), options: BTreeMap::new() }),
                "match" => self.sections.push(Section { patterns: Vec::new(), options: BTreeMap::new() }),
                "include" => {
                    let current = self.sections.last().map(|section| section.patterns.clone()).unwrap_or_default();
                    for value in values {
                        let path = self.expand(value);
                        let found = if path.to_string_lossy().contains(['*', '?']) { Self::glob_paths(&path) } else { vec![path] };
                        for file in found {
                            self.read(&file, depth + 1);
                        }
                    }
                    // An included file's sections end with it; the including section continues.
                    self.sections.push(Section { patterns: current, options: BTreeMap::new() });
                }
                key => {
                    if let (Some(value), Some(section)) = (values.first(), self.sections.last_mut()) {
                        section.options.entry(key.to_string()).or_insert_with(|| value.clone());
                    }
                }
            }
        }
    }
}

fn host_matches(patterns: &[String], alias: &str) -> bool {
    let mut matched = false;
    for pattern in patterns {
        let (negated, pattern) = match pattern.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, pattern.as_str()),
        };
        if glob(pattern, alias) {
            if negated {
                return false;
            }
            matched = true;
        }
    }
    matched
}

pub fn read(home: &Path) -> Config {
    let path = home.join(".ssh").join("config");
    let mut reader = Reader { home, visited: HashSet::new(), sections: vec![Section { patterns: vec!["*".into()], options: BTreeMap::new() }], error: None, root: path.clone() };
    reader.read(&path, 0);
    let mut aliases: Vec<String> = Vec::new();
    for section in &reader.sections {
        for pattern in &section.patterns {
            if !pattern.starts_with('!') && !pattern.contains(['*', '?']) && !aliases.contains(pattern) {
                aliases.push(pattern.clone());
            }
        }
    }
    aliases.truncate(MAX_HOSTS);
    let hosts = aliases
        .into_iter()
        .map(|alias| {
            let mut options: BTreeMap<String, String> = BTreeMap::new();
            for section in reader.sections.iter().filter(|section| host_matches(&section.patterns, &alias)) {
                for (key, value) in &section.options {
                    options.entry(key.clone()).or_insert_with(|| value.clone());
                }
            }
            let hostname = options.get("hostname").cloned().unwrap_or_else(|| alias.clone()).replace("%h", &alias).replace("%%", "%");
            Host {
                hostname,
                user: options.get("user").cloned(),
                port: options.get("port").and_then(|port| port.parse().ok()).filter(|port| *port > 0),
                proxy_jump: options.get("proxyjump").filter(|jump| !jump.eq_ignore_ascii_case("none")).cloned(),
                alias,
            }
        })
        .collect();
    Config { path, error: reader.error, hosts }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directives_split_like_ssh() {
        assert_eq!(tokenize("Host a b # comment"), ["Host", "a", "b"]);
        assert_eq!(tokenize("HostName=box"), ["HostName", "box"]);
        assert_eq!(tokenize(r#"IdentityFile "~/my key""#), ["IdentityFile", "~/my key"]);
    }

    #[test]
    fn hosts_take_the_first_value_of_every_matching_section() {
        let home = std::env::temp_dir().join(format!("omniterm-ssh-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(home.join(".ssh/conf.d")).unwrap();
        std::fs::write(home.join(".ssh/config"), "Include conf.d/*\nHost prod web-*\n  HostName %h.example.com\n  Port 2222\nHost *\n  User ops\n  Port 22\n").unwrap();
        std::fs::write(home.join(".ssh/conf.d/extra"), "Host db\n  ProxyJump bastion\n").unwrap();
        let config = read(&home);
        let names: Vec<_> = config.hosts.iter().map(|host| host.alias.as_str()).collect();
        assert_eq!(names, ["db", "prod"]);
        let prod = &config.hosts[1];
        assert_eq!((prod.hostname.as_str(), prod.port, prod.user.as_deref()), ("prod.example.com", Some(2222), Some("ops")));
        assert_eq!(config.hosts[0].summary(), "ops@db:22 · 经 bastion");
        let _ = std::fs::remove_dir_all(home);
    }
}

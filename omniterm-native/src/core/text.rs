// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Small text helpers shared by the terminal views, the palette and the sidebar.

/// The local directory an OSC 7 report names, if it is local. A report from another host (an
/// `ssh` inside a local shell) is not a local directory.
pub fn cwd_from_osc7(value: &str, windows: bool, local_host: &str) -> Option<String> {
    let rest = value.strip_prefix("file://")?;
    let (host, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => return None,
    };
    let short = |name: &str| name.split('.').next().unwrap_or_default().to_ascii_lowercase();
    if !host.is_empty() && host != "localhost" && !local_host.is_empty() && short(host) != short(local_host) {
        return None;
    }
    let mut decoded = percent_decode(path)?;
    if windows && decoded.len() >= 3 && decoded.as_bytes()[0] == b'/' && decoded.as_bytes()[1].is_ascii_alphabetic() && decoded.as_bytes()[2] == b':' {
        decoded = decoded[1..].replace('/', "\\");
    }
    (!decoded.is_empty()).then_some(decoded)
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// A dropped file's path, quoted for the shell it is typed into.
pub fn quote_path(filename: &str, windows: bool, powershell: bool) -> String {
    let plain = |allowed: &dyn Fn(char) -> bool| filename.chars().all(|character| character.is_ascii_alphanumeric() || allowed(character));
    if windows && powershell {
        // PowerShell expands `$()` and backticks inside double quotes; single quotes are literal.
        if plain(&|c| "_:\\./-".contains(c)) {
            return filename.to_string();
        }
        let mut quoted = String::from("'");
        for character in filename.chars() {
            if matches!(character, '\'' | '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}') {
                quoted.push(character);
            }
            quoted.push(character);
        }
        quoted.push('\'');
        return quoted;
    }
    if windows {
        return if filename.chars().any(|c| c.is_whitespace() || "&()^;,".contains(c)) { format!("\"{filename}\"") } else { filename.to_string() };
    }
    if plain(&|c| "_@%+=:,./-".contains(c)) { filename.to_string() } else { format!("'{}'", filename.replace('\'', "'\\''")) }
}

/// Subsequence match with bonuses for word starts and consecutive characters; 0 means no match.
pub fn fuzzy_score(query: &str, text: &str) -> f32 {
    if query.is_empty() {
        return 1.0;
    }
    let haystack: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0.0;
    let mut position: isize = -1;
    let mut streak = 0;
    for needle in query.to_lowercase().chars() {
        let Some(found) = haystack.iter().skip((position + 1) as usize).position(|&c| c == needle) else { return 0.0 };
        let next = position + 1 + found as isize;
        streak = if next == position + 1 { streak + 1 } else { 0 };
        let word_start = next == 0 || matches!(haystack[next as usize - 1], ' ' | '\t' | '·' | '/' | '_' | '-');
        score += 1.0 + streak as f32 * 2.0 + if word_start { 3.0 } else { 0.0 } - ((next - position - 1).min(5) as f32) * 0.1;
        position = next;
    }
    score
}

/// `~/…` for paths under the home directory.
pub fn path_label(value: &str, home: &str) -> String {
    match value.strip_prefix(home) {
        Some(rest) if !home.is_empty() => format!("~{rest}"),
        _ => value.to_string(),
    }
}

pub fn basename(value: &str) -> &str {
    value.rsplit(['/', '\\']).find(|part| !part.is_empty()).unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc7_reports_only_local_directories() {
        assert_eq!(cwd_from_osc7("file://box/home/a%20b", false, "box.local").as_deref(), Some("/home/a b"));
        assert_eq!(cwd_from_osc7("file://other/home", false, "box"), None);
        assert_eq!(cwd_from_osc7("file:///C:/Users/me", true, "box").as_deref(), Some("C:\\Users\\me"));
        assert_eq!(cwd_from_osc7("http://box/x", false, "box"), None);
    }

    #[test]
    fn dropped_paths_are_quoted_for_their_shell() {
        assert_eq!(quote_path("/tmp/a b", false, false), "'/tmp/a b'");
        assert_eq!(quote_path("/tmp/it's", false, false), "'/tmp/it'\\''s'");
        assert_eq!(quote_path("/tmp/plain.txt", false, false), "/tmp/plain.txt");
        assert_eq!(quote_path("C:\\a b", true, false), "\"C:\\a b\"");
        assert_eq!(quote_path("C:\\it's $x", true, true), "'C:\\it''s $x'");
    }

    #[test]
    fn fuzzy_matching_prefers_word_starts() {
        assert_eq!(fuzzy_score("xyz", "new tab"), 0.0);
        assert!(fuzzy_score("nt", "new tab") > fuzzy_score("nt", "content"));
        assert_eq!(basename("/a/b/"), "b");
        assert_eq!(path_label("/home/me/src", "/home/me"), "~/src");
    }
}

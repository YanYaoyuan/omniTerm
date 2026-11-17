// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Local, bounded semantic coloring of plain terminal output. It never changes
//! the emulator's cells, copied text, remote output, or application ANSI colors.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::cell::Flags;
mod palette;
pub use palette::{Palette, Style};
use crate::terminal::Listener;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Role {
    #[default] Plain,
    Key, String, Number, Literal, Address, Path, Time, Error, Warning, Success, Info,
}


fn time(value: &str) -> bool {
    let b = value.as_bytes();
    let digits = |range: std::ops::Range<usize>| b.get(range).is_some_and(|s| s.iter().all(u8::is_ascii_digit));
    (b.len() >= 10 && digits(0..4) && b[4] == b'-' && digits(5..7) && b[7] == b'-' && digits(8..10))
        || (b.len() >= 8 && digits(0..2) && b[2] == b':' && digits(3..5) && b[5] == b':' && digits(6..8))
}

fn address(value: &str) -> bool {
    let value = value.trim_end_matches([':', '#', '$']);
    value.parse::<std::net::IpAddr>().is_ok() || value.parse::<std::net::SocketAddr>().is_ok()
        || value.split_once('/').is_some_and(|(ip, prefix)| ip.parse::<std::net::IpAddr>().is_ok() && prefix.parse::<u8>().is_ok_and(|n| n <= if ip.contains(':') { 128 } else { 32 }))
        || value.split_once('@').is_some_and(|(user, host)| !user.is_empty() && !host.is_empty() && user.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c)))
}

fn value_role(value: &str) -> Role {
    if time(value) { Role::Time }
    else if address(value) { Role::Address }
    else if value.starts_with('/') || value.starts_with("~/") || value.contains("://") || (value.contains('/') && value.chars().any(|c| c.is_ascii_alphabetic())) { Role::Path }
    else { Role::Plain }
}

fn delimiter(c: char) -> bool { c.is_whitespace() || "{}[](),;=\"'".contains(c) }

fn word_role(word: &str) -> Role {
    match word.to_ascii_lowercase().as_str() {
        "error" | "errors" | "failed" | "failure" | "fatal" | "panic" | "critical" | "exception" | "traceback" | "denied" => Role::Error,
        "warn" | "warning" | "warnings" | "timeout" | "retry" | "retrying" => Role::Warning,
        "ok" | "success" | "successful" | "succeeded" | "passed" | "ready" | "healthy" | "connected" => Role::Success,
        "info" | "debug" | "trace" => Role::Info,
        "true" | "false" | "null" | "none" | "nan" | "inf" | "infinity" => Role::Literal,
        _ => Role::Plain,
    }
}

/// Token offsets are Unicode scalar positions, matching terminal cells rather
/// than UTF-8 byte offsets. Quoted IDs remain one string, never numeric fragments.
pub fn classify(chars: &[char]) -> Vec<Role> {
    let mut roles = vec![Role::Plain; chars.len()];
    let mut i = 0;
    while i < chars.len() {
        let start = i;
        if matches!(chars[i], '"' | '\'') {
            let quote = chars[i]; i += 1;
            let mut escaped = false;
            while i < chars.len() {
                let c = chars[i]; i += 1;
                if escaped { escaped = false; }
                else if c == '\\' { escaped = true; }
                else if c == quote { break; }
            }
            let next = (i..chars.len()).find(|&at| !chars[at].is_whitespace());
            let closed = i > start + 1 && chars[i-1] == quote;
            let role = if closed && next.is_some_and(|at| matches!(chars[at], ':' | '=')) { Role::Key }
                else {
                    let value: String = chars[start+1..i-usize::from(closed)].iter().collect();
                    match value_role(&value) {
                        Role::Plain => match word_role(&value) { Role::Error | Role::Warning | Role::Success | Role::Info => word_role(&value), _ => Role::String },
                        role => role,
                    }
                };
            roles[start..i].fill(role);
            continue;
        }
        if chars[i] == '[' && chars.get(i+1).is_some_and(|c| c.is_ascii_hexdigit() || *c == ':')
            && let Some(end) = (i+1..chars.len().min(i+64)).find(|&at| chars[at] == ']') {
                let mut after = end+1;
                if chars.get(after) == Some(&':') {
                    after += 1;
                    while chars.get(after).is_some_and(char::is_ascii_digit) { after += 1; }
                }
                let token: String = chars[i..after].iter().collect();
                if address(&token) { roles[i..after].fill(Role::Address); i = after; continue; }
        }
        if delimiter(chars[i]) || chars[i] == ':' { i += 1; continue; }
        while i < chars.len() && !delimiter(chars[i]) { i += 1; }
        let token: String = chars[start..i].iter().collect();
        let mut role = value_role(&token);
        if role == Role::Plain {
            // A colon after a bare log/config key is syntax, while colons in
            // addresses, URLs and timestamps were already consumed above.
            let end = (start..i).find(|&at| chars[at] == ':').unwrap_or(i);
            let word: String = chars[start..end].iter().collect();
            role = word_role(&word);
            if role == Role::Plain && (chars.get(end) == Some(&':') || chars.get(i) == Some(&'='))
                && !word.is_empty() && word.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
                && word.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') { role = Role::Key; }
            if end < i { i = end; }
            if role == Role::Plain && word.chars().any(|c| c.is_ascii_digit()) && word.parse::<f64>().is_ok() { role = Role::Number; }
        }
        roles[start..i].fill(role);
    }
    roles
}

/// Join soft-wrapped rows so JSON strings and keys keep their roles across
/// wrapping and scrollback. Work is bounded to the viewport plus 64K context.
pub fn screen(term: &Term<Listener>) -> Vec<Role> {
    const CONTEXT: usize = 64 * 1024;
    let grid = term.grid();
    let columns = term.columns();
    let rows = term.screen_lines();
    let first = -(grid.display_offset() as i32);
    let last = first + rows as i32 - 1;
    let mut begin = first;
    while begin > -(grid.history_size() as i32) && (first - begin) as usize * columns < CONTEXT
        && grid[Line(begin-1)][Column(columns-1)].flags.contains(Flags::WRAPLINE) { begin -= 1; }
    let mut end = last;
    while end < rows as i32-1 && (end-last) as usize * columns < CONTEXT
        && grid[Line(end)][Column(columns-1)].flags.contains(Flags::WRAPLINE) { end += 1; }
    let mut result = vec![Role::Plain; rows * columns];
    let mut text = Vec::new();
    let mut positions = Vec::new();
    for row in begin..=end {
        let line = &grid[Line(row)];
        let wrapped = line[Column(columns-1)].flags.contains(Flags::WRAPLINE);
        for col in 0..columns {
            let cell = &line[Column(col)];
            if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) { continue; }
            text.push(cell.c);
            positions.push((first..=last).contains(&row).then(|| (row-first) as usize * columns + col));
        }
        if !wrapped || row == end {
            let used = text.iter().rposition(|&c| c != ' ').map_or(0, |at| at+1);
            for (role, pos) in classify(&text[..used]).into_iter().zip(&positions) {
                if let Some(pos) = pos { result[*pos] = role; }
            }
            text.clear(); positions.clear();
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn roles(text: &str, needle: &str) -> Vec<Role> {
        let start = text[..text.find(needle).unwrap()].chars().count();
        classify(&text.chars().collect::<Vec<_>>())[start..start+needle.chars().count()].to_vec()
    }

    #[test]
    fn structured_logs_color_keys_values_and_scientific_numbers() {
        let text = r#"{"loss": 0.126, "lr": 7.6e-06, "id":"qa-12345", "ok":true, "中文":"值\"text"}"#;
        for (needle, role) in [("loss",Role::Key),("0.126",Role::Number),("7.6e-06",Role::Number),("qa-12345",Role::String),("true",Role::Literal),("中文",Role::Key)] {
            assert!(roles(text,needle).iter().all(|r| *r==role), "{needle}");
        }
    }

    #[test]
    fn severities_addresses_paths_and_times_are_distinct() {
        for (needle,role) in [("ERROR",Role::Error),("WARNING",Role::Warning),("OK",Role::Success),("INFO",Role::Info),("192.0.2.10:22",Role::Address),("[::1]:22",Role::Address),("/var/log/app.log",Role::Path),("2026-11-17T12:13:30Z",Role::Time)] {
            assert!(roles(needle,needle).iter().all(|r| *r==role), "{needle}");
        }
        assert!(roles("error_count=42","error_count").iter().all(|r| *r==Role::Key));
        assert!(roles("classifier_failed_check","classifier_failed_check").iter().all(|r| *r==Role::Plain));
    }

    #[test]
    fn wrapped_json_and_scrollback_keep_token_context_without_changing_text() {
        use crate::terminal::{Terminal, Size, Options};
        use alacritty_terminal::vte::ansi::CursorShape;
        let (send, _recv) = std::sync::mpsc::channel();
        let terminal = Terminal::new("highlight".into(), Size {cols:12,rows:4}, Options {scrollback:100,cursor:CursorShape::Block,blinking:false}, send);
        let text = r#"{"loss":0.126,"question_ids":["qa-1234567890","qa-9876543210"],"lr":7.6e-06,"ok":true}"#;
        terminal.feed(format!("{text}\r\n").as_bytes());
        let before = terminal.screen_text();
        for scroll in [0,2] {
            terminal.scroll(scroll);
            let term = terminal.lock();
            let actual = screen(&term);
            let expected = classify(&text.chars().collect::<Vec<_>>());
            let history = term.grid().history_size() as i32;
            let first = -(term.grid().display_offset() as i32);
            for row in 0..term.screen_lines() {
                for col in 0..term.columns() {
                    let index = (first+row as i32+history) as usize*term.columns()+col;
                    assert_eq!(actual[row*term.columns()+col], expected.get(index).copied().unwrap_or_default(), "row {row} col {col}");
                }
            }
        }
        terminal.scroll(-100);
        assert_eq!(terminal.screen_text(),before);
    }

    #[test]
    fn fullwidth_characters_keep_their_cell_alignment() {
        use crate::terminal::{Terminal, Size, Options};
        use alacritty_terminal::vte::ansi::CursorShape;
        let (send, _recv) = std::sync::mpsc::channel();
        let terminal = Terminal::new("highlight".into(), Size {cols:30,rows:2}, Options {scrollback:100,cursor:CursorShape::Block,blinking:false}, send);
        terminal.feed("{\"中文\":123,\"值\":\"测试\"}".as_bytes());
        let term = terminal.lock();
        let roles = screen(&term);
        for (col, role) in roles.iter().enumerate().take(term.columns()) {
            match term.grid()[Line(0)][Column(col)].c {
                '中' | '文' | '值' => assert_eq!(*role,Role::Key),
                '测' | '试' => assert_eq!(*role,Role::String),
                _ => {},
            }
        }
    }

    #[test]
    fn malformed_long_lines_are_bounded_and_do_not_panic() {
        for input in ["[".repeat(100_000), "\\\"".repeat(20_000), "中".repeat(20_000), "\"".into()] {
            assert_eq!(classify(&input.chars().collect::<Vec<_>>()).len(),input.chars().count());
        }
    }
}

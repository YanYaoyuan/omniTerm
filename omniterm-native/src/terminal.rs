// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! One terminal's screen: alacritty's emulator fed with the bytes the host's PTY produced, plus
//! what xterm.js gave the desktop app on top: working-directory reports (OSC 7), search, mouse
//! reporting and the exit notice.

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::search::RegexSearch;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{ClearMode, CursorShape, CursorStyle, Handler, Processor};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};

/// Something a terminal asks the app to do.
#[derive(Debug, PartialEq)]
pub enum Request {
    /// A reply the emulator owes the program, such as a cursor position report.
    Reply { id: String, text: String },
    Title { id: String, title: Option<String> },
    /// OSC 7: the shell moved to this directory URL.
    Directory { id: String, url: String },
    Bell { id: String },
}

#[derive(Clone)]
pub struct Listener {
    id: String,
    requests: Sender<Request>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let request = match event {
            Event::PtyWrite(text) => Request::Reply { id: self.id.clone(), text },
            Event::Title(title) => Request::Title { id: self.id.clone(), title: Some(title.chars().take(120).collect()) },
            Event::ResetTitle => Request::Title { id: self.id.clone(), title: None },
            Event::Bell => Request::Bell { id: self.id.clone() },
            // Clipboard access from programs (OSC 52) stays off: the desktop app never allowed it.
            _ => return,
        };
        let _ = self.requests.send(request);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub cols: usize,
    pub rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub scrollback: usize,
    pub cursor: CursorShape,
    pub blinking: bool,
}

impl Options {
    fn config(self) -> Config {
        Config { scrolling_history: self.scrollback, default_cursor_style: CursorStyle { shape: self.cursor, blinking: self.blinking }, ..Config::default() }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchOptions {
    pub case_sensitive: bool,
    pub regex: bool,
}

/// OSC 7 is not something alacritty reports, so the byte stream is watched for it here: a
/// small state machine that survives sequences split across reads and keeps only the URL.
#[derive(Default)]
struct DirectoryWatch {
    matched: usize,
    url: Option<Vec<u8>>,
}

impl DirectoryWatch {
    const PREFIX: &'static [u8] = b"\x1b]7;";
    const MAX: usize = 4096 + 64;

    fn active(&self) -> bool {
        self.matched > 0 || self.url.is_some()
    }

    fn scan(&mut self, bytes: &[u8], found: &mut impl FnMut(String)) {
        for &byte in bytes {
            if let Some(url) = self.url.as_mut() {
                // BEL or ESC (the start of ST) ends the sequence.
                if byte == 0x07 || byte == 0x1b {
                    if let Some(url) = self.url.take().and_then(|url| String::from_utf8(url).ok()) {
                        found(url);
                    }
                    self.matched = usize::from(byte == 0x1b);
                } else if url.len() >= Self::MAX {
                    self.url = None;
                } else {
                    url.push(byte);
                }
                continue;
            }
            if byte == Self::PREFIX[self.matched] {
                self.matched += 1;
                if self.matched == Self::PREFIX.len() {
                    self.url = Some(Vec::new());
                    self.matched = 0;
                }
            } else {
                self.matched = usize::from(byte == 0x1b);
            }
        }
    }
}

impl std::fmt::Debug for Terminal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Terminal").field("id", &self.id).finish_non_exhaustive()
    }
}

pub struct Terminal {
    pub id: String,
    term: Mutex<Term<Listener>>,
    parser: Mutex<(Processor, DirectoryWatch)>,
    requests: Sender<Request>,
    /// Moves on with every change, so a frame can tell whether anything needs drawing.
    pub generation: AtomicU64,
    pub exited: AtomicBool,
}

impl Terminal {
    pub fn new(id: String, size: Size, options: Options, requests: Sender<Request>) -> Arc<Self> {
        let listener = Listener { id: id.clone(), requests: requests.clone() };
        Arc::new(Self {
            id,
            term: Mutex::new(Term::new(options.config(), &size, listener)),
            parser: Mutex::new((Processor::new(), DirectoryWatch::default())),
            requests,
            generation: AtomicU64::new(1),
            exited: AtomicBool::new(false),
        })
    }

    pub fn lock(&self) -> MutexGuard<'_, Term<Listener>> {
        self.term.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Runs on the host's output thread, never the UI thread, so a flood of output can't stall
    /// a frame for longer than one lock.
    pub fn feed(&self, bytes: &[u8]) {
        let mut parser = self.parser.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (processor, watch) = &mut *parser;
        if watch.active() || bytes.contains(&0x1b) {
            let id = &self.id;
            watch.scan(bytes, &mut |url| {
                let _ = self.requests.send(Request::Directory { id: id.clone(), url });
            });
        }
        let mut term = self.lock();
        processor.advance(&mut *term, bytes);
        drop(term);
        self.touch();
    }

    /// Prints the gray exit line the desktop app shows under a finished session.
    pub fn finish(&self, code: Option<i64>) {
        if self.exited.swap(true, Ordering::AcqRel) {
            return;
        }
        let code = code.map(|code| format!(" · 退出码 {code}")).unwrap_or_default();
        self.feed(format!("\r\n\x1b[38;2;139;152;170m进程已退出{code}\x1b[0m").as_bytes());
    }

    pub fn touch(&self) {
        self.generation.fetch_add(1, Ordering::Release);
    }

    pub fn set_options(&self, options: Options) {
        self.lock().set_options(options.config());
        self.touch();
    }

    pub fn resize(&self, size: Size) -> bool {
        let mut term = self.lock();
        if term.columns() == size.cols && term.screen_lines() == size.rows {
            return false;
        }
        term.resize(size);
        drop(term);
        self.touch();
        true
    }

    pub fn mode(&self) -> TermMode {
        *self.lock().mode()
    }

    pub fn scroll(&self, lines: i32) {
        self.lock().scroll_display(Scroll::Delta(lines));
        self.touch();
    }

    pub fn scroll_to_bottom(&self) {
        let mut term = self.lock();
        if term.grid().display_offset() != 0 {
            term.scroll_display(Scroll::Bottom);
            drop(term);
            self.touch();
        }
    }

    /// Clears the screen and the scrollback, as xterm's `clear()` does, keeping the prompt line.
    pub fn clear(&self) {
        let mut term = self.lock();
        term.clear_screen(ClearMode::Saved);
        term.scroll_display(Scroll::Bottom);
        drop(term);
        self.touch();
    }

    /// `column` and `row` are positions on screen, counting the current scroll.
    pub fn start_selection(&self, column: usize, row: usize, side: Side, kind: SelectionType) {
        let mut term = self.lock();
        let point = screen_point(&term, column, row);
        term.selection = Some(Selection::new(kind, point, side));
        drop(term);
        self.touch();
    }

    pub fn extend_selection(&self, column: usize, row: usize, side: Side) {
        let mut term = self.lock();
        let point = screen_point(&term, column, row);
        if let Some(selection) = term.selection.as_mut() {
            selection.update(point, side);
        }
        drop(term);
        self.touch();
    }

    pub fn select_all(&self) {
        let mut term = self.lock();
        let start = Point::new(term.topmost_line(), Column(0));
        let end = Point::new(term.bottommost_line(), term.last_column());
        let mut selection = Selection::new(SelectionType::Simple, start, Side::Left);
        selection.update(end, Side::Right);
        term.selection = Some(selection);
        drop(term);
        self.touch();
    }

    pub fn clear_selection(&self) {
        let mut term = self.lock();
        if term.selection.take().is_some() {
            drop(term);
            self.touch();
        }
    }

    #[cfg(test)]
    pub fn has_selection(&self) -> bool {
        self.lock().selection.as_ref().is_some_and(|selection| !selection.is_empty())
    }

    pub fn selection_text(&self) -> Option<String> {
        self.lock().selection_to_string().filter(|text| !text.is_empty())
    }

    /// Finds the next match after the current selection (or before it when `backwards`),
    /// selects it and scrolls it into view. Returns whether anything matched.
    pub fn find(&self, query: &str, backwards: bool, options: SearchOptions) -> bool {
        if query.is_empty() {
            self.clear_selection();
            return true;
        }
        let pattern = if options.regex { query.to_string() } else { escape(query) };
        let pattern = if options.case_sensitive { format!("(?-i:{pattern})") } else { format!("(?i:{pattern})") };
        let Ok(mut regex) = RegexSearch::new(&pattern) else { return false };
        let mut term = self.lock();
        let origin = match (&term.selection, backwards) {
            (Some(selection), _) => match selection.to_range(&term) {
                Some(range) if backwards => range.start,
                Some(range) => range.end,
                None => term.grid().cursor.point,
            },
            (None, _) => term.grid().cursor.point,
        };
        let direction = if backwards { Direction::Left } else { Direction::Right };
        let side = if backwards { Side::Left } else { Side::Right };
        let origin = if backwards { origin.sub(&*term, Boundary::None, 1) } else { origin.add(&*term, Boundary::None, 1) };
        let Some(found) = term.search_next(&mut regex, origin, direction, side, None) else { return false };
        let mut selection = Selection::new(SelectionType::Simple, *found.start(), Side::Left);
        selection.update(*found.end(), Side::Right);
        term.selection = Some(selection);
        term.scroll_to_point(*found.start());
        drop(term);
        self.touch();
        true
    }

    /// The word or URL at a screen position, for Ctrl/Cmd+click.
    pub fn link_at(&self, column: usize, row: usize) -> Option<String> {
        let term = self.lock();
        let point = screen_point(&term, column, row);
        let line = &term.grid()[point.line];
        let text: String = (0..term.columns()).map(|index| line[Column(index)].c).collect();
        drop(term);
        link_in(&text, point.column.0)
    }

    /// The visible screen as text, one line per row.
    pub fn screen_text(&self) -> String {
        let term = self.lock();
        let mut lines = vec![String::new(); term.screen_lines()];
        let offset = term.grid().display_offset() as i32;
        for cell in term.grid().display_iter() {
            let row = (cell.point.line.0 + offset) as usize;
            if row < lines.len() {
                lines[row].push(cell.c);
            }
        }
        lines.iter().map(|line| line.trim_end()).collect::<Vec<_>>().join("\n")
    }
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for character in text.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(character) {
            out.push('\\');
        }
        out.push(character);
    }
    out
}

/// An http(s) URL covering `column` in a line of text.
pub fn link_in(line: &str, column: usize) -> Option<String> {
    let characters: Vec<char> = line.chars().collect();
    let text: String = characters.iter().collect();
    let mut search = 0;
    while let Some(found) = text[search..].find("http") {
        let start_byte = search + found;
        let start = text[..start_byte].chars().count();
        let tail = &characters[start..];
        let prefix: String = tail.iter().take(8).collect();
        if prefix.starts_with("https://") || prefix.starts_with("http://") {
            let mut end = start;
            while end < characters.len() && !characters[end].is_whitespace() && !"<>\"'`".contains(characters[end]) {
                end += 1;
            }
            while end > start && ".,;:!?)]}".contains(characters[end - 1]) {
                end -= 1;
            }
            if (start..end).contains(&column) {
                return Some(characters[start..end].iter().collect());
            }
        }
        search = start_byte + 4;
    }
    None
}

fn screen_point(term: &Term<Listener>, column: usize, row: usize) -> Point {
    let offset = term.grid().display_offset() as i32;
    let row = row.min(term.screen_lines().saturating_sub(1)) as i32;
    let column = column.min(term.columns().saturating_sub(1));
    Point::new(Line(row - offset), Column(column))
}

/// Mouse reports for programs that asked for them (vim, htop, tmux), in SGR or legacy form.
pub fn mouse_report(mode: TermMode, button: u8, column: usize, row: usize, pressed: bool, motion: bool, modifiers: (bool, bool, bool)) -> Option<String> {
    if !mode.intersects(TermMode::MOUSE_MODE) {
        return None;
    }
    if motion && !mode.intersects(TermMode::MOUSE_MOTION | TermMode::MOUSE_DRAG) {
        return None;
    }
    let (shift, alt, ctrl) = modifiers;
    let code = button + if motion { 32 } else { 0 } + if shift { 4 } else { 0 } + if alt { 8 } else { 0 } + if ctrl { 16 } else { 0 };
    let (x, y) = (column + 1, row + 1);
    if mode.contains(TermMode::SGR_MOUSE) {
        return Some(format!("\x1b[<{code};{x};{y}{}", if pressed { 'M' } else { 'm' }));
    }
    // Legacy reports carry release as button 3 and can't address beyond column 223.
    let code = if pressed { code } else { 3 + (code & !3) };
    if x > 223 || y > 223 {
        return None;
    }
    Some(format!("\x1b[M{}{}{}", (32 + code) as char, (32 + x as u8) as char, (32 + y as u8) as char))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    const OPTIONS: Options = Options { scrollback: 1000, cursor: CursorShape::Beam, blinking: true };

    fn terminal() -> (Arc<Terminal>, std::sync::mpsc::Receiver<Request>) {
        let (sender, receiver) = channel();
        (Terminal::new("t".into(), Size { cols: 20, rows: 4 }, OPTIONS, sender), receiver)
    }

    #[test]
    fn output_lands_on_the_screen_with_colors_stripped() {
        let (terminal, _) = terminal();
        terminal.feed(b"\x1b[31mred\x1b[0m plain\r\nnext");
        assert_eq!(terminal.screen_text(), "red plain\nnext\n\n");
    }

    #[test]
    fn a_cursor_position_query_is_answered_through_the_app() {
        let (terminal, requests) = terminal();
        terminal.feed(b"ab\x1b[6n");
        assert_eq!(requests.try_recv().unwrap(), Request::Reply { id: "t".into(), text: "\x1b[1;3R".into() });
    }

    #[test]
    fn working_directory_reports_are_seen_even_when_split_across_reads() {
        let (terminal, requests) = terminal();
        terminal.feed(b"x\x1b]7;file://box/ho");
        terminal.feed(b"me/a\x07y");
        assert_eq!(requests.try_recv().unwrap(), Request::Directory { id: "t".into(), url: "file://box/home/a".into() });
        terminal.feed(b"\x1b]7;file://box/srv\x1b\\");
        assert_eq!(requests.try_recv().unwrap(), Request::Directory { id: "t".into(), url: "file://box/srv".into() });
    }

    #[test]
    fn a_selection_copies_the_text_under_it_and_search_selects_matches() {
        let (terminal, _) = terminal();
        terminal.feed(b"hello world\r\nHello again");
        terminal.start_selection(0, 0, Side::Left, SelectionType::Simple);
        terminal.extend_selection(4, 0, Side::Right);
        assert_eq!(terminal.selection_text().as_deref(), Some("hello"));
        terminal.clear_selection();
        assert!(terminal.find("hello", false, SearchOptions::default()));
        assert!(terminal.has_selection());
        assert!(terminal.find("Hello", false, SearchOptions { case_sensitive: true, regex: false }));
        assert_eq!(terminal.selection_text().as_deref(), Some("Hello"));
        assert!(!terminal.find("absent", false, SearchOptions::default()));
        assert!(terminal.find("w.rld", false, SearchOptions { case_sensitive: false, regex: true }));
    }

    #[test]
    fn exits_are_announced_once_in_gray() {
        let (terminal, _) = terminal();
        terminal.finish(Some(2));
        terminal.finish(Some(3));
        assert!(terminal.screen_text().contains("进程已退出 · 退出码 2"));
        assert!(!terminal.screen_text().contains("退出码 3"));
    }

    #[test]
    fn links_are_found_under_the_pointer_without_trailing_punctuation() {
        let line = "see https://example.com/a?b=1). done";
        assert_eq!(link_in(line, 6).as_deref(), Some("https://example.com/a?b=1"));
        assert_eq!(link_in(line, 1), None);
    }

    #[test]
    fn mouse_reports_follow_the_mode_the_program_chose() {
        let sgr = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        assert_eq!(mouse_report(sgr, 0, 4, 1, true, false, (false, false, false)).as_deref(), Some("\x1b[<0;5;2M"));
        assert_eq!(mouse_report(sgr, 0, 4, 1, false, false, (false, false, false)).as_deref(), Some("\x1b[<0;5;2m"));
        assert_eq!(mouse_report(TermMode::empty(), 0, 0, 0, true, false, (false, false, false)), None);
        assert_eq!(mouse_report(TermMode::MOUSE_REPORT_CLICK, 0, 0, 0, true, false, (false, false, false)).as_deref(), Some("\x1b[M !!"));
    }
}

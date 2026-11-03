// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! One terminal's screen: alacritty's emulator fed with the bytes the host's PTY produced.

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::Processor;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};

pub const SCROLLBACK: usize = 10_000;

/// Something a terminal asks the app to do.
#[derive(Debug, PartialEq)]
pub enum Request {
    /// A reply the emulator owes the program, such as a cursor position report.
    Reply { id: String, text: String },
    Title { id: String, title: Option<String> },
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
            Event::Title(title) => Request::Title { id: self.id.clone(), title: Some(title) },
            Event::ResetTitle => Request::Title { id: self.id.clone(), title: None },
            // Clipboard access from programs (OSC 52) stays off until it has a permission prompt.
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

pub struct Terminal {
    pub id: String,
    term: Mutex<Term<Listener>>,
    parser: Mutex<Processor>,
    /// Moves on with every change, so a frame can tell whether anything needs drawing.
    pub generation: AtomicU64,
    pub exited: AtomicBool,
}

impl Terminal {
    pub fn new(id: String, size: Size, requests: Sender<Request>) -> Arc<Self> {
        let config = Config { scrolling_history: SCROLLBACK, ..Config::default() };
        let listener = Listener { id: id.clone(), requests };
        Arc::new(Self {
            id,
            term: Mutex::new(Term::new(config, &size, listener)),
            parser: Mutex::new(Processor::new()),
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
        let mut term = self.lock();
        parser.advance(&mut *term, bytes);
        self.touch();
    }

    pub fn touch(&self) {
        self.generation.fetch_add(1, Ordering::Release);
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

    pub fn clear_selection(&self) {
        let mut term = self.lock();
        if term.selection.take().is_some() {
            drop(term);
            self.touch();
        }
    }

    pub fn selection_text(&self) -> Option<String> {
        self.lock().selection_to_string().filter(|text| !text.is_empty())
    }

    /// The visible screen as text, one line per row, for tests and smoke runs.
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

fn screen_point(term: &Term<Listener>, column: usize, row: usize) -> Point {
    let offset = term.grid().display_offset() as i32;
    let row = row.min(term.screen_lines().saturating_sub(1)) as i32;
    let column = column.min(term.columns().saturating_sub(1));
    Point::new(Line(row - offset), Column(column))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    fn terminal() -> (Arc<Terminal>, std::sync::mpsc::Receiver<Request>) {
        let (sender, receiver) = channel();
        (Terminal::new("t".into(), Size { cols: 20, rows: 4 }, sender), receiver)
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
    fn titles_are_reported() {
        let (terminal, requests) = terminal();
        terminal.feed(b"\x1b]2;build\x07");
        assert_eq!(requests.try_recv().unwrap(), Request::Title { id: "t".into(), title: Some("build".into()) });
    }

    #[test]
    fn a_selection_copies_the_text_under_it() {
        let (terminal, _) = terminal();
        terminal.feed(b"hello world");
        terminal.start_selection(0, 0, Side::Left, SelectionType::Simple);
        terminal.extend_selection(4, 0, Side::Right);
        assert_eq!(terminal.selection_text().as_deref(), Some("hello"));
        terminal.clear_selection();
        assert_eq!(terminal.selection_text(), None);
    }

    #[test]
    fn resizing_reflows_and_reports_whether_anything_changed() {
        let (terminal, _) = terminal();
        assert!(!terminal.resize(Size { cols: 20, rows: 4 }));
        assert!(terminal.resize(Size { cols: 10, rows: 3 }));
        terminal.feed(b"0123456789abc");
        assert_eq!(terminal.screen_text(), "0123456789\nabc\n");
    }
}

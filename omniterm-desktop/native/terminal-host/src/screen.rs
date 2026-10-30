//! Terminal screen semantics for Agent context.
//!
//! Context comes from a real terminal parser, never from stripped PTY text: carriage-return
//! overwrites, cursor movement, wide characters and the alternate screen all resolve here.

use vt100::Callbacks;
use vt100::Parser;
use vt100::Screen;

pub const SCROLLBACK_LINES: usize = 2000;
const DELTA_SCROLLBACK_LINES: usize = 20_000;

/// Shell-integration event observed in the output stream. Output is untrusted: these are
/// observations about the terminal, never a source of permissions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellEvent {
    PromptStart { trusted: bool },
    CommandStart { trusted: bool },
    CommandExecuted { trusted: bool },
    CommandFinished { trusted: bool, exit_code: Option<i32> },
    WorkingDirectory(String),
}

#[derive(Debug, Default)]
pub struct Observer {
    nonce: String,
    pub events: Vec<ShellEvent>,
    pub title: Option<String>,
}

impl Observer {
    fn shell_mark(&mut self, params: &[&[u8]]) {
        let Some(kind) = params.get(1).and_then(|kind| kind.first()) else {
            return;
        };
        let trusted = !self.nonce.is_empty()
            && params[2..].iter().any(|param| {
                param
                    .strip_prefix(b"k=")
                    .is_some_and(|nonce| nonce == self.nonce.as_bytes())
            });
        let event = match kind {
            b'A' => ShellEvent::PromptStart { trusted },
            b'B' => ShellEvent::CommandStart { trusted },
            b'C' => ShellEvent::CommandExecuted { trusted },
            b'D' => ShellEvent::CommandFinished {
                trusted,
                exit_code: params
                    .get(2)
                    .and_then(|code| std::str::from_utf8(code).ok())
                    .and_then(|code| code.parse().ok()),
            },
            _ => return,
        };
        self.events.push(event);
    }
}

impl Callbacks for Observer {
    fn set_window_title(&mut self, _: &mut Screen, title: &[u8]) {
        self.title = Some(String::from_utf8_lossy(title).chars().take(120).collect());
    }

    fn unhandled_osc(&mut self, _: &mut Screen, params: &[&[u8]]) {
        match params.first() {
            Some(&b"133") => self.shell_mark(params),
            // A path may contain `;`, which the parser splits on.
            Some(&b"7") if params.len() > 1 => {
                let url = params[1..].join(&b';');
                if let Some(path) = osc7_path(&String::from_utf8_lossy(&url)) {
                    self.events.push(ShellEvent::WorkingDirectory(path));
                }
            }
            _ => {}
        }
    }
}

fn osc7_path(url: &str) -> Option<String> {
    let rest = url.strip_prefix("file://")?;
    let path = &rest[rest.find('/')?..];
    let mut bytes = Vec::with_capacity(path.len());
    let mut input = path.bytes();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let high = (input.next()? as char).to_digit(16)?;
            let low = (input.next()? as char).to_digit(16)?;
            bytes.push((high * 16 + low) as u8);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).ok()
}

pub struct ScreenModel {
    parser: Parser<Observer>,
}

/// Rendered lines, oldest first.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Rendered {
    pub lines: Vec<String>,
    /// Lines that existed but were left out by `max_lines`.
    pub omitted: usize,
    pub alternate_screen: bool,
}

impl ScreenModel {
    pub fn new(rows: u16, cols: u16, nonce: &str) -> Self {
        Self {
            parser: Parser::new_with_callbacks(
                rows,
                cols,
                SCROLLBACK_LINES,
                Observer {
                    nonce: nonce.to_string(),
                    ..Observer::default()
                },
            ),
        }
    }

    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    pub fn take_events(&mut self) -> Vec<ShellEvent> {
        std::mem::take(&mut self.parser.callbacks_mut().events)
    }

    pub fn title(&self) -> Option<&str> {
        self.parser.callbacks().title.as_deref()
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.screen_mut().set_size(rows, cols);
    }

    pub fn alternate_screen(&self) -> bool {
        self.parser.screen().alternate_screen()
    }

    pub fn bracketed_paste(&self) -> bool {
        self.parser.screen().bracketed_paste()
    }

    /// Text of the row holding the cursor, up to the cursor.
    pub fn cursor_line(&self) -> String {
        let screen = self.parser.screen();
        let (row, col) = screen.cursor_position();
        let (_, cols) = screen.size();
        screen
            .rows(0, cols)
            .nth(usize::from(row))
            .map(|line| line.chars().take(usize::from(col) + 1).collect())
            .unwrap_or_default()
    }

    pub fn snapshot(&mut self, max_lines: usize) -> Rendered {
        render(self.parser.screen_mut(), max_lines)
    }
}

/// Reads scrollback plus the screen and restores the live view afterwards.
fn render(screen: &mut Screen, max_lines: usize) -> Rendered {
    let (rows, cols) = screen.size();
    let rows_usize = usize::from(rows.max(1));
    screen.set_scrollback(usize::MAX);
    let available = screen.scrollback();
    let mut lines: Vec<String> = Vec::new();
    let mut previous_wrapped = false;
    let mut collect = |screen: &Screen, take: usize| {
        for (row, text) in screen.rows(0, cols).take(take).enumerate() {
            let text = text.trim_end().to_string();
            let joined = previous_wrapped && !lines.is_empty();
            if joined {
                if let Some(last) = lines.last_mut() {
                    last.push_str(&text);
                }
            } else {
                lines.push(text);
            }
            previous_wrapped = u16::try_from(row).is_ok_and(|row| screen.row_wrapped(row));
        }
    };
    let mut offset = available;
    while offset > 0 {
        let take = offset.min(rows_usize);
        screen.set_scrollback(offset);
        collect(screen, take);
        offset -= take;
    }
    screen.set_scrollback(0);
    collect(screen, rows_usize);
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    let omitted = lines.len().saturating_sub(max_lines);
    if omitted > 0 {
        lines.drain(..omitted);
    }
    Rendered {
        lines,
        omitted,
        alternate_screen: screen.alternate_screen(),
    }
}

/// Renders a byte range on its own. The range starts from a blank screen, so absolute cursor
/// moves land relative to that screen, but overwrites inside the range resolve correctly.
pub fn render_bytes(bytes: &[u8], rows: u16, cols: u16, max_lines: usize) -> Rendered {
    let mut parser = Parser::new(rows.max(2), cols.max(2), DELTA_SCROLLBACK_LINES);
    parser.process(bytes);
    render(parser.screen_mut(), max_lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(input: &[u8]) -> Vec<String> {
        render_bytes(input, 5, 40, 100).lines
    }

    #[test]
    fn carriage_return_overwrites_instead_of_concatenating() {
        assert_eq!(lines(b"progress 10%\rprogress 100%\r\ndone\r\n"), ["progress 100%", "done"]);
    }

    #[test]
    fn backspace_and_erase_resolve_to_the_final_text() {
        assert_eq!(lines(b"abcd\x08\x08XY\r\n"), ["abXY"]);
        assert_eq!(lines(b"secret\r\x1b[2Kpublic\r\n"), ["public"]);
    }

    #[test]
    fn colors_are_not_part_of_the_text() {
        assert_eq!(lines(b"\x1b[31mred\x1b[0m text\r\n"), ["red text"]);
    }

    #[test]
    fn cursor_movement_rewrites_earlier_rows() {
        assert_eq!(lines(b"one\r\ntwo\r\n\x1b[2A\x1b[1;1HONE\x1b[3;1H"), ["ONE", "two"]);
    }

    #[test]
    fn unicode_and_wide_characters_keep_their_columns() {
        assert_eq!(lines("中文🙂ok\r\n".as_bytes()), ["中文🙂ok"]);
        assert_eq!(lines("中文\r文\r\n".as_bytes()), ["文文"]);
    }

    #[test]
    fn alternate_screen_is_reported_and_hides_primary_content_until_exit() {
        let mut screen = ScreenModel::new(5, 40, "");
        screen.process(b"before\r\n\x1b[?1049hfull screen app");
        let during = screen.snapshot(100);
        assert!(during.alternate_screen);
        assert_eq!(during.lines, ["full screen app"]);
        screen.process(b"\x1b[?1049l");
        let after = screen.snapshot(100);
        assert!(!after.alternate_screen);
        assert_eq!(after.lines, ["before"]);
    }

    #[test]
    fn scrollback_joins_wrapped_rows_and_marks_omitted_lines() {
        let mut screen = ScreenModel::new(3, 10, "");
        for index in 0..20 {
            screen.process(format!("line {index}\r\n").as_bytes());
        }
        screen.process(b"0123456789abcdef\r\n");
        let rendered = screen.snapshot(4);
        assert_eq!(rendered.lines, ["line 17", "line 18", "line 19", "0123456789abcdef"]);
        assert_eq!(rendered.omitted, 17);
        assert_eq!(screen.parser.screen().scrollback(), 0);
    }

    #[test]
    fn shell_marks_require_the_session_nonce_to_be_trusted() {
        let mut screen = ScreenModel::new(5, 40, "n0nce");
        screen.process(b"\x1b]133;A;k=n0nce\x07\x1b]133;A\x07\x1b]133;C;k=wrong\x07");
        screen.process(b"\x1b]133;D;3;k=n0nce\x07");
        assert_eq!(
            screen.take_events(),
            [
                ShellEvent::PromptStart { trusted: true },
                ShellEvent::PromptStart { trusted: false },
                ShellEvent::CommandExecuted { trusted: false },
                ShellEvent::CommandFinished { trusted: true, exit_code: Some(3) },
            ]
        );
    }

    #[test]
    fn working_directory_reports_decode_percent_escapes() {
        let mut screen = ScreenModel::new(5, 40, "");
        screen.process(b"\x1b]7;file://host/tmp/my%20dir;x\x07");
        assert_eq!(screen.take_events(), [ShellEvent::WorkingDirectory("/tmp/my dir;x".to_string())]);
    }
}

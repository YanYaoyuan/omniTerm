// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Keys to the bytes a terminal program expects, as xterm sends them.

use eframe::egui::{Key, Modifiers};

/// xterm's modifier parameter: 1 + Shift(1) + Alt(2) + Ctrl(4).
fn modifier_code(modifiers: Modifiers) -> u8 {
    1 + modifiers.shift as u8 + 2 * modifiers.alt as u8 + 4 * modifiers.ctrl as u8
}

fn cursor_key(letter: char, modifiers: Modifiers, app_cursor: bool) -> String {
    let code = modifier_code(modifiers);
    if code > 1 {
        format!("\x1b[1;{code}{letter}")
    } else if app_cursor {
        format!("\x1bO{letter}")
    } else {
        format!("\x1b[{letter}")
    }
}

fn tilde_key(number: u8, modifiers: Modifiers) -> String {
    let code = modifier_code(modifiers);
    if code > 1 { format!("\x1b[{number};{code}~") } else { format!("\x1b[{number}~") }
}

/// Bytes for a key that produces no text of its own, or `None` to let the text event speak.
/// `modifiers.command` is ignored: on macOS it belongs to the app, never the terminal.
pub fn key_bytes(key: Key, modifiers: Modifiers, app_cursor: bool) -> Option<String> {
    let plain = !modifiers.ctrl && !modifiers.alt;
    let bytes = match key {
        Key::Enter => if modifiers.alt { "\x1b\r".into() } else { "\r".into() },
        Key::Backspace => match (modifiers.ctrl, modifiers.alt) {
            (true, _) => "\x08".into(),
            (false, true) => "\x1b\x7f".into(),
            _ => "\x7f".into(),
        },
        Key::Tab => if modifiers.shift { "\x1b[Z".into() } else { "\t".into() },
        Key::Escape => "\x1b".into(),
        Key::ArrowUp => cursor_key('A', modifiers, app_cursor),
        Key::ArrowDown => cursor_key('B', modifiers, app_cursor),
        Key::ArrowRight => cursor_key('C', modifiers, app_cursor),
        Key::ArrowLeft => cursor_key('D', modifiers, app_cursor),
        Key::Home => cursor_key('H', modifiers, app_cursor),
        Key::End => cursor_key('F', modifiers, app_cursor),
        Key::Insert => tilde_key(2, modifiers),
        Key::Delete => tilde_key(3, modifiers),
        Key::PageUp => tilde_key(5, modifiers),
        Key::PageDown => tilde_key(6, modifiers),
        Key::F1 | Key::F2 | Key::F3 | Key::F4 => {
            let letter = match key { Key::F1 => 'P', Key::F2 => 'Q', Key::F3 => 'R', _ => 'S' };
            let code = modifier_code(modifiers);
            if code > 1 { format!("\x1b[1;{code}{letter}") } else { format!("\x1bO{letter}") }
        }
        Key::F5 => tilde_key(15, modifiers),
        Key::F6 => tilde_key(17, modifiers),
        Key::F7 => tilde_key(18, modifiers),
        Key::F8 => tilde_key(19, modifiers),
        Key::F9 => tilde_key(20, modifiers),
        Key::F10 => tilde_key(21, modifiers),
        Key::F12 => tilde_key(24, modifiers),
        _ if plain => return None,
        _ => return control_or_meta(key, modifiers),
    };
    Some(bytes)
}

/// Ctrl+letter and friends are control codes; Alt prefixes ESC, as in xterm's metaSendsEscape.
fn control_or_meta(key: Key, modifiers: Modifiers) -> Option<String> {
    // Ctrl+Alt is AltGr on Windows keyboards: the character arrives as text instead.
    if cfg!(windows) && modifiers.ctrl && modifiers.alt {
        return None;
    }
    let name = key.name();
    let character = match name {
        "Space" => ' ',
        _ if name.chars().count() == 1 => name.chars().next()?,
        _ => return None,
    };
    let base = if modifiers.ctrl {
        let control = match character.to_ascii_lowercase() {
            letter @ 'a'..='z' => letter as u8 - b'a' + 1,
            '[' | '3' => 0x1b,
            '\\' | '4' => 0x1c,
            ']' | '5' => 0x1d,
            '6' => 0x1e,
            '/' | '7' => 0x1f,
            ' ' | '2' | '@' => 0,
            _ => return None,
        };
        (control as char).to_string()
    } else {
        let mut text = character.to_ascii_lowercase().to_string();
        if modifiers.shift {
            text = text.to_uppercase();
        }
        text
    };
    Some(if modifiers.alt { format!("\x1b{base}") } else { base })
}

/// Pasted text, fenced when the program asked for bracketed paste. Line feeds become carriage
/// returns, as a typed Enter would, and a fence inside the text cannot end the paste early.
pub fn paste_bytes(text: &str, bracketed: bool) -> String {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        format!("\x1b[200~{}\x1b[201~", text.replace("\x1b[201~", ""))
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Modifiers = Modifiers::NONE;
    const CTRL: Modifiers = Modifiers { ctrl: true, alt: false, shift: false, mac_cmd: false, command: true };

    #[test]
    fn cursor_keys_follow_the_application_cursor_mode() {
        assert_eq!(key_bytes(Key::ArrowUp, NONE, false).unwrap(), "\x1b[A");
        assert_eq!(key_bytes(Key::ArrowUp, NONE, true).unwrap(), "\x1bOA");
        assert_eq!(key_bytes(Key::ArrowLeft, CTRL, true).unwrap(), "\x1b[1;5D");
    }

    #[test]
    fn editing_and_function_keys_match_xterm() {
        assert_eq!(key_bytes(Key::Enter, NONE, false).unwrap(), "\r");
        assert_eq!(key_bytes(Key::Backspace, NONE, false).unwrap(), "\x7f");
        assert_eq!(key_bytes(Key::Tab, Modifiers::SHIFT, false).unwrap(), "\x1b[Z");
        assert_eq!(key_bytes(Key::Delete, NONE, false).unwrap(), "\x1b[3~");
        assert_eq!(key_bytes(Key::PageDown, NONE, false).unwrap(), "\x1b[6~");
        assert_eq!(key_bytes(Key::F1, NONE, false).unwrap(), "\x1bOP");
        assert_eq!(key_bytes(Key::F5, NONE, false).unwrap(), "\x1b[15~");
    }

    #[test]
    fn control_letters_are_control_codes_and_alt_sends_escape() {
        assert_eq!(key_bytes(Key::C, CTRL, false).unwrap(), "\x03");
        assert_eq!(key_bytes(Key::D, CTRL, false).unwrap(), "\x04");
        assert_eq!(key_bytes(Key::OpenBracket, CTRL, false).unwrap(), "\x1b");
        assert_eq!(key_bytes(Key::B, Modifiers::ALT, false).unwrap(), "\x1bb");
        assert_eq!(key_bytes(Key::Space, CTRL, false).unwrap(), "\0");
        // Plain letters come from the text event, so IMEs and keyboard layouts are respected.
        assert_eq!(key_bytes(Key::A, NONE, false), None);
    }

    #[test]
    fn pastes_are_fenced_and_cannot_close_the_fence() {
        assert_eq!(paste_bytes("a\nb", false), "a\rb");
        assert_eq!(paste_bytes("x\x1b[201~rm", true), "\x1b[200~xrm\x1b[201~");
    }
}

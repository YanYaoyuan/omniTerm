// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Color schemes, matching the desktop app's built-in themes, and the xterm 256-color table.

use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub name: &'static str,
    pub background: Rgb,
    pub foreground: Rgb,
    pub cursor: Rgb,
    pub selection: Rgb,
    pub ansi: [Rgb; 16],
    pub light: bool,
}

const fn hex(value: u32) -> Rgb {
    Rgb { r: (value >> 16) as u8, g: (value >> 8) as u8, b: value as u8 }
}

const fn palette(colors: [u32; 16]) -> [Rgb; 16] {
    let mut out = [Rgb { r: 0, g: 0, b: 0 }; 16];
    let mut index = 0;
    while index < 16 {
        out[index] = hex(colors[index]);
        index += 1;
    }
    out
}

pub const THEMES: [Theme; 3] = [
    Theme {
        name: "Midnight",
        background: hex(0x14181f),
        foreground: hex(0xe0e6ee),
        cursor: hex(0x8db4ff),
        selection: hex(0x2f3d58),
        ansi: palette([
            0x39414e, 0xe58a91, 0xa5c9a1, 0xdec38d, 0x8db4ff, 0xbdacd8, 0x92c4cc, 0xe0e6ee, 0x7d899a, 0xf4a2a8, 0xb9dab5,
            0xefd5a5, 0xaccaff, 0xd4c3ed, 0xaedce3, 0xf5f7fa,
        ]),
        light: false,
    },
    Theme {
        name: "Gruvbox Dark",
        background: hex(0x282828),
        foreground: hex(0xebdbb2),
        cursor: hex(0xebdbb2),
        selection: hex(0x504945),
        ansi: palette([
            0x282828, 0xcc241d, 0x98971a, 0xd79921, 0x458588, 0xb16286, 0x689d6a, 0xa89984, 0x928374, 0xfb4934, 0xb8bb26,
            0xfabd2f, 0x83a598, 0xd3869b, 0x8ec07c, 0xebdbb2,
        ]),
        light: false,
    },
    Theme {
        name: "GitHub Light",
        background: hex(0xffffff),
        foreground: hex(0x1f2328),
        cursor: hex(0x0969da),
        selection: hex(0xc8e1ff),
        ansi: palette([
            0x24292f, 0xcf222e, 0x116329, 0x7d4e00, 0x0969da, 0x8250df, 0x1b7c83, 0x6e7781, 0x57606a, 0xa40e26, 0x1a7f37,
            0x633c01, 0x218bff, 0xa475f9, 0x3192aa, 0x8c959f,
        ]),
        light: true,
    },
];

/// Entry `index` of the xterm 256-color table, for indexes above the 16 theme colors.
pub fn xterm_color(index: u8) -> Rgb {
    match index {
        16..=231 => {
            let value = index - 16;
            let level = |step: u8| if step == 0 { 0 } else { 55 + step * 40 };
            Rgb { r: level(value / 36), g: level(value / 6 % 6), b: level(value % 6) }
        }
        232..=255 => {
            let gray = 8 + (index - 232) * 10;
            Rgb { r: gray, g: gray, b: gray }
        }
        _ => Rgb { r: 0, g: 0, b: 0 },
    }
}

fn dim(color: Rgb) -> Rgb {
    let scale = |value: u8| (value as u16 * 2 / 3) as u8;
    Rgb { r: scale(color.r), g: scale(color.g), b: scale(color.b) }
}

impl Theme {
    /// Resolves a cell color. `overrides` holds colors a program set with OSC 4, 10 or 11.
    pub fn resolve(&self, color: Color, overrides: &dyn Fn(usize) -> Option<Rgb>) -> Rgb {
        match color {
            Color::Spec(rgb) => rgb,
            Color::Indexed(index) => overrides(index as usize).unwrap_or_else(|| self.indexed(index)),
            Color::Named(named) => overrides(named as usize).unwrap_or_else(|| self.named(named)),
        }
    }

    fn indexed(&self, index: u8) -> Rgb {
        if index < 16 { self.ansi[index as usize] } else { xterm_color(index) }
    }

    fn named(&self, named: NamedColor) -> Rgb {
        let index = named as usize;
        if index < 16 {
            return self.ansi[index];
        }
        match named {
            NamedColor::Foreground | NamedColor::BrightForeground => self.foreground,
            NamedColor::Background => self.background,
            NamedColor::Cursor => self.cursor,
            NamedColor::DimForeground => dim(self.foreground),
            NamedColor::DimBlack => dim(self.ansi[0]),
            NamedColor::DimRed => dim(self.ansi[1]),
            NamedColor::DimGreen => dim(self.ansi[2]),
            NamedColor::DimYellow => dim(self.ansi[3]),
            NamedColor::DimBlue => dim(self.ansi[4]),
            NamedColor::DimMagenta => dim(self.ansi[5]),
            NamedColor::DimCyan => dim(self.ansi[6]),
            NamedColor::DimWhite => dim(self.ansi[7]),
            _ => self.foreground,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_256_color_table_matches_xterm() {
        assert_eq!(xterm_color(16), Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(xterm_color(196), Rgb { r: 255, g: 0, b: 0 });
        assert_eq!(xterm_color(231), Rgb { r: 255, g: 255, b: 255 });
        assert_eq!(xterm_color(232), Rgb { r: 8, g: 8, b: 8 });
        assert_eq!(xterm_color(255), Rgb { r: 238, g: 238, b: 238 });
    }

    #[test]
    fn program_set_colors_win_over_the_theme() {
        let theme = THEMES[0];
        let none = |_: usize| None;
        assert_eq!(theme.resolve(Color::Named(NamedColor::Red), &none), theme.ansi[1]);
        assert_eq!(theme.resolve(Color::Named(NamedColor::Background), &none), theme.background);
        let red = Rgb { r: 1, g: 2, b: 3 };
        let set = move |index: usize| (index == 1).then_some(red);
        assert_eq!(theme.resolve(Color::Indexed(1), &set), red);
        assert_eq!(theme.resolve(Color::Spec(red), &none), red);
    }
}

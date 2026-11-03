// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! The desktop app's 13 color schemes, generated from `themes.js` so the colors match exactly,
//! the colors the interface derives from them, and the xterm 256-color table.

use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub id: &'static str,
    pub name: &'static str,
    pub background: Rgb,
    pub foreground: Rgb,
    pub cursor: Rgb,
    /// The selection color already blended over the background.
    pub selection: Rgb,
    pub ansi: [Rgb; 16],
    pub light: bool,
    /// Overrides for colors the terminal palette alone would not make readable.
    pub accent: Option<Rgb>,
    pub text: Option<Rgb>,
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

#[rustfmt::skip]
pub const THEMES: [Theme; 13] = [
    Theme {
        id: "midnight",
        name: "Midnight",
        background: hex(0x14181f),
        foreground: hex(0xe0e6ee),
        cursor: hex(0x8db4ff),
        selection: hex(0x2e394e),
        ansi: palette([0x39414e, 0xe58a91, 0xa5c9a1, 0xdec38d, 0x8db4ff, 0xbdacd8, 0x92c4cc, 0xe0e6ee, 0x7d899a, 0xf4a2a8, 0xb9dab5, 0xefd5a5, 0xaccaff, 0xd4c3ed, 0xaedce3, 0xf5f7fa]),
        light: false,
        accent: None,
        text: None,
    },
    Theme {
        id: "graphite",
        name: "Graphite",
        background: hex(0x1a1a1a),
        foreground: hex(0xd6d6d6),
        cursor: hex(0xf0f0f0),
        selection: hex(0x454545),
        ansi: palette([0x3a3a3a, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xd6d6d6, 0x7a7a7a, 0xef8a92, 0xb0d898, 0xf0d39b, 0x86c3f7, 0xd899ea, 0x7accd6, 0xffffff]),
        light: false,
        accent: None,
        text: None,
    },
    Theme {
        id: "aurora",
        name: "Aurora",
        background: hex(0x0f1a1c),
        foreground: hex(0xd3e6e2),
        cursor: hex(0x6fe0c2),
        selection: hex(0x22413d),
        ansi: palette([0x26393c, 0xf28b82, 0x6fe0c2, 0xf4d58d, 0x7cb7ff, 0xd4a5ff, 0x7fdbe6, 0xd3e6e2, 0x5f7b7f, 0xffa59e, 0x93f0d6, 0xffe4a8, 0xa0cbff, 0xe3c2ff, 0xa3ebf2, 0xf2fbf9]),
        light: false,
        accent: None,
        text: None,
    },
    Theme {
        id: "ember",
        name: "Ember",
        background: hex(0x1d1714),
        foreground: hex(0xecdccf),
        cursor: hex(0xff9e64),
        selection: hex(0x483023),
        ansi: palette([0x3d322c, 0xf47c6b, 0xb5c97a, 0xf2c66d, 0x88aee0, 0xd79ac4, 0x8ec9b9, 0xecdccf, 0x857065, 0xff9a8b, 0xcadc95, 0xffd98c, 0xa7c5ee, 0xe8b5d7, 0xaadccf, 0xfff6ee]),
        light: false,
        accent: None,
        text: None,
    },
    Theme {
        id: "daylight",
        name: "Daylight",
        background: hex(0xfbfbfa),
        foreground: hex(0x2b3038),
        cursor: hex(0x2f6fdb),
        selection: hex(0xd6e2f4),
        ansi: palette([0x2b3038, 0xc4373f, 0x2f7d3a, 0x94660b, 0x2f6fdb, 0x8a45b5, 0x167a86, 0xc9ced6, 0x6b7380, 0xdc4a52, 0x3a9447, 0xb07c12, 0x4a86ec, 0xa35acf, 0x1f93a1, 0xffffff]),
        light: true,
        accent: None,
        text: None,
    },
    Theme {
        id: "tokyoNight",
        name: "Tokyo Night",
        background: hex(0x1a1b26),
        foreground: hex(0xc0caf5),
        cursor: hex(0xc0caf5),
        selection: hex(0x323d5a),
        ansi: palette([0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6, 0x414868, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5]),
        light: false,
        accent: Some(hex(0x7aa2f7)),
        text: None,
    },
    Theme {
        id: "dracula",
        name: "Dracula",
        background: hex(0x282a36),
        foreground: hex(0xf8f8f2),
        cursor: hex(0xf8f8f2),
        selection: hex(0x44475a),
        ansi: palette([0x21222c, 0xff5555, 0x50fa7b, 0xf1fa8c, 0xbd93f9, 0xff79c6, 0x8be9fd, 0xf8f8f2, 0x6272a4, 0xff6e6e, 0x69ff94, 0xffffa5, 0xd6acff, 0xff92df, 0xa4ffff, 0xffffff]),
        light: false,
        accent: Some(hex(0xbd93f9)),
        text: None,
    },
    Theme {
        id: "nord",
        name: "Nord",
        background: hex(0x2e3440),
        foreground: hex(0xd8dee9),
        cursor: hex(0xd8dee9),
        selection: hex(0x455764),
        ansi: palette([0x3b4252, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x88c0d0, 0xe5e9f0, 0x4c566a, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x8fbcbb, 0xeceff4]),
        light: false,
        accent: Some(hex(0x88c0d0)),
        text: None,
    },
    Theme {
        id: "catppuccin",
        name: "Catppuccin Mocha",
        background: hex(0x1e1e2e),
        foreground: hex(0xcdd6f4),
        cursor: hex(0xf5e0dc),
        selection: hex(0x3b3d4f),
        ansi: palette([0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de, 0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8]),
        light: false,
        accent: Some(hex(0xcba6f7)),
        text: None,
    },
    Theme {
        id: "gruvbox",
        name: "Gruvbox Dark",
        background: hex(0x282828),
        foreground: hex(0xebdbb2),
        cursor: hex(0xebdbb2),
        selection: hex(0x4d4a42),
        ansi: palette([0x282828, 0xcc241d, 0x98971a, 0xd79921, 0x458588, 0xb16286, 0x689d6a, 0xa89984, 0x928374, 0xfb4934, 0xb8bb26, 0xfabd2f, 0x83a598, 0xd3869b, 0x8ec07c, 0xebdbb2]),
        light: false,
        accent: Some(hex(0xfabd2f)),
        text: None,
    },
    Theme {
        id: "solarizedDark",
        name: "Solarized Dark",
        background: hex(0x002b36),
        foreground: hex(0x839496),
        cursor: hex(0x93a1a1),
        selection: hex(0x1c414a),
        ansi: palette([0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5, 0x586e75, 0xcb4b16, 0x859900, 0xb58900, 0x268bd2, 0x6c71c4, 0x2aa198, 0xfdf6e3]),
        light: false,
        accent: Some(hex(0x2aa198)),
        text: Some(hex(0xc5ccc9)),
    },
    Theme {
        id: "solarizedLight",
        name: "Solarized Light",
        background: hex(0xfdf6e3),
        foreground: hex(0x586e75),
        cursor: hex(0x268bd2),
        selection: hex(0xd6e3e0),
        ansi: palette([0x073642, 0xdc322f, 0x6c7a00, 0x946f00, 0x268bd2, 0xd33682, 0x1f807a, 0x93a1a1, 0x586e75, 0xcb4b16, 0x6c7a00, 0x946f00, 0x268bd2, 0x6c71c4, 0x1f807a, 0x073642]),
        light: true,
        accent: Some(hex(0x1c6da8)),
        text: Some(hex(0x2f4a53)),
    },
    Theme {
        id: "github",
        name: "GitHub Light",
        background: hex(0xffffff),
        foreground: hex(0x1f2328),
        cursor: hex(0x0969da),
        selection: hex(0xd3e4f8),
        ansi: palette([0x24292f, 0xcf222e, 0x116329, 0x7d4e00, 0x0969da, 0x8250df, 0x1b7c83, 0x6e7781, 0x57606a, 0xa40e26, 0x1a7f37, 0x633c01, 0x218bff, 0xa475f9, 0x3192aa, 0x8c959f]),
        light: true,
        accent: None,
        text: None,
    },
];

pub fn by_id(id: &str) -> &'static Theme {
    THEMES.iter().find(|theme| theme.id == id).unwrap_or(&THEMES[0])
}

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

pub fn mix(from: Rgb, to: Rgb, amount: f32) -> Rgb {
    let channel = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * amount).round() as u8;
    Rgb { r: channel(from.r, to.r), g: channel(from.g, to.g), b: channel(from.b, to.b) }
}

fn luminance(color: Rgb) -> f32 {
    let linear = |value: u8| {
        let unit = value as f32 / 255.0;
        if unit <= 0.04045 { unit / 12.92 } else { ((unit + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
}

pub fn contrast(first: Rgb, second: Rgb) -> f32 {
    let (a, b) = (luminance(first), luminance(second));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// The interface colors derived from a scheme, as `themeTokens` in the desktop app.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub bg: Rgb,
    pub surface: Rgb,
    pub surface_soft: Rgb,
    pub surface_hover: Rgb,
    pub well: Rgb,
    pub text: Rgb,
    pub text_soft: Rgb,
    pub muted: Rgb,
    pub faint: Rgb,
    pub subtle: Rgb,
    pub accent: Rgb,
    pub on_accent: Rgb,
    pub danger: Rgb,
    pub warning: Rgb,
    /// Borders: the ink color at low opacity, as (rgb, alpha).
    pub ink: Rgb,
}

impl Theme {
    pub fn tokens(&self) -> Tokens {
        let background = self.background;
        let text = self.text.unwrap_or(self.foreground);
        let accent = self.accent.unwrap_or(self.cursor);
        let light = self.light;
        let white = hex(0xffffff);
        let dark = hex(0x10151c);
        Tokens {
            bg: background,
            surface: mix(background, text, if light { 0.035 } else { 0.055 }),
            surface_soft: mix(background, text, if light { 0.02 } else { 0.03 }),
            surface_hover: mix(background, text, if light { 0.08 } else { 0.11 }),
            well: mix(background, if light { white } else { hex(0) }, if light { 0.6 } else { 0.16 }),
            text,
            text_soft: mix(text, background, 0.14),
            muted: mix(text, background, if light { 0.24 } else { 0.34 }),
            faint: mix(text, background, if light { 0.38 } else { 0.46 }),
            subtle: mix(text, background, 0.6),
            accent,
            on_accent: if contrast(accent, white) >= contrast(accent, dark) { white } else { dark },
            danger: self.ansi[1],
            warning: self.ansi[3],
            ink: if light { hex(0) } else { white },
        }
    }

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
        let dim = |color: Rgb| mix(color, self.background, 0.34);
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

    #[test]
    fn every_scheme_is_present_and_text_on_accent_stays_readable() {
        assert_eq!(by_id("nord").name, "Nord");
        assert_eq!(by_id("missing").id, "midnight");
        for theme in THEMES {
            let tokens = theme.tokens();
            assert!(contrast(tokens.accent, tokens.on_accent) >= 3.0, "{}", theme.id);
        }
    }
}

// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Semantic role mappings adapted from upstream palettes. See
//! assets/highlight-licenses/NOTICE.md for sources, changes and license texts.

use super::Role;
use crate::theme::{Theme, contrast};
use alacritty_terminal::vte::ansi::Rgb;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Style {
    Auto,
    Dracula, TokyoNight, Catppuccin, Nord, Gruvbox, Solarized, One, RosePine,
    #[default]
    #[serde(other)]
    Moba,
}

impl Style {
    pub const ALL: [Self; 10] = [Self::Auto, Self::Moba, Self::Dracula, Self::TokyoNight,
        Self::Catppuccin, Self::Nord, Self::Gruvbox, Self::Solarized, Self::One, Self::RosePine];

    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "跟随界面主题", Self::Moba => "Moba（经典）", Self::Dracula => "Dracula",
            Self::TokyoNight => "Tokyo Night", Self::Catppuccin => "Catppuccin", Self::Nord => "Nord",
            Self::Gruvbox => "Gruvbox", Self::Solarized => "Solarized", Self::One => "One Dark / Light",
            Self::RosePine => "Rosé Pine",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Auto => "跟随界面主题选择同名高亮；其他主题使用经典配色",
            Self::Moba => "蓝色字段、绿色字符串、金色数字，鲜明易读",
            Self::Dracula => "粉色字段、黄色字符串、紫色数字，色彩鲜明",
            Self::TokyoNight => "蓝紫与青绿，夜间日志清晰醒目",
            Self::Catppuccin => "柔和的蓝、绿与桃色，深色 Mocha / 浅色 Latte",
            Self::Nord => "冰蓝与柔和绿，低饱和的北欧风格",
            Self::Gruvbox => "复古暖色，青绿字段与金橙色数字",
            Self::Solarized => "克制的蓝绿与橙色，适合长时间阅读",
            Self::One => "红色字段、绿色字符串，熟悉的编辑器配色",
            Self::RosePine => "玫瑰、鸢尾紫与金色，柔和温暖",
        }
    }

    pub fn resolved(self, theme: &Theme) -> Self {
        if self != Self::Auto { return self; }
        match theme.id {
            "dracula" => Self::Dracula, "tokyoNight" => Self::TokyoNight,
            "catppuccin" => Self::Catppuccin, "nord" => Self::Nord,
            "gruvbox" => Self::Gruvbox, "solarizedDark" | "solarizedLight" => Self::Solarized,
            "graphite" => Self::One,
            _ => Self::Moba,
        }
    }
}

pub struct Palette([Rgb; 11]);

impl Palette {
    pub fn new(style: Style, theme: &Theme) -> Self {
        // Key, string, number, literal, address, path, time, error, warning, success, info.
        let values = match (style.resolved(theme), theme.light) {
            (Style::Moba | Style::Auto, false) => [0x79b8ff,0x98d99b,0xf0cb7c,0xc5a3ef,0x69d5d0,0x69d5d0,0xa9afff,0xff7587,0xffb66b,0x98d99b,0x79b8ff],
            (Style::Moba | Style::Auto, true) => [0x1e5da9,0x23743b,0x946000,0x8047a9,0x007478,0x007478,0x6250a1,0xba263e,0x995000,0x23743b,0x1e5da9],
            (Style::Dracula, false) => [0xff79c6,0xf1fa8c,0xbd93f9,0xbd93f9,0x8be9fd,0x50fa7b,0xffb86c,0xff5555,0xffb86c,0x50fa7b,0x8be9fd],
            (Style::Dracula, true) => [0xa3144d,0x846e15,0x644ac9,0x644ac9,0x036a96,0x14710a,0xa34d14,0xcb3a2a,0xa34d14,0x14710a,0x036a96],
            (Style::TokyoNight, false) => [0x7aa2f7,0x9ece6a,0xff9e64,0xbb9af7,0x7dcfff,0x73daca,0x9d7cd8,0xf7768e,0xe0af68,0x9ece6a,0x7aa2f7],
            (Style::TokyoNight, true) => [0x2e7de9,0x587539,0xb15c00,0x9854f1,0x007197,0x118c74,0x7847bd,0xf52a65,0x8c6c3e,0x587539,0x2e7de9],
            (Style::Catppuccin, false) => [0x89b4fa,0xa6e3a1,0xfab387,0xcba6f7,0x74c7ec,0x94e2d5,0xb4befe,0xf38ba8,0xf9e2af,0xa6e3a1,0x89b4fa],
            (Style::Catppuccin, true) => [0x1e66f5,0x40a02b,0xfe640b,0x8839ef,0x209fb5,0x179299,0x7287fd,0xd20f39,0xdf8e1d,0x40a02b,0x1e66f5],
            (Style::Nord, _) => [0x88c0d0,0xa3be8c,0xb48ead,0x81a1c1,0x8fbcbb,0x8fbcbb,0x81a1c1,0xbf616a,0xebcb8b,0xa3be8c,0x88c0d0],
            (Style::Gruvbox, false) => [0x83a598,0xb8bb26,0xd3869b,0xd3869b,0x8ec07c,0x8ec07c,0xfabd2f,0xfb4934,0xfe8019,0xb8bb26,0x83a598],
            (Style::Gruvbox, true) => [0x076678,0x79740e,0x8f3f71,0x8f3f71,0x427b58,0x427b58,0xb57614,0x9d0006,0xaf3a03,0x79740e,0x076678],
            (Style::Solarized, _) => [0x268bd2,0x859900,0xcb4b16,0xd33682,0x2aa198,0x2aa198,0x6c71c4,0xdc322f,0xb58900,0x859900,0x268bd2],
            (Style::One, false) => [0xe06c75,0x98c379,0xd19a66,0xc678dd,0x56b6c2,0x61afef,0xc678dd,0xe06c75,0xe5c07b,0x98c379,0x61afef],
            (Style::One, true) => [0xe45649,0x50a14f,0x986801,0xa626a4,0x0184bc,0x4078f2,0xa626a4,0xe45649,0xc18401,0x50a14f,0x4078f2],
            (Style::RosePine, false) => [0xc4a7e7,0xebbcba,0xf6c177,0xeb6f92,0x9ccfd8,0x95b1ac,0xc4a7e7,0xeb6f92,0xf6c177,0x95b1ac,0x9ccfd8],
            (Style::RosePine, true) => [0x907aa9,0xd7827e,0xea9d34,0xb4637a,0x56949f,0x6d8f89,0x907aa9,0xb4637a,0xea9d34,0x6d8f89,0x286983],
        };
        Self(values.map(|value| readable(Rgb {r:(value>>16) as u8,g:(value>>8) as u8,b:value as u8}, theme)))
    }

    pub fn color(&self, role: Role) -> Option<Rgb> {
        let index = match role {
            Role::Plain => return None,
            Role::Key => 0, Role::String => 1, Role::Number => 2, Role::Literal => 3,
            Role::Address => 4, Role::Path => 5, Role::Time => 6, Role::Error => 7,
            Role::Warning => 8, Role::Success => 9, Role::Info => 10,
        };
        Some(self.0[index])
    }
}

// Keep upstream colors when readable, otherwise move minimally toward black or
// white. This also makes Nord's dark palette usable on light backgrounds.
fn readable(color: Rgb, theme: &Theme) -> Rgb {
    if contrast(color,theme.background) >= 4.5 { return color; }
    let target = if theme.light {0} else {255};
    for step in 1..=32 {
        let blend = |v: u8| ((v as u32*(32-step)+target*step)/32) as u8;
        let adjusted = Rgb {r:blend(color.r),g:blend(color.g),b:blend(color.b)};
        if contrast(adjusted,theme.background) >= 4.5 { return adjusted; }
    }
    Rgb {r:target as u8,g:target as u8,b:target as u8}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::THEMES;

    #[test]
    fn every_style_is_readable_on_every_bundled_background() {
        for style in Style::ALL {
            for theme in &THEMES {
                for color in Palette::new(style,theme).0 {
                    assert!(contrast(color,theme.background)>=4.5,"{} on {}",style.name(),theme.name);
                }
            }
        }
    }

    #[test]
    fn automatic_style_tracks_the_theme_and_falls_back_to_classic() {
        assert_eq!(Style::Auto.resolved(crate::theme::by_id("midnight")),Style::Moba);
        let auto = |theme| Palette::new(Style::Auto,crate::theme::by_id(theme)).color(Role::Key);
        assert_ne!(auto("dracula"),auto("gruvbox"));
        assert_ne!(auto("solarizedDark"),auto("solarizedLight"));
        let colors: Vec<_> = Style::ALL.into_iter().filter(|s| *s != Style::Auto).map(|s| Palette::new(s,&THEMES[0]).color(Role::Key)).collect();
        let mut distinct = colors.clone(); distinct.sort_by_key(|c|c.map(|c|(c.r,c.g,c.b)));distinct.dedup();
        assert!(distinct.len()>=8,"styles must offer visibly different key colors");
    }
}

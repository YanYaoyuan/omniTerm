// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Terminal settings, with the desktop app's defaults and limits. Unknown or out-of-range
//! values from disk fall back to the default for that one field instead of losing the rest.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FontFamily {
    System,
    Jetbrains,
    Fira,
    Cascadia,
    Sarasa,
}

impl FontFamily {
    pub const ALL: [FontFamily; 5] = [Self::System, Self::Jetbrains, Self::Fira, Self::Cascadia, Self::Sarasa];

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "系统等宽字体",
            Self::Jetbrains => "JetBrains Mono",
            Self::Fira => "Fira Code",
            Self::Cascadia => "Cascadia Code",
            Self::Sarasa => "Sarasa Mono（中英等宽）",
        }
    }

    /// Installed family names to try, in order; the system monospace font comes last.
    pub fn candidates(self) -> &'static [&'static str] {
        match self {
            Self::System => &[],
            Self::Jetbrains => &["JetBrains Mono"],
            Self::Fira => &["Fira Code", "FiraCode"],
            Self::Cascadia => &["Cascadia Code", "Cascadia Mono"],
            Self::Sarasa => &["Sarasa Mono SC", "Sarasa Term SC", "Sarasa Mono"],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CursorStyle {
    Bar,
    Block,
    Underline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackgroundMode {
    Opaque,
    Translucent,
    Transparent,
}

impl BackgroundMode {
    pub const ALL: [BackgroundMode; 3] = [Self::Opaque, Self::Translucent, Self::Transparent];
    pub fn label(self) -> &'static str {
        match self {
            Self::Opaque => "不透明",
            Self::Translucent => "半透明",
            Self::Transparent => "透明",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageFit {
    Cover,
    Contain,
    Tile,
}

impl ImageFit {
    pub const ALL: [ImageFit; 3] = [Self::Cover, Self::Contain, Self::Tile];
    pub fn label(self) -> &'static str {
        match self {
            Self::Cover => "铺满",
            Self::Contain => "适应",
            Self::Tile => "平铺",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BackgroundImage {
    /// A copy in the data directory, so the original can move or disappear.
    pub path: PathBuf,
    pub name: String,
}

pub const LINE_HEIGHTS: [f32; 5] = [1.0, 1.1, 1.22, 1.35, 1.5];
pub const SCROLLBACKS: [usize; 5] = [1_000, 5_000, 10_000, 50_000, 100_000];
pub const MIN_FONT: f32 = 9.0;
pub const MAX_FONT: f32 = 28.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub font_size: f32,
    pub font_family: FontFamily,
    pub line_height: f32,
    pub scrollback: usize,
    pub cursor_blink: bool,
    pub cursor_style: CursorStyle,
    pub theme: String,
    pub copy_on_select: bool,
    pub option_as_meta: bool,
    pub dim_inactive: bool,
    pub restore_workspace: bool,
    pub remote_panels: bool,
    pub ssh_config_prompt: bool,
    pub background_mode: BackgroundMode,
    pub background_opacity: f32,
    pub background_image: Option<BackgroundImage>,
    pub background_image_opacity: f32,
    pub background_image_fit: ImageFit,
    /// One veil under every panel and terminal when something shows through the window.
    pub surface_opacity: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            font_size: 13.0,
            font_family: FontFamily::System,
            line_height: 1.22,
            scrollback: 10_000,
            cursor_blink: true,
            cursor_style: CursorStyle::Bar,
            theme: "midnight".into(),
            copy_on_select: false,
            option_as_meta: false,
            dim_inactive: true,
            restore_workspace: true,
            remote_panels: true,
            ssh_config_prompt: true,
            background_mode: BackgroundMode::Opaque,
            background_opacity: 0.8,
            background_image: None,
            background_image_opacity: 0.3,
            background_image_fit: ImageFit::Cover,
            surface_opacity: 0.5,
        }
    }
}

impl Settings {
    fn path() -> PathBuf {
        super::paths::file("native-settings.json")
    }

    pub fn load() -> Self {
        std::fs::read(Self::path()).ok().and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).ok()).unwrap_or_default().clamped()
    }

    pub fn save(&self) -> Result<(), String> {
        let json = serde_json::to_vec_pretty(self).map_err(|error| error.to_string())?;
        super::paths::write_atomically(&Self::path(), &json).map_err(|error| format!("无法保存终端设置，本次会话中仍然生效：{error}"))
    }

    /// Every value within the range the interface offers.
    pub fn clamped(mut self) -> Self {
        let fallback = Self::default();
        let finite = |value: f32, fallback: f32| if value.is_finite() { value } else { fallback };
        self.font_size = finite(self.font_size, fallback.font_size).clamp(MIN_FONT, MAX_FONT).round();
        if !LINE_HEIGHTS.iter().any(|height| (height - self.line_height).abs() < 1e-3) {
            self.line_height = fallback.line_height;
        }
        if !SCROLLBACKS.contains(&self.scrollback) {
            self.scrollback = fallback.scrollback;
        }
        if !crate::theme::THEMES.iter().any(|theme| theme.id == self.theme) {
            self.theme = fallback.theme;
        }
        self.background_opacity = finite(self.background_opacity, fallback.background_opacity).clamp(0.1, 0.95);
        self.background_image_opacity = finite(self.background_image_opacity, fallback.background_image_opacity).clamp(0.05, 1.0);
        self.surface_opacity = finite(self.surface_opacity, fallback.surface_opacity).clamp(0.0, 0.95);
        if self.background_image.as_ref().is_some_and(|image| !image.path.is_file()) {
            self.background_image = None;
        }
        self
    }

    pub fn transparent_window(&self) -> bool {
        self.background_mode != BackgroundMode::Opaque
    }
}

/// What the window shows through: the stack behind the interface, bottom to top, is the
/// desktop (only in a transparent window), the theme color at `base`, then the image. When
/// `clear`, every panel and terminal sits on one veil of the theme color at `veil`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scene {
    pub base: f32,
    pub image: bool,
    pub clear: bool,
    pub veil: f32,
}

pub fn scene(settings: &Settings, window_transparent: bool) -> Scene {
    let base = match settings.background_mode {
        _ if !window_transparent => 1.0,
        BackgroundMode::Opaque => 1.0,
        BackgroundMode::Transparent => 0.0,
        BackgroundMode::Translucent => settings.background_opacity,
    };
    let image = settings.background_image.is_some();
    let clear = base < 1.0 || image;
    Scene { base, image, clear, veil: if clear { settings.surface_opacity } else { 1.0 } }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_values_outside_the_offered_range_fall_back_one_by_one() {
        let settings: Settings = serde_json::from_str(r#"{"fontSize": 99, "lineHeight": 7, "theme": "nord", "scrollback": 3, "unknown": 1}"#).unwrap();
        let settings = settings.clamped();
        assert_eq!(settings.font_size, MAX_FONT);
        assert_eq!(settings.line_height, 1.22);
        assert_eq!(settings.scrollback, 10_000);
        assert_eq!(settings.theme, "nord");
    }

    #[test]
    fn the_veil_applies_only_when_something_shows_through() {
        let mut settings = Settings::default();
        assert_eq!(scene(&settings, true), Scene { base: 1.0, image: false, clear: false, veil: 1.0 });
        settings.background_mode = BackgroundMode::Translucent;
        settings.surface_opacity = 0.3;
        // A translucent choice waits for the transparent window that needs a restart.
        assert!(!scene(&settings, false).clear);
        let clear = scene(&settings, true);
        assert_eq!((clear.base, clear.clear, clear.veil), (0.8, true, 0.3));
    }
}

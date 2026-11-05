// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! The look of the interface: fonts, colors derived from the terminal theme like the desktop
//! app's CSS tokens, and the small widgets it is built from.

use crate::theme::{Theme, Tokens};
use alacritty_terminal::vte::ansi::Rgb;
use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Response, RichText, Sense, Stroke, Ui, Vec2};
use std::sync::Arc;

pub const ICONS: &str = "phosphor";

pub fn color(rgb: Rgb) -> Color32 {
    Color32::from_rgb(rgb.r, rgb.g, rgb.b)
}

pub fn alpha(rgb: Rgb, alpha: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(rgb.r, rgb.g, rgb.b, (alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// The colors the interface is drawn with in one frame.
#[derive(Clone, Copy)]
pub struct Palette {
    pub tokens: Tokens,
    /// Panel fill: the surface color, or the shared veil when the backdrop shows through.
    pub panel: Color32,
    pub panel_soft: Color32,
    pub panel_bg: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub light: bool,
}

impl Palette {
    pub fn new(theme: &Theme, veil: Option<f32>) -> Self {
        let tokens = theme.tokens();
        let (panel, panel_soft, panel_bg) = match veil {
            Some(amount) => {
                let veil = alpha(tokens.bg, amount);
                (veil, Color32::TRANSPARENT, Color32::TRANSPARENT)
            }
            None => (color(tokens.surface), color(tokens.surface_soft), color(tokens.bg)),
        };
        Self {
            tokens,
            panel,
            panel_soft,
            panel_bg,
            border: alpha(tokens.ink, 0.05),
            border_strong: alpha(tokens.ink, 0.08),
            light: theme.light,
        }
    }

    pub fn text(&self) -> Color32 {
        color(self.tokens.text)
    }
    pub fn text_soft(&self) -> Color32 {
        color(self.tokens.text_soft)
    }
    pub fn muted(&self) -> Color32 {
        color(self.tokens.muted)
    }
    pub fn faint(&self) -> Color32 {
        color(self.tokens.faint)
    }
    pub fn accent(&self) -> Color32 {
        color(self.tokens.accent)
    }
    pub fn danger(&self) -> Color32 {
        color(self.tokens.danger)
    }
    pub fn hover(&self) -> Color32 {
        color(self.tokens.surface_hover)
    }
    pub fn surface(&self) -> Color32 {
        color(self.tokens.surface)
    }
}

/// Fonts: the platform UI font with CJK fallback, monospace for paths and keys, and Phosphor
/// icons as their own family.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(ICONS.into(), Arc::new(egui::FontData::from_static(include_bytes!("../../assets/Phosphor.ttf"))));
    fonts.families.insert(FontFamily::Name(ICONS.into()), vec![ICONS.into()]);
    for (name, path) in UI_FONTS.iter().chain(CJK_FONTS) {
        if fonts.font_data.contains_key(*name) {
            continue;
        }
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert((*name).into(), Arc::new(egui::FontData::from_owned(bytes)));
            let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
            if UI_FONTS.iter().any(|(ui, _)| ui == name) {
                proportional.insert(0, (*name).into());
            } else {
                proportional.push((*name).into());
                fonts.families.entry(FontFamily::Monospace).or_default().push((*name).into());
            }
        }
    }
    // Icons also appear inside button and label text, so every family falls back to them.
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts.families.entry(family).or_default().push(ICONS.into());
    }
    ctx.set_fonts(fonts);
}

/// The system interface font, first one found, so text looks native.
const UI_FONTS: &[(&str, &str)] = &[
    ("segoe", "C:\\Windows\\Fonts\\segoeui.ttf"),
    ("sf", "/System/Library/Fonts/SFNS.ttf"),
    ("inter", "/usr/share/fonts/truetype/inter/Inter-Regular.ttf"),
    ("noto-sans", "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf"),
];

const CJK_FONTS: &[(&str, &str)] = &[
    ("cjk", "C:\\Windows\\Fonts\\msyh.ttc"),
    ("cjk", "/System/Library/Fonts/PingFang.ttc"),
    ("cjk", "/System/Library/Fonts/STHeiti Light.ttc"),
    ("cjk", "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"),
    ("cjk", "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc"),
    ("cjk", "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc"),
    ("cjk", "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc"),
];

pub fn apply_visuals(ctx: &egui::Context, palette: &Palette) {
    let mut visuals = if palette.light { egui::Visuals::light() } else { egui::Visuals::dark() };
    let t = &palette.tokens;
    visuals.override_text_color = Some(color(t.text_soft));
    visuals.panel_fill = Color32::TRANSPARENT;
    visuals.window_fill = color(t.surface);
    visuals.window_stroke = Stroke::new(1.0, palette.border_strong);
    visuals.window_corner_radius = CornerRadius::same(10);
    visuals.menu_corner_radius = CornerRadius::same(7);
    visuals.extreme_bg_color = color(t.well);
    visuals.faint_bg_color = color(t.surface_soft);
    visuals.selection.bg_fill = alpha(t.accent, 0.28);
    visuals.selection.stroke = Stroke::new(1.0, color(t.accent));
    visuals.hyperlink_color = color(t.accent);
    for (state, fill) in [
        (&mut visuals.widgets.noninteractive, Color32::TRANSPARENT),
        (&mut visuals.widgets.inactive, alpha(t.ink, 0.03)),
        (&mut visuals.widgets.hovered, color(t.surface_hover)),
        (&mut visuals.widgets.active, color(t.surface_hover)),
        (&mut visuals.widgets.open, color(t.surface_hover)),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.corner_radius = CornerRadius::same(5);
        state.bg_stroke = Stroke::new(1.0, palette.border);
        state.fg_stroke = Stroke::new(1.0, color(t.text_soft));
    }
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, color(t.muted));
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.border);
    ctx.set_visuals(visuals);
    ctx.global_style_mut(|style| {
        style.spacing.item_spacing = Vec2::new(6.0, 4.0);
        style.spacing.button_padding = Vec2::new(8.0, 4.0);
        style.spacing.interact_size.y = 24.0;
        style.text_styles.insert(egui::TextStyle::Body, FontId::proportional(12.0));
        style.text_styles.insert(egui::TextStyle::Button, FontId::proportional(12.0));
        style.text_styles.insert(egui::TextStyle::Small, FontId::proportional(10.0));
        style.text_styles.insert(egui::TextStyle::Monospace, FontId::monospace(11.0));
        style.text_styles.insert(egui::TextStyle::Heading, FontId::proportional(16.0));
    });
}

pub fn icon(glyph: &str, size: f32) -> RichText {
    RichText::new(glyph).family(FontFamily::Name(ICONS.into())).size(size)
}

/// A square, borderless button with an icon and a tooltip.
pub fn icon_button(ui: &mut Ui, glyph: &str, tooltip: &str, enabled: bool) -> Response {
    let palette = palette(ui);
    let size = Vec2::splat(25.0);
    let (rect, response) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    if response.hovered() && enabled {
        ui.painter().rect_filled(rect, 5.0, palette.hover());
    }
    let color = if !enabled { palette.faint().gamma_multiply(0.6) } else if response.hovered() { palette.text_soft() } else { palette.muted() };
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, glyph, FontId::new(15.0, FontFamily::Name(ICONS.into())), color);
    let response = response.on_hover_text(tooltip);
    if enabled { response } else { response.on_hover_cursor(egui::CursorIcon::NotAllowed) }
}

/// An on/off switch, like the desktop app's.
pub fn toggle(ui: &mut Ui, value: &mut bool, label: &str) -> Response {
    let palette = palette(ui);
    let (rect, mut response) = ui.allocate_exact_size(Vec2::new(32.0, 18.0), Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    let on = ui.ctx().animate_bool_responsive(response.id, *value);
    let track = if *value { palette.accent() } else { alpha(palette.tokens.ink, 0.12) };
    ui.painter().rect_filled(rect, 9.0, track);
    let x = egui::lerp((rect.left() + 9.0)..=(rect.right() - 9.0), on);
    ui.painter().circle_filled(egui::pos2(x, rect.center().y), 7.0, if *value { color(palette.tokens.on_accent) } else { palette.text_soft() });
    response.on_hover_text(label)
}

/// A row of mutually exclusive choices.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let palette = palette(ui);
    let mut changed = false;
    egui::Frame::new().fill(alpha(palette.tokens.ink, 0.04)).corner_radius(6).inner_margin(2).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.horizontal(|ui| {
            for (option, label) in options {
                let selected = *value == *option;
                let text = RichText::new(*label).size(11.0).color(if selected { palette.text() } else { palette.muted() });
                let button = egui::Button::new(text).fill(if selected { palette.hover() } else { Color32::TRANSPARENT }).stroke(Stroke::NONE).corner_radius(5);
                if ui.add(button).clicked() && !selected {
                    *value = *option;
                    changed = true;
                }
            }
        });
    });
    changed
}

/// A percentage slider in steps of 5, like the desktop app's range inputs.
pub fn percent_slider(ui: &mut Ui, value: &mut f32, minimum: f32, maximum: f32) -> bool {
    let mut percent = (*value * 100.0).round();
    let response = ui.add(egui::Slider::new(&mut percent, minimum..=maximum).step_by(5.0).suffix("%").show_value(true));
    if response.changed() {
        *value = percent / 100.0;
    }
    response.changed()
}

pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    let palette = palette(ui);
    ui.add(egui::Button::new(RichText::new(text).color(color(palette.tokens.on_accent)).strong()).fill(palette.accent()).stroke(Stroke::NONE).corner_radius(6).min_size(Vec2::new(0.0, 28.0)))
}

pub fn secondary_button(ui: &mut Ui, text: &str) -> Response {
    let palette = palette(ui);
    ui.add(egui::Button::new(RichText::new(text).color(palette.text_soft())).fill(alpha(palette.tokens.ink, 0.05)).stroke(Stroke::new(1.0, palette.border_strong)).corner_radius(6).min_size(Vec2::new(0.0, 28.0)))
}

pub fn ghost_button(ui: &mut Ui, text: &str) -> Response {
    let palette = palette(ui);
    ui.add(egui::Button::new(RichText::new(text).color(palette.muted())).fill(Color32::TRANSPARENT).stroke(Stroke::NONE).corner_radius(6).min_size(Vec2::new(0.0, 28.0)))
}

pub fn danger_button(ui: &mut Ui, text: &str) -> Response {
    let palette = palette(ui);
    ui.add(egui::Button::new(RichText::new(text).color(Color32::WHITE).strong()).fill(palette.danger()).stroke(Stroke::NONE).corner_radius(6).min_size(Vec2::new(0.0, 28.0)))
}

/// A keyboard shortcut chip.
pub fn kbd(ui: &mut Ui, text: &str) {
    let palette = palette(ui);
    egui::Frame::new().fill(alpha(palette.tokens.ink, 0.06)).stroke(Stroke::new(1.0, palette.border_strong)).corner_radius(4).inner_margin(egui::Margin::symmetric(5, 1)).show(ui, |ui| {
        ui.label(RichText::new(text).monospace().size(10.0).color(palette.muted()));
    });
}

/// The palette is stored in egui's memory each frame so any widget can reach it.
pub fn palette(ui: &Ui) -> Palette {
    palette_of(ui.ctx())
}

pub fn palette_of(ctx: &egui::Context) -> Palette {
    ctx.data(|data| data.get_temp::<Palette>(palette_id())).unwrap_or_else(|| Palette::new(&crate::theme::THEMES[0], None))
}

pub fn set_palette(ctx: &egui::Context, palette: Palette) {
    ctx.data_mut(|data| data.insert_temp(palette_id(), palette));
}

fn palette_id() -> egui::Id {
    egui::Id::unique("omniterm-palette")
}

// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! One selector and the same short log sample for comparing highlight styles.

use crate::core::settings::Settings;
use crate::highlight::{self, Palette, Role, Style};
use eframe::egui::{self, Color32, FontId, TextFormat, Ui};

pub fn choices(ui: &mut Ui, selected: &mut Style) {
    for style in Style::ALL {
        ui.selectable_value(selected,style,style.name()).on_hover_text(style.description());
    }
}

pub fn selector(ui: &mut Ui, settings: &mut Settings) {
    ui.add_enabled_ui(settings.syntax_highlighting, |ui| {
        egui::ComboBox::from_id_salt("highlight-style").selected_text(settings.highlight_style.name()).width(170.0).show_ui(ui,|ui| {
            choices(ui,&mut settings.highlight_style);
        });
    });
}

pub fn preview(ui: &mut Ui, settings: &Settings) {
    let theme = crate::theme::by_id(&settings.theme);
    let palette = Palette::new(settings.highlight_style,theme);
    let sample = "{\"loss\": 0.126, \"lr\": 7.6e-06, \"asset\": \"worker-01\", \"ready\": true}\n12:13:30 INFO 192.0.2.10:22 /var/log/app.log  WARNING retry=2  ERROR failed  OK";
    let chars: Vec<_> = sample.chars().collect();
    let roles = if settings.syntax_highlighting {highlight::classify(&chars)} else {vec![Role::Plain;chars.len()]};
    let rgb = |c: alacritty_terminal::vte::ansi::Rgb| Color32::from_rgb(c.r,c.g,c.b);
    let mut job = egui::text::LayoutJob::default();
    let mut start = 0;
    for end in 1..=chars.len() {
        if end == chars.len() || roles[end] != roles[start] {
            let text: String = chars[start..end].iter().collect();
            job.append(&text,0.0,TextFormat {font_id:FontId::monospace(12.0),color:rgb(palette.color(roles[start]).unwrap_or(theme.foreground)),..Default::default()});
            start = end;
        }
    }
    egui::Frame::new().fill(rgb(theme.background)).corner_radius(6).inner_margin(12).show(ui,|ui| {
        ui.set_min_width((ui.available_width()-24.0).max(0.0));
        job.wrap.max_width = ui.available_width();
        ui.label(job);
    });
    if settings.syntax_highlighting {ui.small(settings.highlight_style.description());}
}

// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! omniTerm as a native application.
//!
//! `omniterm-app` opens the window. `--bench [MiB]` measures the native path without a window,
//! and `--smoke <png>` opens the window, runs one command in a real shell, saves a screenshot
//! and exits, for CI.

mod agent;
mod backend;
mod bench;
mod core;
mod glyphs;
mod gpu;
mod grid;
mod icons;
mod input;
mod layout;
mod png;
mod remote;
mod smoke;
mod terminal;
mod theme;
mod ui;

use eframe::egui;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--bench") {
        let megabytes = args.get(1).and_then(|value| value.parse().ok()).unwrap_or(32);
        println!("{}", bench::run(megabytes));
        return Ok(());
    }
    let smoke = (args.first().map(String::as_str) == Some("--smoke")).then(|| smoke::Smoke::new(args.get(1).map(Into::into).unwrap_or_else(|| "omniterm-native-smoke.png".into())));
    let settings = core::settings::Settings::load();
    // Transparency is fixed when the window is created; changing it takes a restart.
    let transparent = settings.transparent_window();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("omniTerm")
        .with_app_id("omniterm")
        .with_inner_size([1440.0, 940.0])
        .with_min_inner_size([1000.0, 650.0])
        .with_transparent(transparent);
    // The title bar is the app's own, in the theme's colors. macOS keeps its traffic lights.
    viewport = if cfg!(target_os = "macos") {
        viewport.with_fullsize_content_view(true).with_titlebar_shown(false).with_title_shown(false)
    } else {
        viewport.with_decorations(false)
    };
    let options = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Wgpu, ..Default::default() };
    let result = eframe::run_native("omniTerm", options, Box::new(move |cc| Ok(Box::new(ui::App::new(cc, settings, transparent, smoke)))));
    remote::cleanup();
    result
}

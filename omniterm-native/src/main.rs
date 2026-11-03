// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! omniTerm as a native application.
//!
//! `omniterm-app` opens the window. `--bench [MiB]` measures the native path without a window,
//! and `--smoke <png>` opens the window, runs one command in a real shell, saves a screenshot
//! and exits, for CI.

mod app;
mod backend;
mod bench;
mod glyphs;
mod gpu;
mod grid;
mod input;
mod layout;
mod png;
mod terminal;
mod theme;

use eframe::egui;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--bench") {
        let megabytes = args.get(1).and_then(|value| value.parse().ok()).unwrap_or(32);
        println!("{}", bench::run(megabytes));
        return Ok(());
    }
    let smoke = (args.first().map(String::as_str) == Some("--smoke"))
        .then(|| app::Smoke::new(args.get(1).map(Into::into).unwrap_or_else(|| "omniterm-native-smoke.png".into())));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("omniTerm")
            .with_app_id("omniterm")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([480.0, 320.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native("omniTerm", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, smoke)))))
}

// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! omniTerm as a native application.
//!
//! `omniterm-app` opens the window. `--bench [MiB]` measures the native path without a window,
//! and `--smoke <png>` opens the window, runs one command in a real shell, saves a screenshot
//! and exits, for CI.

// A windowed app on Windows: no console window opens beside it in release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod agent;
mod backend;
mod bench;
mod core;
mod drag_out;
mod glyphs;
mod gpu;
mod grid;
mod icons;
mod input;
mod layout;
mod monitor;
mod png;
mod remote;
mod smoke;
mod terminal;
mod theme;
mod ui;

use eframe::egui;

/// The desktop app's icon, for the window and the task bar.
fn app_icon() -> Option<egui::IconData> {
    let image = image::load_from_memory(include_bytes!("../../omniterm-desktop/assets/icon.png")).ok()?.to_rgba8();
    let (width, height) = image.dimensions();
    Some(egui::IconData { rgba: image.into_raw(), width, height })
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = remote::qizhi::helper(&args, ui::ssh_auth::show) {
        std::process::exit(code);
    }
    if let Some(code) = core::ssh_auth::helper(&args, ui::ssh_auth::show) {
        std::process::exit(code);
    }
    // ssh runs omniTerm as its `xauth` for X11 forwarding on Windows; answer and leave.
    if let Some(code) = core::x11::xauth_shim(&args) {
        std::process::exit(code);
    }
    if args.first().map(String::as_str) == Some("--verify-file-connection") {
        let code = match args.get(1) {
            Some(path) if args.len() == 2 => remote::verify::run(std::path::Path::new(path)),
            _ => {
                eprintln!("Usage: omniTerm --verify-file-connection <connection-parameters.json>");
                2
            }
        };
        std::process::exit(code);
    }
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
    if let Some(icon) = app_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Wgpu, ..Default::default() };
    let result = eframe::run_native("omniTerm", options, Box::new(move |cc| Ok(Box::new(ui::App::new(cc, settings, transparent, smoke)))));
    remote::cleanup();
    core::x11::shutdown();
    core::ssh_auth::cleanup();
    result
}

// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! `--smoke`: a shell starts in a real window, runs a command, its output reaches the screen,
//! and a screenshot is saved. CI runs it on real GPU adapters.

use crate::backend::Backend;
use crate::terminal::Terminal;
use eframe::egui::{self, ViewportCommand};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct Smoke {
    started: Instant,
    typed: bool,
    seen: Option<Duration>,
    frames: u64,
    screenshot: Arc<Mutex<Option<Vec<u8>>>>,
    requested: bool,
    output: PathBuf,
}

impl Smoke {
    pub fn new(output: PathBuf) -> Self {
        Self { started: Instant::now(), typed: false, seen: None, frames: 0, screenshot: Arc::default(), requested: false, output }
    }

    pub fn tick(&mut self, ctx: &egui::Context, backend: &Backend, first: Option<(Arc<Terminal>, eframe::wgpu::Backend)>, font: &str) {
        self.frames += 1;
        let elapsed = self.started.elapsed();
        ctx.request_repaint_after(Duration::from_millis(50));
        let Some((terminal, adapter)) = first else {
            if elapsed > Duration::from_secs(40) {
                eprintln!("smoke failed: no terminal opened after {elapsed:?}");
                std::process::exit(1);
            }
            return;
        };
        if !self.typed && elapsed > Duration::from_millis(1500) {
            self.typed = true;
            // The typed line never contains the expected output, so seeing it proves a run.
            let command = if cfg!(windows) { "Write-Output ('omniterm-' + 'smoke-' + 42)\r" } else { "printf 'omniterm-%s-%s\\n' smoke 42\r" };
            backend.write(&terminal.id, command.to_string());
        }
        if self.seen.is_none() && self.typed && terminal.screen_text().lines().any(|line| line.trim() == "omniterm-smoke-42") {
            self.seen = Some(elapsed);
        }
        if self.seen.is_some() && !self.requested {
            self.requested = true;
            let target = self.screenshot.clone();
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |image| {
                let rgba: Vec<u8> = image.pixels.iter().flat_map(|pixel| pixel.to_array()).collect();
                *target.lock().unwrap_or_else(|p| p.into_inner()) = Some(crate::png::encode(image.size[0] as u32, image.size[1] as u32, &rgba));
            })));
        }
        let shot = self.screenshot.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(png) = shot {
            let written = std::fs::write(&self.output, png).is_ok();
            let report = serde_json::json!({
                "smoke": "passed",
                "firstOutputMs": self.seen.map(|seen| seen.as_millis()),
                "frames": self.frames,
                "screenshot": written.then(|| self.output.display().to_string()),
                "adapter": format!("{adapter:?}"),
                "font": font,
            });
            println!("{report}");
            backend.shutdown();
            std::process::exit(0);
        }
        if elapsed > Duration::from_secs(40) {
            eprintln!("smoke failed: no output after {elapsed:?}; screen:\n{}", terminal.screen_text());
            std::process::exit(1);
        }
    }
}

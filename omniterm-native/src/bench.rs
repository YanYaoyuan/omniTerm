// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! `omniterm-app --bench`: what the native path costs without a window, so runs on different
//! machines and CI can be compared. It measures emulation throughput (alacritty's parser, and the
//! host's vt100 screen model the Agent reads) and how long one full frame of quads takes to build.

use crate::glyphs::Fonts;
use crate::grid::{self, Frame};
use crate::terminal::{Size, Terminal};
use crate::theme::THEMES;
use omniterm_terminal_host::screen::ScreenModel;
use std::sync::mpsc::channel;
use std::time::Instant;

/// Typical build and log output: colored prefixes, paths, CJK and plain text.
pub fn workload(megabytes: usize) -> Vec<u8> {
    let lines: [&[u8]; 4] = [
        b"\x1b[32m   Compiling\x1b[0m omniterm-native v0.0.1 (/home/user/omniTerm/omniterm-native)\r\n",
        b"\x1b[1;31merror[E0308]\x1b[0m: mismatched types \x1b[2m--> src/app.rs:120:17\x1b[0m\r\n",
        "2026-11-03T10:15:32Z INFO 正在连接到 远程主机 host-01 端口 22 · 用户 deploy\r\n".as_bytes(),
        b"drwxr-xr-x  12 user staff   384 Nov  3 10:15 node_modules  -rw-r--r--  1 user staff 1024 README.md\r\n",
    ];
    let mut out = Vec::with_capacity(megabytes << 20);
    while out.len() < megabytes << 20 {
        for line in lines {
            out.extend_from_slice(line);
        }
    }
    out
}

fn rate(bytes: usize, seconds: f64) -> f64 {
    bytes as f64 / (1 << 20) as f64 / seconds
}

pub fn run(megabytes: usize) -> serde_json::Value {
    let data = workload(megabytes);
    let (sender, _receiver) = channel();
    let options = crate::terminal::Options { scrollback: 10_000, cursor: alacritty_terminal::vte::ansi::CursorShape::Beam, blinking: false };
    let terminal = Terminal::new("bench".into(), Size { cols: 200, rows: 60 }, options, sender);
    let started = Instant::now();
    for chunk in data.chunks(64 * 1024) {
        terminal.feed(chunk);
    }
    let emulator = rate(data.len(), started.elapsed().as_secs_f64());

    let mut model = ScreenModel::new(60, 200, "00000000000000000000000000000000");
    let started = Instant::now();
    for chunk in data.chunks(64 * 1024) {
        model.process(chunk);
    }
    let screen_model = rate(data.len(), started.elapsed().as_secs_f64());

    let mut fonts = Fonts::new(14.0, 1.22, &[]);
    let size = [200.0 * fonts.cell.width + 16.0, 60.0 * fonts.cell.height + 16.0];
    let frame = Frame { theme: &THEMES[0], size, focused: true, background_alpha: 1.0, dim: false, cursor_on: true };
    let started = Instant::now();
    let first = grid::build(&terminal.lock(), &mut fonts, &frame).len();
    let cold = started.elapsed().as_secs_f64() * 1000.0;
    let frames = 120;
    let started = Instant::now();
    for _ in 0..frames {
        std::hint::black_box(grid::build(&terminal.lock(), &mut fonts, &frame));
    }
    let warm = started.elapsed().as_secs_f64() * 1000.0 / frames as f64;
    serde_json::json!({
        "workloadMiB": megabytes,
        "emulatorMiBps": (emulator * 10.0).round() / 10.0,
        "agentScreenModelMiBps": (screen_model * 10.0).round() / 10.0,
        "grid": "200x60",
        "quadsPerFrame": first,
        "firstFrameMs": (cold * 100.0).round() / 100.0,
        "frameBuildMs": (warm * 1000.0).round() / 1000.0,
        "font": fonts.family(),
    })
}

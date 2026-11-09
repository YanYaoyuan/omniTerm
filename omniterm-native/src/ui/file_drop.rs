// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! A native file drop has one position and one consumer for the whole frame.
//! In particular, macOS Finder drags do not update egui's ordinary pointer position.

use eframe::egui::{self, Pos2, Rect, Ui};
use std::path::PathBuf;

#[derive(Default)]
pub struct FrameDrop {
    position: Option<Pos2>,
    hovering: bool,
    paths: Vec<PathBuf>,
}

impl FrameDrop {
    pub fn capture(frame: &eframe::Frame, ctx: &egui::Context, blocked: bool) -> Self {
        let position = crate::file_drop_position::position(frame, ctx);
        Self::from_input(ctx, position, blocked)
    }

    fn from_input(ctx: &egui::Context, position: Option<Pos2>, blocked: bool) -> Self {
        // Remove the global payload so no other widget can also type/upload these files.
        let (hovering, paths) = ctx.input_mut(|input| {
            let paths = std::mem::take(&mut input.raw.dropped_files).into_iter()
                .map(|file| file.path().to_path_buf()).filter(|path| !path.as_os_str().is_empty()).collect();
            (!input.raw.hovered_files.is_empty(), paths)
        });
        if blocked || ctx.memory(|memory| memory.top_modal_layer().is_some()) {
            return Self::default();
        }
        Self { position, hovering, paths }
    }

    fn contains(&self, ui: &Ui, rect: Rect) -> bool {
        let Some(position) = self.position else { return false };
        let rect = rect.intersect(ui.clip_rect());
        rect.is_positive() && rect.contains(position)
            && ui.ctx().layer_id_at(position) == Some(ui.layer_id())
    }

    pub fn hovered(&self, ui: &Ui, rect: Rect) -> bool {
        self.hovering && self.contains(ui, rect)
    }

    pub fn take(&mut self, ui: &Ui, rect: Rect) -> Vec<PathBuf> {
        if self.contains(ui, rect) { std::mem::take(&mut self.paths) } else { Vec::new() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::Arc;

    #[derive(Debug)]
    struct File(PathBuf);
    impl egui::DroppedFile for File {
        fn path(&self) -> &Path { &self.0 }
        fn bytes(&self) -> Result<Vec<u8>, String> { panic!("routing must not read file contents") }
    }

    fn input() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1000.0, 600.0))),
            // Last ordinary mouse movement was over SSH; Finder then moved over the files.
            events: vec![egui::Event::PointerMoved(egui::pos2(700.0, 200.0))],
            dropped_files: vec![Arc::new(File("/tmp/报告 1.txt".into())), Arc::new(File("/tmp/photo.png".into()))],
            ..Default::default()
        }
    }

    #[test]
    fn native_drop_position_overrides_stale_terminal_pointer_and_is_consumed_once() {
        let ctx = egui::Context::default();
        ctx.run_ui(input(), |ui| {
            let mut drop = FrameDrop::from_input(ui.ctx(), Some(egui::pos2(100.0, 200.0)), false);
            let sidebar = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(250.0, 600.0));
            let terminal = Rect::from_min_max(egui::pos2(250.0, 0.0), egui::pos2(1000.0, 600.0));
            assert!(drop.take(ui, terminal).is_empty(), "stale pointer must not write to SSH");
            assert_eq!(drop.take(ui, sidebar), [PathBuf::from("/tmp/报告 1.txt"), PathBuf::from("/tmp/photo.png")]);
            assert!(drop.take(ui, sidebar).is_empty());
            assert!(ui.input(|input| input.raw.dropped_files.is_empty()));
        }).drop_without_applying_deltas();
    }

    #[test]
    fn direct_terminal_drop_still_reaches_only_the_target_pane() {
        let ctx = egui::Context::default();
        ctx.run_ui(input(), |ui| {
            let mut drop = FrameDrop::from_input(ui.ctx(), Some(egui::pos2(700.0, 200.0)), false);
            let left = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(500.0, 600.0));
            let right = Rect::from_min_max(egui::pos2(500.0, 0.0), egui::pos2(1000.0, 600.0));
            assert!(drop.take(ui, left).is_empty());
            assert_eq!(drop.take(ui, right).len(), 2);
            assert!(drop.take(ui, right).is_empty());
        }).drop_without_applying_deltas();
    }

    #[test]
    fn unknown_position_and_blocking_dialog_never_fall_back_to_terminal_pointer() {
        for (position, blocked) in [(None, false), (Some(egui::pos2(700.0, 200.0)), true)] {
            let ctx = egui::Context::default();
            ctx.run_ui(input(), |ui| {
                let mut drop = FrameDrop::from_input(ui.ctx(), position, blocked);
                assert!(drop.take(ui, ui.max_rect()).is_empty());
            }).drop_without_applying_deltas();
        }
    }

    #[test]
    fn clipped_out_rows_cannot_capture_drops_and_no_payload_survives_the_frame() {
        let ctx = egui::Context::default();
        ctx.run_ui(input(), |ui| {
            let mut drop = FrameDrop::from_input(ui.ctx(), Some(egui::pos2(100.0, 400.0)), false);
            let panel = ui.max_rect();
            ui.set_clip_rect(Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(250.0, 300.0)));
            assert!(drop.take(ui, panel).is_empty());
        }).drop_without_applying_deltas();
        ctx.run_ui(Default::default(), |ui| {
            let mut drop = FrameDrop::from_input(ui.ctx(), Some(egui::pos2(100.0, 200.0)), false);
            assert!(drop.take(ui, ui.max_rect()).is_empty());
        }).drop_without_applying_deltas();
    }

    #[test]
    fn foreground_overlay_and_existing_modal_block_underlying_drop_targets() {
        let sidebar = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(250.0, 600.0));
        let terminal = Rect::from_min_max(egui::pos2(250.0, 0.0), egui::pos2(1000.0, 600.0));
        let overlay = |ctx: &egui::Context| {
            egui::Area::new(egui::Id::unique("drop-test-overlay"))
                .fixed_pos(egui::Pos2::ZERO).order(egui::Order::Foreground)
                .movable(false).interactable(true)
                .show(ctx, |ui| { ui.set_min_size(egui::vec2(900.0, 500.0)); });
        };
        let ctx = egui::Context::default();
        // Area's first sizing frame deliberately records it as non-interactable.
        // Draw a second frame so this models an already visible foreground window.
        for _ in 0..2 {
            let mut warmup = input();
            warmup.dropped_files.clear();
            ctx.run_ui(warmup, |ui| overlay(ui.ctx())).drop_without_applying_deltas();
        }
        for position in [egui::pos2(100.0, 200.0), egui::pos2(700.0, 200.0)] {
            ctx.run_ui(input(), |ui| {
                // Like the app, capture before drawing the overlay again this frame.
                let mut drop = FrameDrop::from_input(ui.ctx(), Some(position), false);
                assert_eq!(drop.paths.len(), 2, "a nonmodal overlay must be rejected by hit testing, not by removing the payload");
                assert_eq!(ui.ctx().layer_id_at(position), Some(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("drop-test-overlay"))));
                assert!(drop.take(ui, sidebar).is_empty(), "covered file panel must not upload");
                assert!(drop.take(ui, terminal).is_empty(), "covered terminal must not receive paths");
                overlay(ui.ctx());
            }).drop_without_applying_deltas();
        }

        let ctx = egui::Context::default();
        let modal = |ctx: &egui::Context| {
            egui::Modal::new(egui::Id::unique("drop-test-confirmation"))
                .show(ctx, |ui| { ui.set_min_size(egui::vec2(300.0, 100.0)); });
        };
        for _ in 0..2 {
            let mut warmup = input();
            warmup.dropped_files.clear();
            ctx.run_ui(warmup, |ui| modal(ui.ctx())).drop_without_applying_deltas();
        }
        assert!(ctx.memory(|memory| memory.top_modal_layer().is_some()));
        ctx.run_ui(input(), |ui| {
            let mut drop = FrameDrop::from_input(ui.ctx(), Some(egui::pos2(700.0, 200.0)), false);
            assert!(drop.paths.is_empty(), "an existing confirmation blocks the payload before any drop consumer runs");
            assert!(drop.take(ui, sidebar).is_empty());
            assert!(drop.take(ui, terminal).is_empty());
            assert!(ui.input(|input| input.raw.dropped_files.is_empty()));
            modal(ui.ctx());
        }).drop_without_applying_deltas();
    }
}

// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! External file drags need a fresh destination position. winit's macOS file
//! events contain paths, but do not update egui's ordinary pointer position.

use eframe::egui::{self, Pos2};

/// Sample once per frame, before the caller consumes `raw.dropped_files`, then
/// use that one position for every possible drop target. On macOS an unavailable
/// native position must never fall back to an old terminal pointer position.
pub fn position(frame: &eframe::Frame, ctx: &egui::Context) -> Option<Pos2> {
    let (hovering, dropped) = ctx.input(|input| (!input.raw.hovered_files.is_empty(), !input.raw.dropped_files.is_empty()));
    if !hovering && !dropped {
        return None;
    }
    if hovering {
        // AppKit does not send normal mouse-motion events while Finder owns the
        // drag, so refresh the drop highlight even when no UI event arrives.
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
    #[cfg(target_os = "macos")]
    {
        native_position(frame, ctx.pixels_per_point())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = frame;
        ctx.pointer_hover_pos()
    }
}

#[cfg(target_os = "macos")]
fn native_position(frame: &eframe::Frame, pixels_per_point: f32) -> Option<Pos2> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSView;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let _main_thread = MainThreadMarker::new()?;
    let window_handle = frame.window_handle().ok()?;
    let RawWindowHandle::AppKit(handle) = window_handle.as_raw() else { return None };
    // SAFETY: AppKit handles contain a live NSView borrowed from this Frame.
    // The handle remains borrowed here, and AppKit access is on the main thread.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let window = view.window()?;
    let point = view.convertPoint_fromView(window.mouseLocationOutsideOfEventStream(), None);
    let bounds = view.bounds();
    content_position(
        [point.x, point.y],
        [bounds.origin.x, bounds.origin.y],
        [bounds.size.width, bounds.size.height],
        view.isFlipped(),
        window.backingScaleFactor(),
        pixels_per_point as f64,
    )
}

/// AppKit view units → physical pixels → egui points. The backing scale and
/// egui scale differ when the user changes UI zoom, including on Retina screens.
#[cfg(any(target_os = "macos", test))]
fn content_position(point: [f64; 2], origin: [f64; 2], size: [f64; 2], flipped: bool, backing_scale: f64, pixels_per_point: f64) -> Option<Pos2> {
    if !point.into_iter().chain(origin).chain(size).chain([backing_scale, pixels_per_point]).all(f64::is_finite)
        || size[0] <= 0.0 || size[1] <= 0.0 || backing_scale <= 0.0 || pixels_per_point <= 0.0
    {
        return None;
    }
    let x = point[0] - origin[0];
    let y = if flipped { point[1] - origin[1] } else { size[1] - (point[1] - origin[1]) };
    if x < 0.0 || y < 0.0 || x >= size[0] || y >= size[1] {
        return None;
    }
    let scale = backing_scale / pixels_per_point;
    let position = Pos2::new((x * scale) as f32, (y * scale) as f32);
    position.is_finite().then_some(position)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_content_coordinates_respect_view_orientation_and_origin() {
        let top_left = content_position([110.0, 120.0], [10.0, 20.0], [800.0, 600.0], true, 2.0, 2.0);
        let bottom_left = content_position([110.0, 520.0], [10.0, 20.0], [800.0, 600.0], false, 2.0, 2.0);
        assert_eq!(top_left, Some(Pos2::new(100.0, 100.0)));
        assert_eq!(bottom_left, top_left);
        assert_eq!(content_position([90.0, 80.0], [-10.0, -20.0], [800.0, 600.0], true, 2.0, 2.0), top_left);
    }

    #[test]
    fn retina_and_egui_zoom_use_distinct_scales() {
        let bounds = [1200.0, 800.0];
        assert_eq!(content_position([300.0, 150.0], [0.0; 2], bounds, true, 2.0, 2.0), Some(Pos2::new(300.0, 150.0)));
        assert_eq!(content_position([300.0, 150.0], [0.0; 2], bounds, true, 2.0, 3.0), Some(Pos2::new(200.0, 100.0)));
        assert_eq!(content_position([300.0, 150.0], [0.0; 2], bounds, true, 1.0, 1.5), Some(Pos2::new(200.0, 100.0)));
    }

    #[test]
    fn missing_invalid_or_outside_coordinates_do_not_choose_a_target() {
        for point in [[-1.0, 1.0], [1.0, -1.0], [800.0, 1.0], [1.0, 600.0], [f64::NAN, 1.0], [1.0, f64::INFINITY]] {
            assert!(content_position(point, [0.0; 2], [800.0, 600.0], true, 2.0, 2.0).is_none());
        }
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(content_position([1.0; 2], [0.0; 2], [800.0, 600.0], true, scale, 2.0).is_none());
            assert!(content_position([1.0; 2], [0.0; 2], [800.0, 600.0], true, 2.0, scale).is_none());
        }
        assert!(content_position([1.0; 2], [0.0; 2], [0.0, 600.0], true, 2.0, 2.0).is_none());
        assert!(content_position([1.0; 2], [f64::NAN, 0.0], [800.0, 600.0], true, 2.0, 2.0).is_none());
    }
}

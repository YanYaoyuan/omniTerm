// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Dragging a local file out of the window into the system file manager or another app.
//! Windows (OLE) and macOS (NSDraggingSession) have a drag source that works with any
//! window; on Linux one needs a GTK window, so files are downloaded there instead.

use std::path::Path;

/// Whether this platform can drag files out of the window.
pub const SUPPORTED: bool = cfg!(any(windows, target_os = "macos"));

/// Starts a drag of `file` from the window, while the mouse button is still held. On
/// Windows this returns once the file is dropped or the drag is canceled.
#[cfg(any(windows, target_os = "macos"))]
pub fn start(window: &eframe::Frame, file: &Path) -> Result<(), String> {
    let item = drag::DragItem::Files(vec![file.to_path_buf()]);
    drag::start_drag(window, item, drag::Image::Raw(preview().to_vec()), |_, _| {}, drag::Options::default()).map_err(|error| format!("无法开始拖动：{error}"))
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn start(_window: &eframe::Frame, _file: &Path) -> Result<(), String> {
    Err("这个平台不支持把文件拖出窗口，请用“下载”。".into())
}

/// The app icon at drag-image size, encoded once.
#[cfg(any(windows, target_os = "macos"))]
fn preview() -> &'static [u8] {
    static PREVIEW: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    PREVIEW.get_or_init(|| {
        let mut bytes = Vec::new();
        if let Ok(icon) = image::load_from_memory(include_bytes!("../../omniterm-desktop/assets/icon.png")) {
            let _ = icon.thumbnail(48, 48).write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png);
        }
        bytes
    })
}

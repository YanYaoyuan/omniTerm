// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Dragging a local file out of the window into the system file manager or another app.
//! Windows (OLE) and macOS (NSDraggingSession) go through the `drag` crate. On Linux an
//! X11 window gets an XDND source on a connection of its own, and a Wayland window starts a
//! `wl_data_device` drag on winit's display, both offering the file as `text/uri-list`.

use std::path::Path;

#[cfg(target_os = "linux")]
mod wayland;
#[cfg(target_os = "linux")]
mod x11;

/// Whether this platform can drag files out of the window.
pub const SUPPORTED: bool = cfg!(any(windows, target_os = "macos", target_os = "linux"));

/// Starts a drag of `file` from the window, while the mouse button is still held. On
/// Windows this returns once the file is dropped or the drag is canceled.
#[cfg(any(windows, target_os = "macos"))]
pub fn start(window: &eframe::Frame, file: &Path) -> Result<(), String> {
    let item = drag::DragItem::Files(vec![file.to_path_buf()]);
    drag::start_drag(window, item, drag::Image::Raw(preview().to_vec()), |_, _| {}, drag::Options::default()).map_err(|error| format!("无法开始拖动：{error}"))
}

#[cfg(target_os = "linux")]
pub fn start(window: &eframe::Frame, file: &Path) -> Result<(), String> {
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
    let unavailable = |error: raw_window_handle::HandleError| format!("无法开始拖动：{error}");
    let handle = window.window_handle().map_err(unavailable)?.as_raw();
    let display = window.display_handle().map_err(unavailable)?.as_raw();
    match (handle, display) {
        (RawWindowHandle::Wayland(handle), RawDisplayHandle::Wayland(display)) => wayland::start(display.display.as_ptr(), handle.surface.as_ptr(), file),
        (RawWindowHandle::Xlib(handle), RawDisplayHandle::Xlib(display)) => x11::start(display.display.map(|display| display.as_ptr()), handle.window as u32, file),
        (RawWindowHandle::Xcb(handle), _) => x11::start(None, handle.window.get(), file),
        _ => Err("这个窗口系统不支持把文件拖出窗口，请用“下载”。".into()),
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub fn start(_window: &eframe::Frame, _file: &Path) -> Result<(), String> {
    Err("这个平台不支持把文件拖出窗口，请用“下载”。".into())
}

/// Runs every frame. On Wayland a drag needs the serial of the button press that began it,
/// so the drag source watches the pointer from the start, and answers the compositor's
/// requests for the file while a drag is under way.
pub fn pump(window: &eframe::Frame, ctx: &eframe::egui::Context) {
    #[cfg(target_os = "linux")]
    {
        use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
        if let Ok(RawDisplayHandle::Wayland(display)) = window.display_handle().map(|handle| handle.as_raw()) {
            wayland::pump(display.display.as_ptr(), ctx);
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (window, ctx);
}

/// A `file://` URI for `text/uri-list`: every byte outside the unreserved set is escaped.
#[cfg(target_os = "linux")]
fn file_uri(file: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file://");
    for &byte in file.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri.push_str("\r\n");
    uri
}

/// The app icon at drag-image size, encoded once.
#[cfg(any(windows, target_os = "macos"))]
fn preview() -> &'static [u8] {
    static PREVIEW: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    PREVIEW.get_or_init(|| {
        let mut bytes = Vec::new();
        if let Ok(icon) = image::load_from_memory(include_bytes!("../../../omniterm-desktop/assets/icon.png")) {
            let _ = icon.thumbnail(48, 48).write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png);
        }
        bytes
    })
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn uri_lists_escape_everything_a_file_manager_could_misread() {
        assert_eq!(file_uri(Path::new("/tmp/omniterm-drag-1/a b/报告 #1%.txt")), "file:///tmp/omniterm-drag-1/a%20b/%E6%8A%A5%E5%91%8A%20%231%25.txt\r\n");
        assert_eq!(file_uri(Path::new("/x/new\nline")), "file:///x/new%0Aline\r\n");
    }
}

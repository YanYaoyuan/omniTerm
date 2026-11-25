// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Read-only global pointer sampling. No event hooks, input injection or permissions prompts.
use eframe::egui::{Pos2, Rect, Vec2};
pub use platform::Tracker;
pub struct Sample { pub pointer: Pos2, pub bounds: Rect }

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use x11rb::{connection::Connection, protocol::xproto::ConnectionExt, rust_connection::RustConnection};
    pub struct Tracker { connection: RustConnection, root: u32 }
    impl Tracker {
        pub fn new() -> Result<Self, String> {
            if std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v == "wayland") || std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty()) {
                return Err("Wayland 暂不支持跨应用读取鼠标位置；桌面跟随需要 X11 会话。".into());
            }
            let (connection, screen) = x11rb::connect(None).map_err(|e| e.to_string())?;
            let root = connection.setup().roots[screen].root;
            // Without compositing, an alpha window becomes an opaque rectangle.
            let atom = connection.intern_atom(false, format!("_NET_WM_CM_S{screen}").as_bytes()).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?.atom;
            let owner = connection.get_selection_owner(atom).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?.owner;
            if owner == 0 { return Err("桌面跟随需要启用 X11 窗口合成器，以显示透明小猫。".into()); }
            Ok(Self { connection, root })
        }
        pub fn sample(&self) -> Result<Sample, String> {
            let p = self.connection.query_pointer(self.root).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?;
            if !p.same_screen { return Err("鼠标已离开当前 X11 屏幕，请收回后重新放出小猫。".into()); }
            let g = self.connection.get_geometry(self.root).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?;
            Ok(Sample { pointer: Pos2::new(f32::from(p.root_x), f32::from(p.root_y)), bounds: Rect::from_min_size(Pos2::ZERO, Vec2::new(f32::from(g.width), f32::from(g.height))) })
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use windows_sys::Win32::{Foundation::POINT, UI::WindowsAndMessaging::{GetCursorPos, GetSystemMetrics, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN}};
    pub struct Tracker;
    impl Tracker {
        pub fn new() -> Result<Self, String> { Ok(Self) }
        pub fn sample(&self) -> Result<Sample, String> {
            let mut p = POINT { x: 0, y: 0 };
            // SAFETY: the POINT is writable and these APIs only read desktop state.
            unsafe {
                if GetCursorPos(&mut p) == 0 { return Err("暂时无法读取桌面鼠标位置。".into()); }
                Ok(Sample { pointer: Pos2::new(p.x as f32, p.y as f32), bounds: Rect::from_min_size(Pos2::new(GetSystemMetrics(SM_XVIRTUALSCREEN) as f32, GetSystemMetrics(SM_YVIRTUALSCREEN) as f32), Vec2::new(GetSystemMetrics(SM_CXVIRTUALSCREEN) as f32, GetSystemMetrics(SM_CYVIRTUALSCREEN) as f32)) })
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::ffi::c_void;
    #[repr(C)] struct Point { x: f64, y: f64 }
    #[repr(C)] struct Size { width: f64, height: f64 }
    #[repr(C)] struct Bounds { origin: Point, size: Size }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventCreate(source: *const c_void) -> *const c_void;
        fn CGEventGetLocation(event: *const c_void) -> Point;
        fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
        fn CGDisplayBounds(display: u32) -> Bounds;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" { fn CFRelease(value: *const c_void); }
    pub struct Tracker;
    impl Tracker {
        pub fn new() -> Result<Self, String> { Ok(Self) }
        pub fn sample(&self) -> Result<Sample, String> {
            // SAFETY: null source samples current state, never posts an event. Each
            // retained event is released. Display IDs and CGRect follow the C ABI.
            unsafe {
                let event = CGEventCreate(std::ptr::null());
                if event.is_null() { return Err("暂时无法读取桌面鼠标位置。".into()); }
                let p = CGEventGetLocation(event); CFRelease(event);
                let mut ids = [0; 32]; let mut count = 0;
                if CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) != 0 { return Err("无法读取显示器范围。".into()); }
                let mut bounds = Rect::NOTHING;
                for id in &ids[..count as usize] {
                    let b = CGDisplayBounds(*id);
                    bounds = bounds.union(Rect::from_min_size(Pos2::new(b.origin.x as f32, b.origin.y as f32), Vec2::new(b.size.width as f32, b.size.height as f32)));
                }
                Ok(Sample { pointer: Pos2::new(p.x as f32, p.y as f32), bounds })
            }
        }
    }
}

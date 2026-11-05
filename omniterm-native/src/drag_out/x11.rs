// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! An XDND (version 5) drag source on X11. It runs on a connection of its own, so its
//! events never pass through winit: a hidden window owns `XdndSelection`, a pointer grab on
//! the root window follows the drag, and the drop target fetches the file as
//! `text/uri-list`.

use super::file_uri;
use std::ffi::c_void;
use std::path::Path;
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    self, AtomEnum, ClientMessageEvent, ConnectionExt as _, CreateWindowAux, EventMask, GrabMode, GrabStatus, PropMode, SelectionNotifyEvent, SelectionRequestEvent, Timestamp, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{COPY_DEPTH_FROM_PARENT, COPY_FROM_PARENT, CURRENT_TIME, NONE};

/// The newest protocol version spoken; targets older than 3 are ignored, as GTK and Qt do.
const VERSION: u32 = 5;
/// How long a released drag waits for the target's status before giving up.
const STATUS_WAIT: Duration = Duration::from_secs(1);
/// How long the target may take to fetch the file after the drop.
const FINISH_WAIT: Duration = Duration::from_secs(30);
/// How deep the window tree is searched for a drop target.
const MAX_DEPTH: usize = 32;

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        XdndAware,
        XdndProxy,
        XdndSelection,
        XdndTypeList,
        XdndEnter,
        XdndLeave,
        XdndPosition,
        XdndStatus,
        XdndDrop,
        XdndFinished,
        XdndActionCopy,
        TARGETS,
        URI_LIST: b"text/uri-list",
    }
}

fn failed(error: impl std::fmt::Display) -> String {
    format!("无法开始拖动：{error}")
}

/// Takes over the pointer and starts the drag; the protocol then runs on its own thread.
/// `display` is winit's Xlib display, whose implicit grab from the button press has to go
/// before another connection can grab the pointer.
pub fn start(display: Option<*mut c_void>, own: Window, file: &Path) -> Result<(), String> {
    if let Some(display) = display {
        release_implicit_grab(display);
    }
    let (conn, screen) = RustConnection::connect(None).map_err(failed)?;
    let root = conn.setup().roots[screen].root;
    let atoms = Atoms::new(&conn).map_err(failed)?.reply().map_err(failed)?;
    let source = conn.generate_id().map_err(failed)?;
    conn.create_window(COPY_DEPTH_FROM_PARENT, source, root, -16, -16, 1, 1, 0, WindowClass::INPUT_ONLY, COPY_FROM_PARENT, &CreateWindowAux::new().override_redirect(1)).map_err(failed)?;
    conn.change_property32(PropMode::REPLACE, source, atoms.XdndTypeList, AtomEnum::ATOM, &[atoms.URI_LIST]).map_err(failed)?;
    conn.set_selection_owner(source, atoms.XdndSelection, CURRENT_TIME).map_err(failed)?;
    let cursor = drag_cursor(&conn).unwrap_or(NONE);
    // The server may handle winit's ungrab a moment after this connection's first try.
    let deadline = Instant::now() + Duration::from_millis(300);
    loop {
        let reply = conn.grab_pointer(false, root, EventMask::POINTER_MOTION | EventMask::BUTTON_RELEASE, GrabMode::ASYNC, GrabMode::ASYNC, NONE, cursor, CURRENT_TIME).map_err(failed)?.reply().map_err(failed)?;
        if reply.status == GrabStatus::SUCCESS {
            break;
        }
        if Instant::now() >= deadline {
            let _ = conn.destroy_window(source);
            let _ = conn.flush();
            return Err("无法开始拖动：另一个程序占用了鼠标指针。".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    conn.flush().map_err(failed)?;
    let drag = Drag { conn, atoms, root, source, own, uri: file_uri(file).into_bytes(), target: None, accepted: false, waiting: false, pending: None, phase: Phase::Dragging };
    std::thread::Builder::new().name("omniterm-drag-out".into()).spawn(move || drag.run()).map_err(failed)?;
    Ok(())
}

/// Ends the grab winit's connection got from the button press, both the XInput 2 grab
/// winit's event selection makes and a core one.
fn release_implicit_grab(display: *mut c_void) {
    let display = display.cast::<x11_dl::xlib::Display>();
    // SAFETY: `display` is winit's live Xlib display, used here on the thread that runs
    // winit's event loop, between its event dispatches.
    unsafe {
        if let Ok(xinput) = x11_dl::xinput2::XInput2::open() {
            let mut device = 0;
            if (xinput.XIGetClientPointer)(display, 0, &mut device) != 0 {
                (xinput.XIUngrabDevice)(display, device, x11_dl::xlib::CurrentTime);
            }
        }
        if let Ok(xlib) = x11_dl::xlib::Xlib::open() {
            (xlib.XUngrabPointer)(display, x11_dl::xlib::CurrentTime);
            (xlib.XFlush)(display);
        }
    }
}

/// The hand from the standard cursor font, shown while dragging.
fn drag_cursor(conn: &RustConnection) -> Option<u32> {
    const HAND2: u16 = 60;
    let font = conn.generate_id().ok()?;
    conn.open_font(font, b"cursor").ok()?;
    let cursor = conn.generate_id().ok()?;
    conn.create_glyph_cursor(cursor, font, font, HAND2, HAND2 + 1, 0, 0, 0, 0xffff, 0xffff, 0xffff).ok()?;
    let _ = conn.close_font(font);
    Some(cursor)
}

struct Target {
    window: Window,
    /// Where messages go: the target, or the proxy it names.
    deliver: Window,
    version: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Dragging,
    /// Released while the target's status was outstanding: decide once it arrives.
    Deciding(Timestamp),
    Dropped,
}

struct Drag {
    conn: RustConnection,
    atoms: Atoms,
    root: Window,
    source: Window,
    /// The app's own window, which is never a target.
    own: Window,
    uri: Vec<u8>,
    target: Option<Target>,
    accepted: bool,
    /// A position was sent and its status has not come back yet.
    waiting: bool,
    /// The latest position seen while waiting, sent when the status arrives.
    pending: Option<(i16, i16, Timestamp)>,
    phase: Phase,
}

impl Drag {
    fn run(mut self) {
        let _ = self.drive();
        let _ = self.conn.ungrab_pointer(CURRENT_TIME);
        let _ = self.conn.destroy_window(self.source);
        let _ = self.conn.flush();
    }

    fn drive(&mut self) -> Result<(), String> {
        // While the button is down the grab guarantees a release; after it, the target gets
        // a bounded time to answer.
        let mut deadline: Option<Instant> = None;
        loop {
            let event = match deadline {
                None => self.conn.wait_for_event().map_err(failed)?,
                Some(limit) => match self.conn.poll_for_event().map_err(failed)? {
                    Some(event) => event,
                    None if Instant::now() >= limit => {
                        if matches!(self.phase, Phase::Deciding(_)) {
                            self.leave()?;
                        }
                        return Ok(());
                    }
                    None => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                },
            };
            match event {
                Event::MotionNotify(motion) if self.phase == Phase::Dragging => self.moved(motion.root_x, motion.root_y, motion.time)?,
                Event::ButtonRelease(release) if self.phase == Phase::Dragging => {
                    self.conn.ungrab_pointer(release.time).map_err(failed)?;
                    self.conn.flush().map_err(failed)?;
                    if self.target.is_none() {
                        return Ok(());
                    }
                    if self.waiting {
                        self.phase = Phase::Deciding(release.time);
                        deadline = Some(Instant::now() + STATUS_WAIT);
                    } else if self.finish(release.time)? {
                        deadline = Some(Instant::now() + FINISH_WAIT);
                    } else {
                        return Ok(());
                    }
                }
                Event::ClientMessage(message) if message.type_ == self.atoms.XdndStatus => {
                    let data = message.data.as_data32();
                    if self.target.as_ref().is_none_or(|target| target.window != data[0]) {
                        continue;
                    }
                    self.accepted = data[1] & 1 != 0;
                    self.waiting = false;
                    match self.phase {
                        Phase::Dragging => {
                            if let Some((x, y, time)) = self.pending.take() {
                                self.position(x, y, time)?;
                            }
                        }
                        Phase::Deciding(time) => {
                            if self.finish(time)? {
                                deadline = Some(Instant::now() + FINISH_WAIT);
                            } else {
                                return Ok(());
                            }
                        }
                        Phase::Dropped => {}
                    }
                }
                Event::ClientMessage(message) if message.type_ == self.atoms.XdndFinished && self.phase == Phase::Dropped => return Ok(()),
                Event::SelectionRequest(request) => self.answer(&request)?,
                _ => {}
            }
        }
    }

    /// Follows the pointer: leaves the old target, enters a new one and reports positions,
    /// one at a time as the protocol asks.
    fn moved(&mut self, x: i16, y: i16, time: Timestamp) -> Result<(), String> {
        let found = self.find(x, y);
        if found.as_ref().map(|target| target.window) != self.target.as_ref().map(|target| target.window) {
            self.leave()?;
            if let Some(target) = found {
                self.send(&target, self.atoms.XdndEnter, [self.source, target.version << 24, self.atoms.URI_LIST, 0, 0])?;
                self.target = Some(target);
            }
        }
        if self.target.is_some() {
            if self.waiting {
                self.pending = Some((x, y, time));
            } else {
                self.position(x, y, time)?;
            }
        }
        Ok(())
    }

    fn position(&mut self, x: i16, y: i16, time: Timestamp) -> Result<(), String> {
        let Some(target) = &self.target else { return Ok(()) };
        let point = (u32::from(x as u16) << 16) | u32::from(y as u16);
        self.send(target, self.atoms.XdndPosition, [self.source, 0, point, time, self.atoms.XdndActionCopy])?;
        self.waiting = true;
        Ok(())
    }

    fn leave(&mut self) -> Result<(), String> {
        if let Some(target) = self.target.take() {
            self.send(&target, self.atoms.XdndLeave, [self.source, 0, 0, 0, 0])?;
        }
        self.accepted = false;
        self.waiting = false;
        self.pending = None;
        Ok(())
    }

    /// Drops on a target that accepted, or leaves one that did not. True after a drop.
    fn finish(&mut self, time: Timestamp) -> Result<bool, String> {
        match &self.target {
            Some(target) if self.accepted => {
                self.send(target, self.atoms.XdndDrop, [self.source, 0, time, 0, 0])?;
                self.phase = Phase::Dropped;
                Ok(true)
            }
            _ => {
                self.leave()?;
                Ok(false)
            }
        }
    }

    /// The XDND-aware window under the point, searched from the root down. Windows that
    /// vanish mid-search simply end it.
    fn find(&self, x: i16, y: i16) -> Option<Target> {
        let mut parent = self.root;
        for _ in 0..MAX_DEPTH {
            let child = self.conn.translate_coordinates(self.root, parent, x, y).ok()?.reply().ok()?.child;
            if child == NONE || child == self.source || child == self.own {
                return None;
            }
            if let Some(version) = self.property(child, self.atoms.XdndAware, AtomEnum::ATOM.into()).filter(|version| *version >= 3) {
                let deliver = self.property(child, self.atoms.XdndProxy, AtomEnum::WINDOW.into()).filter(|proxy| *proxy != NONE).unwrap_or(child);
                return Some(Target { window: child, deliver, version: version.min(VERSION) });
            }
            parent = child;
        }
        None
    }

    fn property(&self, window: Window, property: u32, kind: u32) -> Option<u32> {
        self.conn.get_property(false, window, property, kind, 0, 1).ok()?.reply().ok()?.value32()?.next()
    }

    fn send(&self, target: &Target, kind: u32, data: [u32; 5]) -> Result<(), String> {
        let message = ClientMessageEvent::new(32, target.window, kind, data);
        self.conn.send_event(false, target.deliver, EventMask::NO_EVENT, message).map_err(failed)?;
        self.conn.flush().map_err(failed)
    }

    /// Hands the file's URI to the target, or lists what is on offer.
    fn answer(&self, request: &SelectionRequestEvent) -> Result<(), String> {
        // Obsolete requestors leave the property out and mean the target's name.
        let property = if request.property == NONE { request.target } else { request.property };
        let answered = request.selection == self.atoms.XdndSelection
            && if request.target == self.atoms.URI_LIST {
                self.conn.change_property8(PropMode::REPLACE, request.requestor, property, self.atoms.URI_LIST, &self.uri).is_ok()
            } else if request.target == self.atoms.TARGETS {
                self.conn.change_property32(PropMode::REPLACE, request.requestor, property, AtomEnum::ATOM, &[self.atoms.TARGETS, self.atoms.URI_LIST]).is_ok()
            } else {
                false
            };
        let notify = SelectionNotifyEvent {
            response_type: xproto::SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: request.time,
            requestor: request.requestor,
            selection: request.selection,
            target: request.target,
            property: if answered { property } else { NONE },
        };
        self.conn.send_event(false, request.requestor, EventMask::NO_EVENT, notify).map_err(failed)?;
        self.conn.flush().map_err(failed)
    }
}

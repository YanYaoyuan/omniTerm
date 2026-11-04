// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! A `wl_data_device` drag source on Wayland. It shares winit's display connection but has
//! an event queue, seat binding and pointer of its own: the pointer records the serial of
//! each button press, which `start_drag` must name, and the data source writes the file's
//! URI when the drop target asks for it. Events reach this queue whenever winit reads the
//! socket; the UI dispatches them every frame.

use super::file_uri;
use eframe::egui;
use std::cell::RefCell;
use std::ffi::c_void;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};
use wayland_client::backend::{Backend, ObjectId};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::WlDataOffer;
use wayland_client::protocol::wl_data_source::{self, WlDataSource};
use wayland_client::protocol::wl_pointer::{self, WlPointer};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};

const URI_LIST: &str = "text/uri-list";
/// Compositors before data device version 3 never say a drag is over; stop waiting then.
const LONGEST_DRAG: Duration = Duration::from_secs(120);

#[derive(Default)]
struct State {
    pointer: Option<WlPointer>,
    /// The serial of the latest button press on one of the app's surfaces.
    press: Option<u32>,
    /// Data sources still in a drag, and when the latest one started.
    drags: usize,
    since: Option<Instant>,
}

impl State {
    fn dragging(&self) -> bool {
        self.drags > 0 && self.since.is_some_and(|since| since.elapsed() < LONGEST_DRAG)
    }
}

struct Session {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
    manager: WlDataDeviceManager,
    device: WlDataDevice,
}

thread_local! {
    /// Set up on the first frame; `None` inside once setting up failed, so it is not retried.
    static SESSION: RefCell<Option<Option<Session>>> = const { RefCell::new(None) };
}

fn with_session<R>(display: *mut c_void, work: impl FnOnce(&mut Session) -> R) -> Option<R> {
    SESSION.with(|cell| {
        let mut slot = cell.borrow_mut();
        slot.get_or_insert_with(|| connect(display)).as_mut().map(work)
    })
}

fn connect(display: *mut c_void) -> Option<Session> {
    // SAFETY: `display` is winit's `wl_display`, which outlives the app's UI and this session.
    let backend = unsafe { Backend::from_foreign_display(display.cast()) };
    let conn = Connection::from_backend(backend);
    let (globals, mut queue) = registry_queue_init::<State>(&conn).ok()?;
    let handle = queue.handle();
    let seat: WlSeat = globals.bind(&handle, 1..=7, ()).ok()?;
    let manager: WlDataDeviceManager = globals.bind(&handle, 1..=3, ()).ok()?;
    let device = manager.get_data_device(&seat, &handle, ());
    let mut state = State::default();
    // The seat's capabilities arrive now, and with them the pointer.
    queue.roundtrip(&mut state).ok()?;
    Some(Session { conn, queue, state, manager, device })
}

pub fn pump(display: *mut c_void, ctx: &egui::Context) {
    with_session(display, |session| {
        let _ = session.queue.dispatch_pending(&mut session.state);
        // The compositor asks for the data while the UI may be idle; keep answering.
        if session.state.dragging() {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
    });
}

pub fn start(display: *mut c_void, surface: *mut c_void, file: &Path) -> Result<(), String> {
    with_session(display, |session| {
        let _ = session.queue.dispatch_pending(&mut session.state);
        let serial = session.state.press.ok_or("无法开始拖动：没有找到这次按下鼠标的记录。")?;
        // SAFETY: `surface` is the live `wl_surface` of winit's window on this connection.
        let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.cast()) }.map_err(|error| format!("无法开始拖动：{error}"))?;
        let surface = WlSurface::from_id(&session.conn, id).map_err(|error| format!("无法开始拖动：{error}"))?;
        let source = session.manager.create_data_source(&session.queue.handle(), file_uri(file).into_bytes());
        source.offer(URI_LIST.into());
        if source.version() >= 3 {
            source.set_actions(DndAction::Copy);
        }
        session.device.start_drag(Some(&source), &surface, None, serial);
        session.state.drags += 1;
        session.state.since = Some(Instant::now());
        session.conn.flush().map_err(|error| format!("无法开始拖动：{error}"))
    })
    .unwrap_or_else(|| Err("无法开始拖动：Wayland 合成器没有提供拖放服务。".into()))
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &WlRegistry, _: <WlRegistry as Proxy>::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlSeat, ()> for State {
    fn event(state: &mut Self, seat: &WlSeat, event: wl_seat::Event, _: &(), _: &Connection, handle: &QueueHandle<Self>) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(capabilities) } = event {
            let pointer = capabilities.contains(wl_seat::Capability::Pointer);
            if pointer && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(handle, ()));
            } else if !pointer && let Some(old) = state.pointer.take() {
                if old.version() >= 3 {
                    old.release();
                }
                state.press = None;
            }
        }
    }
}

impl Dispatch<WlPointer, ()> for State {
    fn event(state: &mut Self, _: &WlPointer, event: wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_pointer::Event::Button { serial, state: WEnum::Value(wl_pointer::ButtonState::Pressed), .. } = event {
            state.press = Some(serial);
        }
    }
}

impl Dispatch<WlDataDeviceManager, ()> for State {
    fn event(_: &mut Self, _: &WlDataDeviceManager, _: <WlDataDeviceManager as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlDataDevice, ()> for State {
    fn event(_: &mut Self, _: &WlDataDevice, event: wl_data_device::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        // This device only starts drags; offers the compositor announces to it are let go.
        match event {
            wl_data_device::Event::Selection { id: Some(offer) } | wl_data_device::Event::Enter { id: Some(offer), .. } => offer.destroy(),
            _ => {}
        }
    }

    wayland_client::event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, ()),
    ]);
}

impl Dispatch<WlDataOffer, ()> for State {
    fn event(_: &mut Self, _: &WlDataOffer, _: <WlDataOffer as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlDataSource, Vec<u8>> for State {
    fn event(state: &mut Self, source: &WlDataSource, event: wl_data_source::Event, uri: &Vec<u8>, _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_data_source::Event::Send { mime_type, fd } if mime_type == URI_LIST => {
                let _ = std::fs::File::from(fd).write_all(uri);
            }
            wl_data_source::Event::Cancelled | wl_data_source::Event::DndFinished => {
                source.destroy();
                state.drags = state.drags.saturating_sub(1);
            }
            _ => {}
        }
    }
}

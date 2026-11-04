// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! The main window. Drawing code never changes state directly: it records `Intent`s, which
//! are applied once the frame is laid out, so every change goes through one place.

mod agent_panel;
mod chrome;
mod dialogs;
mod dock;
mod monitor_view;
mod opening;
mod process_window;
mod palette;
mod remote_panel;
pub mod shortcuts;
mod sidebar;
pub mod style;
mod terminals;

use crate::backend::{Backend, Notice, Open, Session};
use crate::core::profile::{Kind, Profile, Store};
use crate::core::settings::{Scene, Settings, scene};
use crate::core::ssh_config;
use crate::core::workspace::{self, Workspace};
use crate::core::{paths, text};
use crate::glyphs::Fonts;
use crate::gpu::{self, Instance};
use crate::layout::Node;
use crate::terminal::{Options, SearchOptions, Size};
use crate::theme::{self, Theme};
use alacritty_terminal::vte::ansi::CursorShape;
use eframe::egui::{self, ViewportCommand};
use eframe::egui_wgpu;
use shortcuts::Action;
use opening::{Pending, Placement};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use style::Palette;

/// A terminal pane: its session plus what the interface knows and caches about it.
pub struct Pane {
    pub session: Session,
    pub key: u64,
    pub title: String,
    pub term_title: Option<String>,
    pub live_cwd: Option<String>,
    pub exit: Option<Option<i64>>,
    quads: Arc<Vec<Instance>>,
    built: BuildKey,
    version: u64,
}

#[derive(Clone, Copy, PartialEq)]
struct BuildKey {
    generation: u64,
    size: [u32; 2],
    focused: bool,
    dim: bool,
    cursor_on: bool,
    theme: *const Theme,
    cell: (u32, u32),
    atlas: u64,
    alpha: u32,
}

impl Pane {
    pub fn id(&self) -> &str {
        &self.session.terminal.id
    }
    pub fn running(&self) -> bool {
        self.exit.is_none()
    }
    pub fn cwd(&self) -> &str {
        self.live_cwd.as_deref().unwrap_or(&self.session.cwd)
    }
    pub fn label(&self) -> String {
        self.term_title.clone().unwrap_or_else(|| self.title.clone())
    }
}

pub struct Tab {
    pub id: u64,
    pub root: Node,
    pub focus: String,
    pub title: String,
    pub broadcast: bool,
    pub zoomed: bool,
}

pub enum Dialog {
    Settings,
    Connection(Box<dialogs::ConnectionForm>),
    Delete { profile: Profile, error: Option<String> },
    SshImport(dialogs::ImportState),
}

#[derive(Default)]
pub struct Search {
    pub open: bool,
    pub value: String,
    pub found: bool,
    pub options: SearchOptions,
    pub focus: bool,
}

pub struct Toast {
    pub message: String,
    pub error: bool,
    pub until: Instant,
}

/// Something the user asked for while a frame was drawn. Intents live for one frame, so the
/// size of their largest variant costs nothing worth an allocation.
#[allow(clippy::large_enum_variant)]
pub enum Intent {
    Action(Action),
    NewTab { kind: Kind, profile: Option<Profile>, cwd: Option<String> },
    SelectTab(u64),
    CloseTab(u64),
    RenameTab(u64, String),
    MoveTab { from: u64, to: u64 },
    Focus(String),
    SetRatio { tab: u64, path: Vec<usize>, ratio: f32 },
    ChooseDirectory,
    Dialog(Dialog),
    CloseDialog,
    SaveProfile { profile: Profile, connect: bool, persist: bool },
    DeleteProfile(String),
    ImportHosts { aliases: Vec<String> },
    NeverAskImport,
    OpenSshImport,
    Copy,
    Paste,
    SelectAll,
    Find { backwards: bool },
    CloseSearch,
    Notify(String, bool),
    Relaunch,
    ChooseBackground,
    ClearBackground,
    Window(ViewportCommand),
    SetTheme(&'static str),
    /// A pane dragged onto a side of another pane, or onto the tab strip (`None`) for a tab.
    MovePane { id: String, target: Option<(String, crate::layout::Direction)> },
    DockPane { id: String, edge: dock::Edge },
    OpenProcesses,
    /// Types the readiness marks script into a terminal; the user submits it with Enter.
    StageIntegration(String),
    AllowAutoSubmit { id: String, enabled: bool },
    ResetDock,
    /// Hands a downloaded file to the OS as a drag out of the window.
    DragOut(std::path::PathBuf),
}

pub struct App {
    backend: Backend,
    render_state: egui_wgpu::RenderState,
    fonts: Fonts,
    scale: f32,
    settings: Settings,
    saved_settings: Settings,
    /// Whether this window was created transparent; changing it needs a restart.
    window_transparent: bool,
    panes: HashMap<String, Pane>,
    tabs: Vec<Tab>,
    active: Option<u64>,
    next_id: u64,
    pending: Pending,
    profiles: Vec<Profile>,
    store: Store,
    cwd: String,
    home: String,
    hostname: String,
    sidebar_visible: bool,
    agent_visible: bool,
    focus_mode: bool,
    dialog: Option<Dialog>,
    palette: Option<palette::State>,
    search: Search,
    toast: Option<Toast>,
    selecting: Option<String>,
    preedit: String,
    seen_hosts: HashSet<String>,
    workspace_dirty: Option<Instant>,
    remotes: Arc<crate::remote::manager::Manager>,
    agent: Arc<crate::agent::service::Service>,
    monitors: Arc<crate::monitor::service::Service>,
    monitor_visible: bool,
    monitor_form: Option<monitor_view::Form>,
    pending_seen: HashSet<String>,
    processes: process_window::ProcessWindow,
    dock: dock::Dock,
    dragging_panel: Option<dock::Panel>,
    dragging_pane: Option<String>,
    /// A file to drag out of the window once this frame's intents are applied.
    drag_out: Option<std::path::PathBuf>,
    /// Terminal panes docked on a window edge, beside the panels, outside every tab.
    docked: Vec<(String, dock::Edge)>,
    /// The docked pane with the keyboard, when one has it rather than the active tab.
    docked_focus: Option<String>,
    /// Glyphs rasterized this frame, uploaded by whichever pane's callback prepares first.
    frame_uploads: Arc<std::sync::Mutex<Vec<crate::glyphs::Upload>>>,
    agent_state: agent_panel::AgentState,
    workspace_layout: Option<serde_json::Value>,
    file_panels: HashMap<String, remote_panel::PanelState>,
    sidebar_files: bool,
    watched: Option<String>,
    background: Option<(std::path::PathBuf, egui::TextureHandle)>,
    applied_theme: Option<&'static str>,
    blink_epoch: Instant,
    smoke: Option<crate::smoke::Smoke>,
    pub mac: bool,
    windows: bool,
}

const UI_STATE: &str = "native-ui.json";

#[derive(serde::Serialize, serde::Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct UiState {
    sidebar_hidden: bool,
    agent_hidden: bool,
    seen_ssh_hosts: Vec<String>,
    agent_profile: Option<String>,
    monitor_visible: bool,
    dock: Option<dock::Dock>,
}

fn next_counter(counter: &mut u64) -> u64 {
    *counter += 1;
    *counter
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, settings: Settings, window_transparent: bool, smoke: Option<crate::smoke::Smoke>) -> Self {
        let render_state = cc.wgpu_render_state.clone().expect("omniTerm draws with wgpu");
        gpu::install(&render_state);
        style::install_fonts(&cc.egui_ctx);
        let context = cc.egui_ctx.clone();
        let backend = Backend::new(move || context.request_repaint(), crate::remote::control_path);
        let context = cc.egui_ctx.clone();
        let remotes = Arc::new(crate::remote::manager::Manager::new(move || context.request_repaint()));
        let context = cc.egui_ctx.clone();
        let agent = crate::agent::service::Service::new(backend.host().clone(), paths::file("agent-home"), move || context.request_repaint());
        let context = cc.egui_ctx.clone();
        let monitors = crate::monitor::service::Service::new(backend.host().clone(), remotes.clone(), move || context.request_repaint());
        agent.set_monitors(monitors.clone());
        let scale = cc.egui_ctx.pixels_per_point();
        let ui_state: UiState = std::fs::read(paths::file(UI_STATE)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
        let store = Store::new(paths::file("profiles.json"));
        let home = paths::home().to_string_lossy().into_owned();
        let cwd = std::env::current_dir().ok().map(|dir| dir.to_string_lossy().into_owned()).filter(|dir| dir != "/" && std::path::Path::new(dir).is_dir()).unwrap_or_else(|| home.clone());
        let fonts = Fonts::new(settings.font_size * scale, settings.line_height, settings.font_family.candidates());
        let mut app = Self {
            backend,
            render_state,
            fonts,
            scale,
            saved_settings: settings.clone(),
            settings,
            window_transparent,
            panes: HashMap::new(),
            tabs: Vec::new(),
            active: None,
            next_id: 0,
            pending: Pending::default(),
            profiles: Vec::new(),
            store,
            cwd,
            hostname: hostname(),
            home,
            sidebar_visible: !ui_state.sidebar_hidden,
            agent_visible: !ui_state.agent_hidden,
            focus_mode: false,
            dialog: None,
            palette: None,
            search: Search { found: true, ..Search::default() },
            toast: None,
            selecting: None,
            preedit: String::new(),
            seen_hosts: ui_state.seen_ssh_hosts.into_iter().collect(),
            workspace_dirty: None,
            remotes,
            agent,
            monitors,
            monitor_visible: ui_state.monitor_visible,
            monitor_form: None,
            pending_seen: HashSet::new(),
            processes: process_window::ProcessWindow::default(),
            dock: ui_state.dock.clone().unwrap_or_default(),
            dragging_panel: None,
            dragging_pane: None,
            drag_out: None,
            docked: Vec::new(),
            docked_focus: None,
            frame_uploads: Arc::default(),
            agent_state: agent_panel::AgentState::with_profile(ui_state.agent_profile.clone()),
            workspace_layout: None,
            file_panels: HashMap::new(),
            sidebar_files: true,
            watched: None,
            background: None,
            applied_theme: None,
            blink_epoch: Instant::now(),
            smoke,
            mac: cfg!(target_os = "macos"),
            windows: cfg!(windows),
        };
        match app.store.list() {
            Ok(profiles) => app.profiles = profiles,
            Err(error) => app.notify(error, true),
        }
        app.start();
        app
    }

    /// The first launch layout: the saved workspace, or one shell.
    fn start(&mut self) {
        let restored = self.settings.restore_workspace && self.smoke.is_none() && self.restore();
        if !restored {
            self.open(Kind::Shell, None, None, Placement::NewTab);
        }
        if self.settings.ssh_config_prompt && self.smoke.is_none() {
            let config = ssh_config::read(std::path::Path::new(&self.home));
            let hosts: Vec<_> = config
                .hosts
                .into_iter()
                .filter(|host| !self.seen_hosts.contains(&host.alias) && !self.profiles.iter().any(|profile| profile.kind == Some(Kind::Ssh) && profile.host.as_deref() == Some(&host.alias)))
                .collect();
            if !hosts.is_empty() {
                self.dialog = Some(Dialog::SshImport(dialogs::ImportState::new(config.path, hosts, true)));
            }
        }
    }

    /// Opens a fresh shell for every saved pane. A directory that no longer exists falls back
    /// to the default one instead of losing the pane.
    fn restore(&mut self) -> bool {
        let Some(saved) = Workspace::load() else { return false };
        let mut missing = 0;
        let mut restored = Vec::new();
        for tab in &saved.tabs {
            let root = workspace::restore(&tab.root, &mut |cwd| {
                let candidates: Vec<String> = if !cwd.is_empty() && cwd != self.cwd { vec![cwd.to_string(), self.cwd.clone()] } else { vec![self.cwd.clone()] };
                for (index, candidate) in candidates.iter().enumerate() {
                    if let Ok(session) = self.open_now(Kind::Shell, None, Some(candidate.clone())) {
                        missing += usize::from(index > 0);
                        let id = session.terminal.id.clone();
                        self.attach(session);
                        return Some(id);
                    }
                }
                None
            });
            if let Some(root) = root {
                let focus = root.panes()[0].clone();
                let id = next_counter(&mut self.next_id);
                restored.push(Tab { id, root, focus, title: tab.title.clone(), broadcast: false, zoomed: false });
            }
        }
        if restored.is_empty() {
            return false;
        }
        self.active = restored.get(saved.active.min(restored.len() - 1)).map(|tab| tab.id);
        self.tabs = restored;
        if missing > 0 {
            self.notify(format!("{missing} 个窗格的上次目录已不存在，已在默认目录打开。"), true);
        }
        true
    }

    fn terminal_options(&self) -> Options {
        let cursor = match self.settings.cursor_style {
            crate::core::settings::CursorStyle::Bar => CursorShape::Beam,
            crate::core::settings::CursorStyle::Block => CursorShape::Block,
            crate::core::settings::CursorStyle::Underline => CursorShape::Underline,
        };
        Options { scrollback: self.settings.scrollback, cursor, blinking: self.settings.cursor_blink }
    }

    /// The grid size a new pane will likely get: the focused pane's, before the first layout.
    fn likely_size(&self) -> Size {
        self.focused().map(|pane| {
            let term = pane.session.terminal.lock();
            use alacritty_terminal::grid::Dimensions;
            Size { cols: term.columns(), rows: term.screen_lines() }
        }).unwrap_or(Size { cols: 100, rows: 30 })
    }

    fn request(&self, kind: Kind, profile: Option<Profile>, cwd: Option<String>) -> Open {
        Open {
            token: 0,
            kind,
            cwd: cwd.unwrap_or_else(|| self.cwd.clone()),
            profile,
            companion: kind.remote() && self.settings.remote_panels,
            local_shell: self.settings.local_shell.clone(),
            size: self.likely_size(),
            options: self.terminal_options(),
        }
    }

    fn open(&mut self, kind: Kind, profile: Option<Profile>, cwd: Option<String>, placement: Placement) {
        let mut open = self.request(kind, profile, cwd);
        open.token = next_counter(&mut self.next_id);
        self.pending.insert(open.token, placement);
        self.backend.open(open);
    }

    fn open_now(&self, kind: Kind, profile: Option<Profile>, cwd: Option<String>) -> Result<Session, String> {
        self.backend.open_blocking(self.request(kind, profile, cwd))
    }

    fn attach(&mut self, session: Session) -> String {
        let id = session.terminal.id.clone();
        if let Some(companion) = session.companion.clone() {
            self.remotes.attach(&id, companion);
        }
        let local = self.panes.values().filter(|pane| pane.session.kind == Kind::Shell).count() + 1;
        let title = match session.kind {
            Kind::Shell if self.panes.is_empty() => "本地终端".to_string(),
            Kind::Shell => format!("本地终端 {local}"),
            _ => session.title.clone(),
        };
        let key = next_counter(&mut self.next_id);
        self.panes.insert(id.clone(), Pane {
            session,
            key,
            title,
            term_title: None,
            live_cwd: None,
            exit: None,
            quads: Arc::default(),
            built: BuildKey { generation: 0, size: [0, 0], focused: false, dim: false, cursor_on: false, theme: std::ptr::null(), cell: (0, 0), atlas: u64::MAX, alpha: 0 },
            version: 0,
        });
        self.mark_workspace();
        id
    }

    fn opened(&mut self, token: u64, result: Result<Box<Session>, String>) {
        let (placement, session) = match self.pending.complete(token, result, &self.tabs, &self.backend) {
            Ok(Some(completed)) => completed,
            Ok(None) => return,
            Err(error) => {
                self.notify(error, true);
                return;
            }
        };
        let id = self.attach(*session);
        match placement {
            Placement::Split { tab, target, row } => {
                let tab = self.tab_mut(tab).expect("checked");
                tab.root = tab.root.split(&target, &id, row);
                tab.focus = id;
                tab.zoomed = false;
            }
            Placement::NewTab => {
                let tab = Tab { id: next_counter(&mut self.next_id), root: Node::Pane(id.clone()), focus: id, title: String::new(), broadcast: false, zoomed: false };
                self.active = Some(tab.id);
                self.tabs.push(tab);
            }
        }
        self.mark_workspace();
    }

    pub fn tab(&self, id: u64) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    fn tab_mut(&mut self, id: u64) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.active.and_then(|id| self.tab(id))
    }

    pub fn focused(&self) -> Option<&Pane> {
        if let Some(pane) = self.docked_focus.as_ref().and_then(|id| self.panes.get(id)) {
            return Some(pane);
        }
        self.active_tab().and_then(|tab| self.panes.get(&tab.focus))
    }

    pub fn is_docked(&self, id: &str) -> bool {
        self.docked.iter().any(|(docked, _)| docked == id)
    }

    /// Takes a pane out of whatever tab holds it; an emptied tab goes away.
    fn take_from_tabs(&mut self, id: &str) {
        let Some(source) = self.tab_of(id) else { return };
        if let Some(tab) = self.tab_mut(source) {
            match tab.root.remove(id) {
                Some(root) => {
                    if tab.focus == id {
                        tab.focus = root.panes()[0].clone();
                    }
                    tab.zoomed &= root.panes().len() > 1;
                    tab.root = root;
                }
                None => self.drop_tab(source),
            }
        }
    }

    /// Docks a pane on a window edge, keeping its live session and view.
    fn dock_pane(&mut self, id: &str, edge: dock::Edge) {
        if !self.panes.contains_key(id) {
            return;
        }
        self.take_from_tabs(id);
        self.docked.retain(|(docked, _)| docked != id);
        self.docked.push((id.to_string(), edge));
        self.docked_focus = Some(id.to_string());
        self.mark_workspace();
    }

    fn tab_of(&self, pane: &str) -> Option<u64> {
        self.tabs.iter().find(|tab| tab.root.panes().iter().any(|id| id == pane)).map(|tab| tab.id)
    }

    fn notify(&mut self, message: impl Into<String>, error: bool) {
        self.toast = Some(Toast { message: message.into(), error, until: Instant::now() + Duration::from_millis(6500) });
    }

    fn mark_workspace(&mut self) {
        self.workspace_dirty.get_or_insert_with(Instant::now);
    }

    fn split(&mut self, row: bool) {
        let Some(tab) = self.active_tab() else {
            self.open(Kind::Shell, None, None, Placement::NewTab);
            return;
        };
        let (tab_id, target) = (tab.id, tab.focus.clone());
        // Remote sessions duplicate their connection, like iTerm's "split with current
        // profile". Serial devices and tunnels cannot be opened twice.
        let (kind, profile, cwd) = match self.panes.get(&target) {
            Some(pane) if matches!(pane.session.kind, Kind::Ssh | Kind::Sftp) && pane.session.profile.is_some() => (pane.session.kind, pane.session.profile.clone(), None),
            Some(pane) if pane.session.kind == Kind::Shell => (Kind::Shell, None, Some(pane.cwd().to_string())),
            _ => (Kind::Shell, None, None),
        };
        self.open(kind, profile, cwd, Placement::Split { tab: tab_id, target, row });
    }

    fn close_pane(&mut self, id: &str) {
        self.pending.cancel_pane(id);
        self.docked.retain(|(docked, _)| docked != id);
        if self.docked_focus.as_deref() == Some(id) {
            self.docked_focus = None;
        }
        self.remotes.detach(id);
        self.file_panels.remove(id);
        self.backend.close(id);
        self.panes.remove(id);
        if self.search.open && self.focused().is_none_or(|pane| pane.id() == id) {
            self.search.open = false;
        }
        let mut emptied = None;
        for tab in &mut self.tabs {
            if !tab.root.panes().iter().any(|pane| pane == id) {
                continue;
            }
            match tab.root.remove(id) {
                Some(root) => {
                    if tab.focus == id {
                        tab.focus = root.neighbor(id, &tab.root).unwrap_or_else(|| root.panes()[0].clone());
                    }
                    tab.zoomed &= root.panes().len() > 1;
                    tab.root = root;
                }
                None => emptied = Some(tab.id),
            }
        }
        if let Some(tab) = emptied {
            self.drop_tab(tab);
        }
        let live: Vec<u64> = self.panes.values().map(|pane| pane.key).collect();
        gpu::forget(&self.render_state, &live);
        self.mark_workspace();
    }

    fn close_tab(&mut self, tab: u64) {
        let Some(ids) = self.tab(tab).map(|tab| tab.root.panes()) else { return };
        for id in ids {
            self.remotes.detach(&id);
            self.file_panels.remove(&id);
            self.backend.close(&id);
            self.panes.remove(&id);
        }
        self.drop_tab(tab);
        let live: Vec<u64> = self.panes.values().map(|pane| pane.key).collect();
        gpu::forget(&self.render_state, &live);
        self.mark_workspace();
    }

    /// Removes a tab whose sessions are already closed and moves focus to a neighbor.
    fn drop_tab(&mut self, tab: u64) {
        self.pending.cancel_tab(tab);
        let Some(index) = self.tabs.iter().position(|item| item.id == tab) else { return };
        self.tabs.remove(index);
        if self.active == Some(tab) {
            self.active = self.tabs.get(index.min(self.tabs.len().saturating_sub(1))).map(|tab| tab.id);
        }
    }

    fn focus_pane(&mut self, id: &str) {
        if self.is_docked(id) {
            self.docked_focus = Some(id.to_string());
            return;
        }
        self.docked_focus = None;
        if let Some(tab) = self.tab_of(id) {
            self.active = Some(tab);
            // A zoomed tab shows its focused pane, so the zoom follows the focus.
            if let Some(tab) = self.tab_mut(tab) {
                tab.focus = id.to_string();
            }
        }
    }

    fn cycle_pane(&mut self, step: isize) {
        let Some(tab) = self.active_tab() else { return };
        let ids = tab.root.panes();
        let at = ids.iter().position(|id| *id == tab.focus).unwrap_or(0) as isize;
        let next = ids[((at + step).rem_euclid(ids.len() as isize)) as usize].clone();
        self.focus_pane(&next);
    }

    fn move_focus(&mut self, direction: crate::layout::Direction) {
        let Some(tab) = self.active_tab() else { return };
        if let Some(next) = tab.root.neighbor_in(&tab.focus, direction) {
            self.focus_pane(&next);
        }
    }

    /// Writes typed input to the focused pane, or to every live pane of the tab while
    /// broadcasting. Replies the emulator generates go only to their own pane (see backend).
    fn type_text(&mut self, data: String) {
        if let Some(id) = self.docked_focus.clone().filter(|id| self.panes.contains_key(id)) {
            if let Some(pane) = self.panes.get(&id) {
                pane.session.terminal.clear_selection();
                pane.session.terminal.scroll_to_bottom();
            }
            self.backend.write(&id, data);
            return;
        }
        let Some(tab) = self.active_tab() else { return };
        let targets: Vec<String> = if tab.broadcast { tab.root.panes().into_iter().filter(|id| self.panes.get(id).is_some_and(|pane| pane.running() || *id == tab.focus)).collect() } else { vec![tab.focus.clone()] };
        for id in targets {
            if let Some(pane) = self.panes.get(&id) {
                pane.session.terminal.clear_selection();
                pane.session.terminal.scroll_to_bottom();
            }
            self.backend.write(&id, data.clone());
        }
    }

    fn paste(&mut self, text: &str) {
        let Some(pane) = self.focused() else { return };
        if !pane.running() {
            self.notify("该会话已结束，请新建终端后再粘贴。", true);
            return;
        }
        let bracketed = pane.session.terminal.mode().contains(alacritty_terminal::term::TermMode::BRACKETED_PASTE);
        self.type_text(crate::input::paste_bytes(text, bracketed));
    }

    fn copy(&mut self, ctx: &egui::Context) -> bool {
        match self.focused().and_then(|pane| pane.session.terminal.selection_text()) {
            Some(text) => {
                ctx.copy_text(text);
                true
            }
            None => false,
        }
    }

    fn set_font_size(&mut self, size: f32) {
        self.settings.font_size = size.clamp(crate::core::settings::MIN_FONT, crate::core::settings::MAX_FONT).round();
    }

    fn toggle_focus_mode(&mut self, ctx: &egui::Context) {
        self.focus_mode = !self.focus_mode;
        ctx.send_viewport_cmd(ViewportCommand::Fullscreen(self.focus_mode));
    }

    fn run(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::NewTab => self.open(Kind::Shell, None, None, Placement::NewTab),
            Action::ClosePane => {
                if let Some(id) = self.focused().map(|pane| pane.id().to_string()) {
                    self.close_pane(&id);
                }
            }
            Action::SplitRight => self.split(true),
            Action::SplitDown => self.split(false),
            Action::NextPane => self.cycle_pane(1),
            Action::PreviousPane => self.cycle_pane(-1),
            Action::PaneLeft => self.move_focus(crate::layout::Direction::Left),
            Action::PaneRight => self.move_focus(crate::layout::Direction::Right),
            Action::PaneUp => self.move_focus(crate::layout::Direction::Up),
            Action::PaneDown => self.move_focus(crate::layout::Direction::Down),
            Action::MaximizePane => {
                if let Some(id) = self.active
                    && let Some(tab) = self.tab_mut(id)
                    && tab.root.panes().len() > 1
                {
                    tab.zoomed = !tab.zoomed;
                }
            }
            Action::Find => {
                if self.focused().is_some() {
                    self.search.open = true;
                    self.search.focus = true;
                }
            }
            Action::Clear => {
                if let Some(pane) = self.focused() {
                    pane.session.terminal.clear();
                }
            }
            Action::Palette => self.palette = Some(palette::State::default()),
            Action::Broadcast => {
                if let Some(id) = self.active
                    && let Some(tab) = self.tab_mut(id)
                {
                    tab.broadcast = !tab.broadcast;
                    let enabled = tab.broadcast;
                    self.notify(if enabled { "广播输入已开启：键盘输入会发送到此标签内的所有窗格。" } else { "广播输入已关闭。" }, false);
                }
            }
            Action::SendToAgent => {
                let selection = self.focused().and_then(|pane| pane.session.terminal.selection_text()).map(|text| text.trim().to_string()).filter(|text| !text.is_empty());
                match selection {
                    None => self.notify("请先在终端中选择要附加给 Agent 的内容。", true),
                    Some(selection) => {
                        if let Some(id) = self.focused().map(|pane| pane.id().to_string()) {
                            self.agent_state.attach(&id, selection);
                        }
                        self.agent_visible = true;
                        self.notify("已附加选中内容，随下一条消息发送给 omniTerm Agent。", false);
                    }
                }
            }
            Action::ToggleAgent => self.agent_visible = !self.agent_visible,
            Action::ToggleSidebar => self.sidebar_visible = !self.sidebar_visible,
            Action::ToggleFullscreen => self.toggle_focus_mode(ctx),
            Action::FontBigger => self.set_font_size(self.settings.font_size + 1.0),
            Action::FontSmaller => self.set_font_size(self.settings.font_size - 1.0),
            Action::FontReset => self.set_font_size(Settings::default().font_size),
            Action::Settings => self.dialog = Some(Dialog::Settings),
        }
    }

    fn apply(&mut self, ctx: &egui::Context, intent: Intent) {
        match intent {
            Intent::Action(action) => self.run(ctx, action),
            Intent::NewTab { kind, profile, cwd } => self.open(kind, profile, cwd, Placement::NewTab),
            Intent::SelectTab(id) => self.active = Some(id),
            Intent::CloseTab(id) => self.close_tab(id),
            Intent::RenameTab(id, title) => {
                if let Some(tab) = self.tab_mut(id) {
                    tab.title = title.trim().chars().take(60).collect();
                }
                self.mark_workspace();
            }
            Intent::MoveTab { from, to } => {
                if let (Some(from), Some(to)) = (self.tabs.iter().position(|tab| tab.id == from), self.tabs.iter().position(|tab| tab.id == to)) {
                    let tab = self.tabs.remove(from);
                    self.tabs.insert(to, tab);
                    self.mark_workspace();
                }
            }
            Intent::Focus(id) => self.focus_pane(&id),
            Intent::SetRatio { tab, path, ratio } => {
                if let Some(tab) = self.tab_mut(tab) {
                    tab.root.set_ratio(&path, ratio);
                }
                self.mark_workspace();
            }
            Intent::ChooseDirectory => {
                if let Some(directory) = rfd::FileDialog::new().set_title("打开工作文件夹").set_directory(&self.cwd).pick_folder() {
                    self.cwd = directory.to_string_lossy().into_owned();
                    self.open(Kind::Shell, None, Some(self.cwd.clone()), Placement::NewTab);
                }
            }
            Intent::Dialog(dialog) => self.dialog = Some(dialog),
            // Closing the import list counts as having seen those hosts, so it asks only once.
            Intent::CloseDialog => {
                if matches!(self.dialog, Some(Dialog::SshImport(_))) {
                    self.finish_import();
                } else {
                    self.dialog = None;
                }
            }
            Intent::SaveProfile { profile, connect, persist } => {
                let saved = if persist || !connect { self.store.save(&profile).map(Some) } else { profile.validate(true).map(|_| None) };
                match saved {
                    Ok(list) => {
                        if let Some(list) = list {
                            self.profiles = list;
                        }
                        if connect {
                            self.open(profile.kind.unwrap_or(Kind::Ssh), Some(profile), None, Placement::NewTab);
                        }
                        self.dialog = None;
                    }
                    Err(error) => {
                        if let Some(Dialog::Connection(form)) = &mut self.dialog {
                            form.error = Some(error);
                        }
                    }
                }
            }
            Intent::DeleteProfile(id) => match self.store.delete(&id) {
                Ok(list) => {
                    self.profiles = list;
                    self.dialog = None;
                }
                Err(error) => {
                    if let Some(Dialog::Delete { error: slot, .. }) = &mut self.dialog {
                        *slot = Some(error);
                    }
                }
            },
            Intent::ImportHosts { aliases } => {
                let profiles: Vec<Profile> = aliases.iter().map(|alias| Profile { kind: Some(Kind::Ssh), name: Some(alias.clone()), host: Some(alias.clone()), ..Profile::default() }).collect();
                match self.store.import(&profiles) {
                    Ok(list) => {
                        self.profiles = list;
                        self.finish_import();
                        self.notify(format!("已导入 {} 个 SSH 主机。", aliases.len()), false);
                    }
                    Err(error) => {
                        if let Some(Dialog::SshImport(state)) = &mut self.dialog {
                            state.error = Some(error);
                        }
                    }
                }
            }
            Intent::NeverAskImport => {
                self.settings.ssh_config_prompt = false;
                self.finish_import();
            }
            Intent::OpenSshImport => {
                let config = ssh_config::read(std::path::Path::new(&self.home));
                if let Some(error) = config.error {
                    self.notify(error, true);
                    return;
                }
                let total = config.hosts.len();
                let hosts: Vec<_> = config.hosts.into_iter().filter(|host| !self.profiles.iter().any(|profile| profile.kind == Some(Kind::Ssh) && profile.host.as_deref() == Some(&host.alias))).collect();
                if hosts.is_empty() {
                    self.notify(if total > 0 { "~/.ssh/config 中的主机都已导入。" } else { "~/.ssh/config 中没有可导入的主机。" }, false);
                } else {
                    self.dialog = Some(Dialog::SshImport(dialogs::ImportState::new(config.path, hosts, false)));
                }
            }
            Intent::Copy => {
                if !self.copy(ctx) {
                    self.notify("请先在终端中选择要复制的文本。", true);
                }
            }
            Intent::Paste => match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
                Ok(text) => self.paste(&text),
                Err(error) => self.notify(format!("无法读取剪贴板：{error}"), true),
            },
            Intent::SelectAll => {
                if let Some(pane) = self.focused() {
                    pane.session.terminal.select_all();
                }
            }
            Intent::Find { backwards } => {
                if let Some(pane) = self.focused() {
                    self.search.found = pane.session.terminal.find(&self.search.value, backwards, self.search.options);
                }
            }
            Intent::CloseSearch => {
                self.search.open = false;
                if let Some(pane) = self.focused() {
                    pane.session.terminal.clear_selection();
                }
            }
            Intent::Notify(message, error) => self.notify(message, error),
            Intent::Relaunch => relaunch(ctx),
            Intent::ChooseBackground => self.choose_background(),
            Intent::ClearBackground => {
                if let Some(image) = self.settings.background_image.take() {
                    let _ = std::fs::remove_file(image.path);
                }
                self.background = None;
            }
            Intent::Window(command) => ctx.send_viewport_cmd(command),
            Intent::SetTheme(id) => self.settings.theme = id.to_string(),
            Intent::OpenProcesses => self.open_processes(),
            Intent::StageIntegration(id) => match self.panes.get(&id).and_then(|pane| crate::backend::shell_integration(&pane.session.nonce)) {
                Some(script) => {
                    self.backend.write(&id, script);
                    self.focus_pane(&id);
                    self.notify("脚本已输入到该终端，确认后按回车启用。", false);
                }
                None => self.notify("这个终端没有可用的就绪标记。", true),
            },
            Intent::AllowAutoSubmit { id, enabled } => {
                if let Err(error) = self.backend.request("session.allowAutoSubmit", serde_json::json!({ "id": id, "enabled": enabled })) {
                    self.notify(error, true);
                }
            }
            Intent::MovePane { id, target } => self.move_pane(&id, target),
            Intent::DockPane { id, edge } => self.dock_pane(&id, edge),
            Intent::ResetDock => self.dock = dock::Dock::default(),
            Intent::DragOut(file) => self.drag_out = Some(file),
        }
    }

    fn finish_import(&mut self) {
        if let Some(Dialog::SshImport(state)) = self.dialog.take() {
            self.seen_hosts.extend(state.hosts.into_iter().map(|host| host.alias));
        }
    }

    /// Copies the chosen image into the data directory, as the desktop app does, so moving
    /// or deleting the original keeps the background.
    fn choose_background(&mut self) {
        let Some(path) = rfd::FileDialog::new().set_title("选择背景图片").add_filter("图片", &["png", "jpg", "jpeg", "webp", "gif", "bmp"]).pick_file() else { return };
        let result = (|| -> Result<crate::core::settings::BackgroundImage, String> {
            let size = std::fs::metadata(&path).map_err(|error| error.to_string())?.len();
            if size > 32 << 20 {
                return Err("图片不能超过 32 MB。".into());
            }
            let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
            image::load_from_memory(&bytes).map_err(|_| "无法读取这张图片。".to_string())?;
            let extension = path.extension().and_then(|extension| extension.to_str()).unwrap_or("png").to_ascii_lowercase();
            let directory = paths::file("backgrounds");
            let target = directory.join(format!("{}.{extension}", uuid::Uuid::new_v4()));
            paths::write_atomically(&target, &bytes).map_err(|error| error.to_string())?;
            Ok(crate::core::settings::BackgroundImage { path: target, name: path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default() })
        })();
        match result {
            Ok(image) => {
                if let Some(old) = self.settings.background_image.replace(image) {
                    let _ = std::fs::remove_file(old.path);
                }
                self.background = None;
            }
            Err(error) => self.notify(error, true),
        }
    }

    fn handle_notices(&mut self) {
        while let Ok(notice) = self.backend.notices.try_recv() {
            match notice {
                Notice::Opened { token, result } => self.opened(token, result),
                Notice::Exited { id, code } => {
                    if let Some(pane) = self.panes.get_mut(&id) {
                        pane.exit = Some(code);
                    }
                }
                Notice::Title { id, title } => {
                    if let Some(pane) = self.panes.get_mut(&id) {
                        pane.term_title = title;
                    }
                }
                Notice::Directory { id, url } => {
                    let cwd = text::cwd_from_osc7(&url, self.windows, &self.hostname);
                    if let (Some(pane), Some(cwd)) = (self.panes.get_mut(&id), cwd) {
                        pane.live_cwd = Some(cwd);
                        self.mark_workspace();
                    }
                }
                Notice::Host(value) => {
                    if value["event"] == "operation.update" {
                        self.monitors.operation_changed(&value["params"]);
                        self.agent.operation_changed(value["params"].clone());
                    }
                }
            }
        }
    }

    /// Settings take effect everywhere they matter, and are saved once they change.
    fn sync_settings(&mut self) {
        if self.settings == self.saved_settings {
            return;
        }
        let before = std::mem::replace(&mut self.saved_settings, self.settings.clone());
        if before.font_family != self.settings.font_family {
            self.fonts = Fonts::new(self.settings.font_size * self.scale, self.settings.line_height, self.settings.font_family.candidates());
        } else {
            self.fonts.configure(self.settings.font_size * self.scale, self.settings.line_height);
        }
        if (before.scrollback, before.cursor_style, before.cursor_blink) != (self.settings.scrollback, self.settings.cursor_style, self.settings.cursor_blink) {
            let options = self.terminal_options();
            for pane in self.panes.values() {
                pane.session.terminal.set_options(options);
            }
        }
        if before.restore_workspace && !self.settings.restore_workspace {
            Workspace::clear();
        }
        if let Err(error) = self.settings.save() {
            self.notify(error, true);
        }
    }

    fn save_workspace(&mut self, force: bool) {
        let Some(since) = self.workspace_dirty else { return };
        if !force && since.elapsed() < Duration::from_millis(400) {
            return;
        }
        self.workspace_dirty = None;
        if !self.settings.restore_workspace || self.smoke.is_some() {
            return;
        }
        let cwd_of = |id: &str| self.panes.get(id).filter(|pane| pane.session.kind == Kind::Shell).map(|pane| pane.cwd().to_string());
        let mut tabs = Vec::new();
        let mut active = 0;
        // A docked pane comes back as a tab: its dock place belongs to a session that won't return.
        let docked: Vec<Tab> = self.docked.iter().map(|(id, _)| Tab { id: 0, root: Node::Pane(id.clone()), focus: id.clone(), title: String::new(), broadcast: false, zoomed: false }).collect();
        for tab in self.tabs.iter().chain(&docked) {
            if let Some(root) = workspace::snapshot(&tab.root, &cwd_of) {
                if Some(tab.id) == self.active {
                    active = tabs.len();
                }
                tabs.push(workspace::SavedTab { title: tab.title.clone(), root });
            }
        }
        Workspace { version: workspace::VERSION, active, tabs }.save();
    }

    fn save_ui_state(&self) {
        let state = UiState { sidebar_hidden: !self.sidebar_visible, agent_hidden: !self.agent_visible, seen_ssh_hosts: self.seen_hosts.iter().cloned().collect(), agent_profile: self.agent_state.profile.clone(), monitor_visible: self.monitor_visible, dock: Some(self.dock.clone()) };
        if let Ok(json) = serde_json::to_vec(&state) {
            let _ = paths::write_atomically(&paths::file(UI_STATE), &json);
        }
    }

    /// Moves a pane, with its live session, beside another pane or into a tab of its own.
    fn move_pane(&mut self, id: &str, target: Option<(String, crate::layout::Direction)>) {
        if target.as_ref().is_some_and(|(other, _)| other == id) || !self.panes.contains_key(id) {
            return;
        }
        let docked = self.is_docked(id);
        // A pane that is already its tab's only pane has nowhere new to go on the tab strip.
        if !docked && target.is_none() && self.tab_of(id).and_then(|tab| self.tab(tab)).is_some_and(|tab| tab.root.panes().len() == 1) {
            return;
        }
        // Out of its tab or the dock first; an emptied tab goes away.
        self.docked.retain(|(other, _)| other != id);
        if self.docked_focus.as_deref() == Some(id) {
            self.docked_focus = None;
        }
        self.take_from_tabs(id);
        match target {
            Some((other, side)) => {
                if let Some(destination) = self.tab_of(&other).and_then(|tab| self.tab_mut(tab)) {
                    destination.root = destination.root.insert_beside(&other, id, side);
                    destination.focus = id.to_string();
                    destination.zoomed = false;
                    self.active = Some(destination.id);
                }
            }
            None => {
                let tab = Tab { id: next_counter(&mut self.next_id), root: Node::Pane(id.to_string()), focus: id.to_string(), title: String::new(), broadcast: false, zoomed: false };
                self.active = Some(tab.id);
                self.tabs.push(tab);
            }
        }
        self.mark_workspace();
    }

    fn panel_visible(&self, panel: dock::Panel) -> bool {
        match panel {
            dock::Panel::Sidebar => self.sidebar_visible,
            dock::Panel::Agent => self.agent_visible,
            dock::Panel::Monitor => self.monitor_visible,
        }
    }

    pub fn open_processes(&mut self) {
        let host = self.focused().filter(|pane| pane.session.kind == Kind::Ssh && self.remotes.get(pane.id()).is_some()).map(|pane| pane.id().to_string());
        self.processes.open_for(host);
    }

    /// The SSH companion of the focused pane, when the file panel and metrics are on.
    pub fn focused_remote(&self) -> Option<Arc<crate::remote::manager::Remote>> {
        let pane = self.focused()?;
        if !self.settings.remote_panels {
            return None;
        }
        self.remotes.get(pane.id())
    }

    /// Only the focused SSH tab samples its host; the others stop until they are shown.
    fn watch_metrics(&mut self) {
        let focused = self.focused_remote().map(|remote| remote.id().to_string());
        if focused == self.watched {
            return;
        }
        if let Some(previous) = self.watched.take().and_then(|id| self.remotes.get(&id)) {
            previous.watch_metrics(false);
        }
        if let Some(remote) = focused.as_ref().and_then(|id| self.remotes.get(id)) {
            remote.watch_metrics(true);
        }
        self.watched = focused;
    }

    /// The host keeps the live sessions; this tells it where they sit and which has focus,
    /// so the Agent can name terminals by tab and pane.
    fn publish_layout(&mut self) {
        let tabs: Vec<serde_json::Value> = self
            .tabs
            .iter()
            .map(|tab| {
                let panes: Vec<serde_json::Value> = tab.root.panes().iter().map(|id| serde_json::json!({ "sessionId": id, "title": self.panes.get(id).map(|pane| pane.title.clone()).unwrap_or_default() })).collect();
                serde_json::json!({ "id": tab.id.to_string(), "title": tab.title, "panes": panes })
            })
            .collect();
        let mut tabs = tabs;
        if !self.docked.is_empty() {
            let panes: Vec<serde_json::Value> = self.docked.iter().map(|(id, _)| serde_json::json!({ "sessionId": id, "title": self.panes.get(id).map(|pane| pane.title.clone()).unwrap_or_default() })).collect();
            tabs.push(serde_json::json!({ "id": "docked", "title": "已停靠", "panes": panes }));
        }
        let layout = serde_json::json!({ "windowId": "main", "focusedSessionId": self.focused().map(|pane| pane.id().to_string()), "activeTabId": self.active.map(|id| id.to_string()), "tabs": tabs });
        if self.workspace_layout.as_ref() != Some(&layout) {
            let _ = self.backend.request("workspace.update", layout.clone());
            self.workspace_layout = Some(layout);
        }
    }

    fn scene(&self) -> Scene {
        scene(&self.settings, self.window_transparent)
    }

    pub fn theme(&self) -> &'static Theme {
        theme::by_id(&self.settings.theme)
    }

    fn global_keys(&mut self, ctx: &egui::Context) {
        let events = ctx.input(|input| input.events.clone());
        for event in events {
            let egui::Event::Key { key, pressed: true, modifiers, .. } = event else { continue };
            if let Some(action) = shortcuts::action(key, modifiers, self.mac) {
                ctx.input_mut(|input| input.consume_key(modifiers, key));
                self.run(ctx, action);
            } else if let Some(number) = shortcuts::tab_number(key, modifiers, self.mac) {
                ctx.input_mut(|input| input.consume_key(modifiers, key));
                let tab = if number == 9 { self.tabs.last() } else { self.tabs.get(number - 1) };
                if let Some(tab) = tab {
                    self.active = Some(tab.id);
                }
            } else if shortcuts::tab_cycle(key, modifiers) != 0 && !self.tabs.is_empty() {
                ctx.input_mut(|input| input.consume_key(modifiers, key));
                let step = shortcuts::tab_cycle(key, modifiers) as isize;
                let index = self.tabs.iter().position(|tab| Some(tab.id) == self.active).unwrap_or(0) as isize;
                self.active = Some(self.tabs[(index + step).rem_euclid(self.tabs.len() as isize) as usize].id);
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let scale = ctx.pixels_per_point();
        if (scale - self.scale).abs() > f32::EPSILON {
            self.scale = scale;
            self.fonts.configure(self.settings.font_size * scale, self.settings.line_height);
            self.fonts = Fonts::new(self.settings.font_size * scale, self.settings.line_height, self.settings.font_family.candidates());
        }
        self.handle_notices();
        let theme = self.theme();
        let scene = self.scene();
        let palette = Palette::new(theme, scene.clear.then_some(scene.veil));
        style::set_palette(&ctx, palette);
        if self.applied_theme != Some(theme.id) {
            self.applied_theme = Some(theme.id);
            ctx.set_theme(if theme.light { egui::Theme::Light } else { egui::Theme::Dark });
            style::apply_visuals(&ctx, &palette);
        }
        let modal = self.dialog.is_some() || self.palette.is_some();
        self.frame_uploads = Arc::default();
        let epoch = self.fonts.epoch;
        if !modal {
            self.global_keys(&ctx);
        }

        let mut intents = Vec::new();
        chrome::backdrop(self, ui, scene);
        if !self.focus_mode {
            chrome::title_bar(self, ui, &mut intents);
            chrome::status_bar(self, ui, &mut intents);
        }
        // A proposal waits on a person, so it must not sit in a hidden panel.
        let pending: Vec<String> = self.monitors.list().into_iter().filter(|monitor| monitor.status == crate::monitor::service::Status::Pending).map(|monitor| monitor.id).collect();
        if pending.iter().any(|id| !self.pending_seen.contains(id)) {
            self.monitor_visible = true;
            self.pending_seen.extend(pending);
        }
        dock::show(self, ui, &mut intents, self.focus_mode);
        dock::drag_overlay(self, &ctx);
        dock::pane_overlay(self, &ctx, &mut intents);
        egui::CentralPanel::default().frame(egui::Frame::new()).show(ui, |ui| terminals::show(self, ui, &mut intents, modal));
        dialogs::show(self, &ctx, &mut intents);
        palette::show(self, &ctx, &mut intents);
        let hosts: Vec<(String, String)> = self.panes.values().filter(|pane| self.remotes.get(pane.id()).is_some()).map(|pane| (pane.id().to_string(), pane.title.clone())).collect();
        let monitors = self.monitors.clone();
        let mut started = None;
        process_window::show(&mut self.processes, &ctx, &self.remotes, &hosts, |host, pid, command| {
            let request = serde_json::json!({ "host": host.map(|id| serde_json::json!({ "sessionId": id })).unwrap_or(serde_json::json!("local")), "label": format!("{command} ({pid})"), "target": { "kind": "process", "pid": pid }, "conditions": [], "intervalSeconds": 5, "durationMinutes": 60 });
            started = Some(monitors.propose(&request).map(|monitor| monitor.id));
        });
        match started {
            Some(Ok(id)) => {
                // The process window starts read-only monitors only, so this approval cannot grant anything.
                let monitors = self.monitors.clone();
                std::thread::spawn(move || {
                    let _ = monitors.approve(&id);
                });
                self.monitor_visible = true;
                self.notify("已开始只读监控这个进程。", false);
            }
            Some(Err(error)) => self.notify(error, true),
            None => {}
        }
        chrome::toast(self, &ctx);
        for intent in intents {
            self.apply(&ctx, intent);
        }
        // A drag released over nothing that takes it simply ends.
        if ctx.input(|input| input.pointer.any_released()) {
            self.dragging_pane = None;
        }
        // The OS takes the pointer over from here, and on Windows keeps it until the drop,
        // so egui's own drag of the row ends now rather than on a release it never sees.
        if let Some(file) = self.drag_out.take() {
            ctx.stop_dragging();
            if let Err(error) = crate::drag_out::start(frame, &file) {
                self.notify(error, true);
            }
        }

        // Every pane has been built by now and no callback has prepared yet, so glyphs first
        // seen anywhere this frame go up before anything is drawn. If the atlas started over
        // meanwhile, the panes built before that point are rebuilt next frame.
        self.frame_uploads.lock().unwrap_or_else(|p| p.into_inner()).append(&mut self.fonts.uploads);
        if self.fonts.epoch != epoch {
            ctx.request_repaint();
        }
        self.sync_settings();
        self.save_workspace(false);
        if ctx.input(|input| input.viewport().close_requested()) {
            self.save_workspace(true);
            self.save_ui_state();
            self.remotes.shutdown();
            self.agent.shutdown();
            self.monitors.shutdown();
            self.backend.shutdown();
        }
        self.watch_metrics();
        self.publish_layout();
        if let Some(smoke) = self.smoke.as_mut() {
            let first = self.tabs.first().and_then(|tab| self.panes.get(&tab.focus)).map(|pane| (pane.session.terminal.clone(), self.render_state.adapter.get_info().backend));
            smoke.tick(&ctx, &self.backend, first, self.fonts.family());
        }
        if self.workspace_dirty.is_some() {
            ctx.request_repaint_after(Duration::from_millis(450));
        }
    }

    fn on_exit(&mut self) {
        self.save_workspace(true);
        self.save_ui_state();
        self.remotes.shutdown();
        self.agent.shutdown();
        self.backend.shutdown();
    }
}

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok().map(|name| name.trim().to_string()))
        .unwrap_or_default()
}

/// Starts a new copy of the app and closes this one, for settings that need a new window.
fn relaunch(ctx: &egui::Context) {
    if let Ok(executable) = std::env::current_exe() {
        let _ = std::process::Command::new(executable).spawn();
    }
    ctx.send_viewport_cmd(ViewportCommand::Close);
}

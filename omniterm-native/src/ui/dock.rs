// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! The docking workspace: the sidebar, the Agent and the monitoring panel each sit on a window
//! edge, chosen by dragging the panel's grip onto that edge. Panels on the same edge share it
//! with a divider; sizes are pixels, so the terminals take every window resize.

use super::style;
use super::{App, Intent};
use crate::icons;
use eframe::egui::{self, Align2, Color32, CursorIcon, FontId, Id, Rect, Sense, Stroke, Ui, Vec2};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Panel {
    Sidebar,
    Agent,
    Monitor,
}

impl Panel {
    pub const ALL: [Panel; 3] = [Panel::Sidebar, Panel::Agent, Panel::Monitor];

    fn label(self) -> &'static str {
        match self {
            Panel::Sidebar => "工作区面板",
            Panel::Agent => "omniTerm Agent 面板",
            Panel::Monitor => "监控面板",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Dock {
    pub sidebar: Edge,
    pub agent: Edge,
    pub monitor: Edge,
    /// Pixel sizes across the edge, per edge.
    pub sizes: Vec<(Edge, f32)>,
    /// How panels sharing an edge divide it, the first one's share.
    pub splits: Vec<(Edge, f32)>,
}

impl Default for Dock {
    fn default() -> Self {
        Self { sidebar: Edge::Left, agent: Edge::Right, monitor: Edge::Right, sizes: vec![(Edge::Left, 248.0), (Edge::Right, 400.0), (Edge::Top, 260.0), (Edge::Bottom, 260.0)], splits: Vec::new() }
    }
}

impl Dock {
    pub fn edge(&self, panel: Panel) -> Edge {
        match panel {
            Panel::Sidebar => self.sidebar,
            Panel::Agent => self.agent,
            Panel::Monitor => self.monitor,
        }
    }

    fn set(&mut self, panel: Panel, edge: Edge) {
        match panel {
            Panel::Sidebar => self.sidebar = edge,
            Panel::Agent => self.agent = edge,
            Panel::Monitor => self.monitor = edge,
        }
    }

    pub fn size(&self, edge: Edge) -> f32 {
        self.sizes.iter().find(|(e, _)| *e == edge).map(|(_, size)| *size).unwrap_or(if matches!(edge, Edge::Left | Edge::Right) { 320.0 } else { 240.0 })
    }

    fn set_size(&mut self, edge: Edge, size: f32) {
        match self.sizes.iter_mut().find(|(e, _)| *e == edge) {
            Some(entry) => entry.1 = size,
            None => self.sizes.push((edge, size)),
        }
    }

    fn split(&self, edge: Edge) -> f32 {
        self.splits.iter().find(|(e, _)| *e == edge).map(|(_, share)| *share).unwrap_or(0.6)
    }

    fn set_split(&mut self, edge: Edge, share: f32) {
        let share = share.clamp(0.15, 0.85);
        match self.splits.iter_mut().find(|(e, _)| *e == edge) {
            Some(entry) => entry.1 = share,
            None => self.splits.push((edge, share)),
        }
    }
}

/// The grip at the top of a panel; dragging it shows where the panel can go.
pub fn grip(app: &mut App, ui: &mut Ui, panel: Panel) {
    let palette = style::palette(ui);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(16.0, 22.0), Sense::drag());
    let color = if response.hovered() || response.dragged() { palette.text_soft() } else { palette.faint() };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, icons::DOTS_SIX_VERTICAL, FontId::new(14.0, egui::FontFamily::Name(style::ICONS.into())), color);
    let response = response.on_hover_cursor(CursorIcon::Grab).on_hover_text(format!("拖动以移动{}：放到窗口的任一边缘", panel.label()));
    if response.drag_started() {
        app.dragging_panel = Some(panel);
    }
}

/// While a pane is dragged, the thin bands along the window edges dock it there. They are
/// narrow so the panes of the active tab stay the targets everywhere else.
pub fn edge_band(ctx: &egui::Context, pointer: Option<egui::Pos2>) -> Option<Edge> {
    let pointer = pointer?;
    let screen = ctx.content_rect();
    let band = Vec2::new((screen.width() * 0.06).max(40.0), (screen.height() * 0.07).max(36.0));
    if pointer.x < screen.left() + band.x {
        Some(Edge::Left)
    } else if pointer.x > screen.right() - band.x {
        Some(Edge::Right)
    } else if pointer.y < screen.top() + band.y {
        Some(Edge::Top)
    } else if pointer.y > screen.bottom() - band.y {
        Some(Edge::Bottom)
    } else {
        None
    }
}

fn band_rect(screen: Rect, edge: Edge) -> Rect {
    let band = Vec2::new((screen.width() * 0.06).max(40.0), (screen.height() * 0.07).max(36.0));
    match edge {
        Edge::Left => Rect::from_min_size(screen.min, Vec2::new(band.x, screen.height())),
        Edge::Right => Rect::from_min_max(egui::pos2(screen.right() - band.x, screen.top()), screen.max),
        Edge::Top => Rect::from_min_size(screen.min, Vec2::new(screen.width(), band.y)),
        Edge::Bottom => Rect::from_min_max(egui::pos2(screen.left(), screen.bottom() - band.y), screen.max),
    }
}

/// Shows the docking bands for a dragged pane and docks it on release over one.
pub fn pane_overlay(app: &mut App, ctx: &egui::Context, intents: &mut Vec<Intent>) {
    let Some(id) = app.dragging_pane.clone() else { return };
    let palette = style::palette_of(ctx);
    let screen = ctx.content_rect();
    let pointer = ctx.input(|input| input.pointer.hover_pos());
    let target = edge_band(ctx, pointer);
    egui::Area::new(Id::unique("pane-dock-overlay")).order(egui::Order::Foreground).fixed_pos(screen.min).interactable(false).show(ctx, |ui| {
        for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
            let hot = Some(edge) == target;
            ui.painter().rect(band_rect(screen, edge).shrink(4.0), 6.0, style::alpha(palette.tokens.accent, if hot { 0.24 } else { 0.05 }), Stroke::new(if hot { 2.0 } else { 1.0 }, style::alpha(palette.tokens.accent, if hot { 0.85 } else { 0.25 })), egui::StrokeKind::Inside);
        }
        if let (Some(pointer), Some(_)) = (pointer, target) {
            ui.painter().text(pointer + Vec2::new(14.0, 14.0), Align2::LEFT_TOP, "停靠到窗口边缘", FontId::proportional(12.0), palette.text());
        }
    });
    // Released anywhere else, the tab panes and the tab strip take the drop.
    if let Some(edge) = target.filter(|_| ctx.input(|input| input.pointer.any_released())) {
        intents.push(Intent::DockPane { id, edge });
        app.dragging_pane = None;
    }
}

fn drop_zones(screen: Rect) -> [(Edge, Rect); 4] {
    let band = Vec2::new(screen.width() * 0.22, screen.height() * 0.22);
    [
        (Edge::Left, Rect::from_min_size(screen.min, Vec2::new(band.x, screen.height()))),
        (Edge::Right, Rect::from_min_max(egui::pos2(screen.right() - band.x, screen.top()), screen.max)),
        (Edge::Top, Rect::from_min_size(screen.min + Vec2::new(band.x, 0.0), Vec2::new(screen.width() - band.x * 2.0, band.y))),
        (Edge::Bottom, Rect::from_min_max(egui::pos2(screen.left() + band.x, screen.bottom() - band.y), egui::pos2(screen.right() - band.x, screen.bottom()))),
    ]
}

/// While a grip is dragged, the window edges light up; releasing over one moves the panel.
pub fn drag_overlay(app: &mut App, ctx: &egui::Context) {
    let Some(panel) = app.dragging_panel else { return };
    let palette = style::palette_of(ctx);
    let screen = ctx.content_rect();
    let pointer = ctx.input(|input| input.pointer.hover_pos());
    let target = pointer.and_then(|pointer| drop_zones(screen).into_iter().find(|(_, zone)| zone.contains(pointer)).map(|(edge, _)| edge));
    egui::Area::new(Id::unique("dock-overlay")).order(egui::Order::Foreground).fixed_pos(screen.min).interactable(false).show(ctx, |ui| {
        for (edge, zone) in drop_zones(screen) {
            let hot = Some(edge) == target;
            ui.painter().rect(zone.shrink(6.0), 8.0, style::alpha(palette.tokens.accent, if hot { 0.22 } else { 0.06 }), Stroke::new(if hot { 2.0 } else { 1.0 }, style::alpha(palette.tokens.accent, if hot { 0.8 } else { 0.3 })), egui::StrokeKind::Inside);
        }
        if let Some(pointer) = pointer {
            ui.painter().text(pointer + Vec2::new(14.0, 14.0), Align2::LEFT_TOP, panel.label(), FontId::proportional(12.0), palette.text());
        }
    });
    ctx.set_cursor_icon(CursorIcon::Grabbing);
    if ctx.input(|input| input.pointer.any_released()) {
        if let Some(edge) = target {
            app.dock.set(panel, edge);
        }
        app.dragging_panel = None;
    }
}

#[derive(Clone, PartialEq)]
enum Member {
    Panel(Panel),
    Terminal(String),
}

/// Lays out the docked panels and terminals around the central area, in a fixed order per
/// edge. Full-screen terminals keep docked terminals and hide the panels.
pub fn show(app: &mut App, ui: &mut Ui, intents: &mut Vec<Intent>, terminals_only: bool) {
    for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
        let panels = Panel::ALL.into_iter().filter(|panel| !terminals_only && app.dock.edge(*panel) == edge && app.panel_visible(*panel)).map(Member::Panel);
        let terminals = app.docked.iter().filter(|(_, docked)| *docked == edge).map(|(id, _)| Member::Terminal(id.clone()));
        let members: Vec<Member> = panels.chain(terminals).collect();
        if members.is_empty() {
            continue;
        }
        let palette = style::palette(ui);
        let id = format!("dock-{edge:?}");
        let size = app.dock.size(edge);
        let builder = match edge {
            Edge::Left => egui::Panel::left(id),
            Edge::Right => egui::Panel::right(id),
            Edge::Top => egui::Panel::top(id),
            Edge::Bottom => egui::Panel::bottom(id),
        };
        let range = if matches!(edge, Edge::Left | Edge::Right) { 180.0..=900.0 } else { 120.0..=700.0 };
        let response = builder.resizable(true).default_size(size).size_range(range).frame(egui::Frame::new().fill(Color32::TRANSPARENT)).show(ui, |ui| {
            let rect = ui.max_rect();
            if members.len() == 1 {
                member(app, ui, &members[0], rect, intents);
                return;
            }
            // Two or three panels share the edge, split along it.
            let vertical = matches!(edge, Edge::Left | Edge::Right);
            let share = app.dock.split(edge);
            let length = if vertical { rect.height() } else { rect.width() };
            let first = length * share;
            let rest = (length - first) / (members.len() - 1) as f32;
            let mut offset = 0.0;
            for (index, panel) in members.iter().enumerate() {
                let span = if index == 0 { first } else { rest };
                let area = if vertical { Rect::from_min_size(rect.min + Vec2::new(0.0, offset), Vec2::new(rect.width(), span)) } else { Rect::from_min_size(rect.min + Vec2::new(offset, 0.0), Vec2::new(span, rect.height())) };
                member(app, ui, panel, area.shrink2(if vertical { Vec2::new(0.0, 1.5) } else { Vec2::new(1.5, 0.0) }), intents);
                offset += span;
            }
            // The divider after the first panel moves the split.
            let at = if vertical { rect.top() + first } else { rect.left() + first };
            let handle = if vertical { Rect::from_center_size(egui::pos2(rect.center().x, at), Vec2::new(rect.width(), 6.0)) } else { Rect::from_center_size(egui::pos2(at, rect.center().y), Vec2::new(6.0, rect.height())) };
            let response = ui.interact(handle, Id::unique(("dock-split", edge as u8)), Sense::drag()).on_hover_cursor(if vertical { CursorIcon::ResizeVertical } else { CursorIcon::ResizeHorizontal });
            if response.dragged() && let Some(pointer) = response.interact_pointer_pos() {
                let share = if vertical { (pointer.y - rect.top()) / rect.height() } else { (pointer.x - rect.left()) / rect.width() };
                app.dock.set_split(edge, share);
            }
            if response.hovered() || response.dragged() {
                ui.painter().rect_filled(handle.shrink(1.5), 1.0, style::alpha(palette.tokens.accent, 0.5));
            }
        });
        let rect = response.response.rect;
        app.dock.set_size(edge, if matches!(edge, Edge::Left | Edge::Right) { rect.width() } else { rect.height() });
    }
}

fn member(app: &mut App, ui: &mut Ui, member: &Member, rect: Rect, intents: &mut Vec<Intent>) {
    let palette = style::palette(ui);
    let panel = match member {
        Member::Panel(panel) => *panel,
        Member::Terminal(id) => {
            super::terminals::docked(app, ui, id, rect, intents);
            return;
        }
    };
    ui.painter().rect_filled(rect, 0.0, if panel == Panel::Sidebar { palette.panel } else { palette.panel_soft });
    let inner = rect.shrink2(if panel == Panel::Sidebar { Vec2::new(12.0, 0.0) } else { Vec2::new(10.0, 0.0) });
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner).id_salt(("dock-member", panel as u8)), |ui| {
        ui.set_clip_rect(rect);
        match panel {
            Panel::Sidebar => {
                ui.horizontal(|ui| {
                    ui.add_space(rect.width() - 46.0);
                    grip(app, ui, panel);
                });
                super::sidebar::show(app, ui, intents);
            }
            Panel::Agent => super::agent_panel::show(app, ui, intents),
            Panel::Monitor => super::monitor_view::show(app, ui, intents),
        }
    });
    let edge_line = rect.x_range();
    ui.painter().hline(edge_line, rect.bottom() - 0.5, Stroke::new(1.0, palette.border));
}

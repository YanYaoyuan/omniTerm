// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Turns a terminal's screen into GPU quads. Pure: no window, no GPU, so it is benchmarked and
//! tested on its own.

use crate::glyphs::Fonts;
use crate::gpu::{COLOR, Instance, MASK, SOLID};
use crate::terminal::Listener;
use crate::theme::Theme;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};

pub const PADDING: f32 = 8.0;

fn rgba(color: Rgb, alpha: f32) -> [f32; 4] {
    [color.r as f32 / 255.0, color.g as f32 / 255.0, color.b as f32 / 255.0, alpha]
}

use crate::theme::mix;

fn solid(x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) -> Instance {
    Instance { rect: [x, y, w, h], color, kind: SOLID, ..Default::default() }
}

pub struct Frame<'a> {
    pub theme: &'a Theme,
    pub size: [f32; 2],
    pub focused: bool,
    /// Opacity of the terminal background, so the window's backdrop can show through.
    pub background_alpha: f32,
    /// An unfocused pane of a split is faded so the focused one stands out.
    pub dim: bool,
    /// The blinking cursor's current phase.
    pub cursor_on: bool,
}

/// Builds every quad for one pane: background runs, glyphs, decorations, then the cursor.
pub fn build(term: &Term<Listener>, fonts: &mut Fonts, frame: &Frame) -> Vec<Instance> {
    let theme = frame.theme;
    let cell = fonts.cell;
    let content = term.renderable_content();
    let colors = content.colors;
    let overrides = |index: usize| colors[index];
    let offset = content.display_offset as i32;
    let rows = term.screen_lines() as i32;
    let mut out = Vec::with_capacity(term.columns() * term.screen_lines() / 2 + 16);
    let mut glyphs = Vec::with_capacity(term.columns() * term.screen_lines());
    if frame.background_alpha > 0.0 {
        out.push(solid(0.0, 0.0, frame.size[0], frame.size[1], rgba(theme.background, frame.background_alpha)));
    }

    let cursor_visible = frame.cursor_on && content.mode.contains(TermMode::SHOW_CURSOR) && content.cursor.shape != CursorShape::Hidden;
    let cursor_row = content.cursor.point.line.0 + offset;
    let cursor_column = content.cursor.point.column.0;
    let block_cursor = cursor_visible && frame.focused && matches!(content.cursor.shape, CursorShape::Block);
    let selection = content.selection;

    // Background runs on one row are merged into one quad.
    let mut run: Option<(i32, usize, usize, Rgb)> = None;
    let flush = |run: &mut Option<(i32, usize, usize, Rgb)>, out: &mut Vec<Instance>| {
        if let Some((row, start, end, color)) = run.take() {
            out.push(solid(
                PADDING + start as f32 * cell.width,
                PADDING + row as f32 * cell.height,
                (end - start) as f32 * cell.width,
                cell.height,
                rgba(color, 1.0),
            ));
        }
    };

    for indexed in content.display_iter {
        let row = indexed.point.line.0 + offset;
        if row < 0 || row >= rows {
            continue;
        }
        let column = indexed.point.column.0;
        let cell_ref = indexed.cell;
        let flags = cell_ref.flags;
        let mut fg = theme.resolve(cell_ref.fg, &overrides);
        let mut bg = theme.resolve(cell_ref.bg, &overrides);
        let default_bg = matches!(cell_ref.bg, Color::Named(NamedColor::Background));
        let mut has_bg = !default_bg;
        if flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
            has_bg = true;
        }
        if flags.contains(Flags::DIM) {
            fg = mix(fg, bg, 0.34);
        }
        if frame.dim {
            fg = mix(fg, theme.background, 0.38);
        }
        if selection.is_some_and(|range| range.contains(indexed.point)) {
            bg = theme.selection;
            has_bg = true;
        }
        let at_cursor = row == cursor_row && column == cursor_column;
        if block_cursor && at_cursor {
            fg = theme.background;
        }
        let width = if flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };
        match &mut run {
            Some((r, _, end, color)) if has_bg && *r == row && *end == column && *color == bg => *end += width,
            _ => {
                flush(&mut run, &mut out);
                if has_bg {
                    run = Some((row, column, column + width, bg));
                }
            }
        }
        if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            continue;
        }
        let x = PADDING + column as f32 * cell.width;
        let y = PADDING + row as f32 * cell.height;
        if flags.intersects(Flags::ALL_UNDERLINES) {
            let thickness = (cell.height / 14.0).round().max(1.0);
            glyphs.push(solid(x, y + cell.height - thickness * 2.0, cell.width * width as f32, thickness, rgba(fg, 1.0)));
        }
        if flags.contains(Flags::STRIKEOUT) {
            let thickness = (cell.height / 14.0).round().max(1.0);
            glyphs.push(solid(x, y + (cell.height / 2.0).round(), cell.width * width as f32, thickness, rgba(fg, 1.0)));
        }
        let character = cell_ref.c;
        if character == ' ' || character == '\t' || flags.contains(Flags::HIDDEN) {
            continue;
        }
        let mut text = String::from(character);
        if let Some(extra) = cell_ref.zerowidth() {
            text.extend(extra);
        }
        let color = rgba(fg, 1.0);
        for piece in fonts.glyph(&text, flags.contains(Flags::BOLD), flags.contains(Flags::ITALIC)) {
            glyphs.push(Instance {
                rect: [(x + piece.offset[0]).round(), (y + piece.offset[1]).round(), piece.atlas[2], piece.atlas[3]],
                atlas: piece.atlas,
                color,
                kind: if piece.colored { COLOR } else { MASK },
                _pad: [0; 3],
            });
        }
    }
    flush(&mut run, &mut out);

    // The block cursor sits under its glyph; the others are drawn over it.
    if cursor_visible && (0..rows).contains(&cursor_row) {
        let x = PADDING + cursor_column as f32 * cell.width;
        let y = PADDING + cursor_row as f32 * cell.height;
        let color = rgba(theme.cursor, 1.0);
        let thin = (cell.width / 8.0).round().max(1.0);
        if !frame.focused {
            for edge in [solid(x, y, cell.width, thin, color), solid(x, y + cell.height - thin, cell.width, thin, color), solid(x, y, thin, cell.height, color), solid(x + cell.width - thin, y, thin, cell.height, color)] {
                glyphs.push(edge);
            }
        } else {
            match content.cursor.shape {
                CursorShape::Block => out.push(solid(x, y, cell.width, cell.height, color)),
                CursorShape::Underline => glyphs.push(solid(x, y + cell.height - thin * 2.0, cell.width, thin * 2.0, color)),
                CursorShape::Beam => glyphs.push(solid(x, y, thin.max(2.0), cell.height, color)),
                CursorShape::HollowBlock => {
                    for edge in [solid(x, y, cell.width, thin, color), solid(x, y + cell.height - thin, cell.width, thin, color), solid(x, y, thin, cell.height, color), solid(x + cell.width - thin, y, thin, cell.height, color)] {
                        glyphs.push(edge);
                    }
                }
                CursorShape::Hidden => {}
            }
        }
    }
    out.extend(glyphs);
    out
}

/// How many columns and rows fit in a pane of `size` physical pixels.
pub fn grid_size(size: [f32; 2], cell: crate::glyphs::CellSize) -> (usize, usize) {
    let columns = ((size[0] - 2.0 * PADDING) / cell.width).floor().max(2.0) as usize;
    let rows = ((size[1] - 2.0 * PADDING) / cell.height).floor().max(1.0) as usize;
    (columns, rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::{Size, Terminal};
    use crate::theme::THEMES;
    use std::sync::mpsc::channel;

    #[test]
    fn text_backgrounds_and_the_cursor_become_quads() {
        let (sender, _receiver) = channel();
        let options = crate::terminal::Options { scrollback: 100, cursor: CursorShape::Block, blinking: false };
        let terminal = Terminal::new("t".into(), Size { cols: 10, rows: 2 }, options, sender);
        terminal.feed(b"\x1b[41mab\x1b[0m c");
        let mut fonts = Fonts::new(14.0, 1.2, &[]);
        let frame = Frame { theme: &THEMES[0], size: [200.0, 60.0], focused: true, background_alpha: 1.0, dim: false, cursor_on: true };
        let quads = build(&terminal.lock(), &mut fonts, &frame);
        let solids: Vec<_> = quads.iter().filter(|quad| quad.kind == SOLID).collect();
        // Pane background, the red run covering both cells, and the block cursor.
        assert_eq!(solids.len(), 3);
        assert_eq!(solids[1].rect[2], fonts.cell.width * 2.0);
        assert_eq!(solids[1].color, rgba(THEMES[0].ansi[1], 1.0));
        // Three characters drew at least one glyph each, unless no font is installed at all.
        let glyphs = quads.iter().filter(|quad| quad.kind != SOLID).count();
        assert!(glyphs == 0 || glyphs >= 3);
    }

    #[test]
    fn the_grid_fits_inside_the_padding() {
        let cell = crate::glyphs::CellSize { width: 10.0, height: 20.0 };
        assert_eq!(grid_size([816.0, 416.0], cell), (80, 20));
        assert_eq!(grid_size([1.0, 1.0], cell), (2, 1));
    }
}

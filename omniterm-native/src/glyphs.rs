// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Fonts, the terminal cell size, and glyphs rasterized once into a GPU atlas.
//!
//! Each distinct character and style is shaped and rasterized the first time it appears and
//! reused from the atlas after that, so steady-state drawing does no text shaping at all.
//! Fallback fonts (CJK, emoji, symbols) come from cosmic-text's shaping.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style, SwashCache, SwashContent, Weight};
use std::collections::HashMap;

pub const ATLAS_SIZE: u32 = 2048;

/// The first installed monospace family on each platform; fontconfig's choice otherwise.
const PREFERRED: &[&str] = if cfg!(windows) {
    &["Cascadia Mono", "Consolas", "Sarasa Mono SC", "Courier New"]
} else if cfg!(target_os = "macos") {
    &["SF Mono", "Menlo", "Monaco"]
} else {
    &["JetBrains Mono", "DejaVu Sans Mono", "Noto Sans Mono", "Liberation Mono"]
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CellSize {
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    /// Rectangle in the atlas, in pixels.
    pub atlas: [f32; 4],
    /// Top-left corner relative to the cell's top-left corner, in pixels.
    pub offset: [f32; 2],
    /// Already colored (emoji), as opposed to a coverage mask tinted with the text color.
    pub colored: bool,
}

#[derive(Hash, PartialEq, Eq, Clone)]
struct GlyphKey {
    text: String,
    bold: bool,
    italic: bool,
}

pub struct Upload {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

struct Shelf {
    x: u32,
    y: u32,
    row: u32,
}

pub struct Fonts {
    system: FontSystem,
    rasterizer: SwashCache,
    family: Option<String>,
    size: f32,
    line_height: f32,
    pub cell: CellSize,
    glyphs: HashMap<GlyphKey, Vec<Piece>>,
    shelf: Shelf,
    /// Pixels waiting to be copied into the atlas texture.
    pub uploads: Vec<Upload>,
    /// Moves on each time the atlas starts over, so quads built against an older atlas are rebuilt.
    pub epoch: u64,
}

impl Fonts {
    /// `wanted` lists the families the user chose, tried before the platform's defaults.
    pub fn new(size: f32, line_height: f32, wanted: &[&str]) -> Self {
        let mut system = FontSystem::new();
        let family = wanted
            .iter()
            .chain(PREFERRED)
            .find(|name| system.db().faces().any(|face| face.families.iter().any(|(family, _)| family == *name)))
            .map(|name| name.to_string());
        if let Some(name) = &family {
            system.db_mut().set_monospace_family(name.clone());
        }
        let mut fonts = Self {
            system,
            rasterizer: SwashCache::new(),
            family,
            size,
            line_height,
            cell: CellSize::default(),
            glyphs: HashMap::new(),
            shelf: Shelf { x: 1, y: 1, row: 0 },
            uploads: Vec::new(),
            epoch: 0,
        };
        fonts.set_size(size);
        fonts
    }

    /// Changes size and line height together, re-rasterizing only if either changed.
    pub fn configure(&mut self, size: f32, line_height: f32) {
        if (size - self.size).abs() > f32::EPSILON || (line_height - self.line_height).abs() > f32::EPSILON {
            self.line_height = line_height;
            self.set_size(size);
        }
    }

    pub fn family(&self) -> &str {
        self.family.as_deref().unwrap_or("monospace")
    }

    /// `size` is in physical pixels, so it already includes the display's scale factor.
    pub fn set_size(&mut self, size: f32) {
        self.size = size.clamp(6.0, 96.0);
        let height = (self.size * self.line_height).round();
        let mut buffer = Buffer::new(&mut self.system, Metrics::new(self.size, height));
        buffer.set_text("M", &Attrs::new().family(Family::Monospace), Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.system, false);
        let width = buffer.layout_runs().flat_map(|run| run.glyphs.iter()).map(|glyph| glyph.w).next().unwrap_or(self.size * 0.6);
        self.cell = CellSize { width: width.round().max(1.0), height: height.max(1.0) };
        self.clear();
    }

    fn clear(&mut self) {
        self.glyphs.clear();
        self.shelf = Shelf { x: 1, y: 1, row: 0 };
        self.uploads.clear();
        self.epoch += 1;
    }

    /// The atlas pieces that draw `text` in one cell, rasterizing them on first use.
    pub fn glyph(&mut self, text: &str, bold: bool, italic: bool) -> &[Piece] {
        let key = GlyphKey { text: text.to_string(), bold, italic };
        if !self.glyphs.contains_key(&key) {
            let pieces = match self.rasterize(&key) {
                Some(pieces) => pieces,
                None => {
                    // The atlas is full: start over, then this glyph is the first one in it.
                    self.clear();
                    self.rasterize(&key).unwrap_or_default()
                }
            };
            self.glyphs.insert(key.clone(), pieces);
        }
        &self.glyphs[&key]
    }

    /// `None` when the atlas has no room left.
    fn rasterize(&mut self, key: &GlyphKey) -> Option<Vec<Piece>> {
        let attrs = Attrs::new()
            .family(Family::Monospace)
            .weight(if key.bold { Weight::BOLD } else { Weight::NORMAL })
            .style(if key.italic { Style::Italic } else { Style::Normal });
        let mut buffer = Buffer::new(&mut self.system, Metrics::new(self.size, self.cell.height));
        buffer.set_text(&key.text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.system, false);
        let mut placed = Vec::new();
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                placed.push(glyph.physical((0.0, run.line_y), 1.0));
            }
        }
        let mut pieces = Vec::new();
        for physical in placed {
            let Some(image) = self.rasterizer.get_image_uncached(&mut self.system, physical.cache_key) else { continue };
            let (width, height) = (image.placement.width, image.placement.height);
            if width == 0 || height == 0 {
                continue;
            }
            let (x, y) = self.allocate(width, height)?;
            let colored = matches!(image.content, SwashContent::Color);
            let rgba = to_rgba(&image.content, &image.data, (width * height) as usize);
            self.uploads.push(Upload { x, y, width, height, rgba });
            pieces.push(Piece {
                atlas: [x as f32, y as f32, width as f32, height as f32],
                offset: [(physical.x + image.placement.left) as f32, (physical.y - image.placement.top) as f32],
                colored,
            });
        }
        Some(pieces)
    }

    fn allocate(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
        if width + 2 > ATLAS_SIZE || height + 2 > ATLAS_SIZE {
            return None;
        }
        if self.shelf.x + width + 1 > ATLAS_SIZE {
            self.shelf = Shelf { x: 1, y: self.shelf.y + self.shelf.row + 1, row: 0 };
        }
        if self.shelf.y + height + 1 > ATLAS_SIZE {
            return None;
        }
        let spot = (self.shelf.x, self.shelf.y);
        self.shelf.x += width + 1;
        self.shelf.row = self.shelf.row.max(height);
        Some(spot)
    }
}

/// Masks become white with coverage in alpha; color glyphs are premultiplied.
fn to_rgba(content: &SwashContent, data: &[u8], pixels: usize) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(pixels * 4);
    match content {
        SwashContent::Mask => {
            for &alpha in data.iter().take(pixels) {
                rgba.extend_from_slice(&[255, 255, 255, alpha]);
            }
        }
        SwashContent::SubpixelMask => {
            for pixel in data.chunks_exact(4).take(pixels) {
                let alpha = ((pixel[0] as u16 + pixel[1] as u16 + pixel[2] as u16) / 3) as u8;
                rgba.extend_from_slice(&[255, 255, 255, alpha]);
            }
        }
        SwashContent::Color => {
            for pixel in data.chunks_exact(4).take(pixels) {
                let alpha = pixel[3] as u16;
                let premultiply = |value: u8| (value as u16 * alpha / 255) as u8;
                rgba.extend_from_slice(&[premultiply(pixel[0]), premultiply(pixel[1]), premultiply(pixel[2]), pixel[3]]);
            }
        }
    }
    rgba.resize(pixels * 4, 0);
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_carry_coverage_in_alpha_and_colors_are_premultiplied() {
        assert_eq!(to_rgba(&SwashContent::Mask, &[0, 128], 2), [255, 255, 255, 0, 255, 255, 255, 128]);
        assert_eq!(to_rgba(&SwashContent::Color, &[200, 100, 50, 128], 1), [100, 50, 25, 128]);
    }

    #[test]
    fn the_atlas_packs_rows_and_reports_when_it_is_full() {
        let mut fonts = Fonts { shelf: Shelf { x: 1, y: 1, row: 0 }, ..Fonts::new(14.0, 1.2, &[]) };
        let first = fonts.allocate(10, 20).unwrap();
        let second = fonts.allocate(10, 5).unwrap();
        assert_eq!((first, second), ((1, 1), (12, 1)));
        assert!(fonts.allocate(ATLAS_SIZE, 1).is_none());
        fonts.shelf = Shelf { x: 1, y: ATLAS_SIZE - 3, row: 0 };
        assert!(fonts.allocate(4, 8).is_none());
    }
}

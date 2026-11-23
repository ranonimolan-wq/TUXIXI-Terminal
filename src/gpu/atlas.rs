// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! Glyph atlas: a shelf-packing R8 coverage texture. Glyphs are
//! rasterized on the CPU once, cached as atlas entries keyed by
//! (glyph id, style) and uploaded to the GPU; the packer is pure and
//! unit-tested headless.

use std::collections::HashMap;

use crate::font::GlyphBitmap;

// Glyph atlas (pure shelf packer + CPU pixel buffer)
// ---------------------------------------------------------------------------

/// Atlas texture side in pixels. One R8 2048x2048 texture = 4 MiB of
/// GPU (and bounded CPU) memory, enough for tens of thousands of
/// glyphs at terminal sizes.
pub const ATLAS_SIZE: u32 = 2048;

/// Where one glyph lives inside the atlas: pixel rect + normalized uv.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AtlasEntry {
    /// Pixel rect inside the atlas (x, y, w, h).
    pub px: [u32; 4],
    /// Normalized uv rect (u0, v0, u1, v1).
    pub uv: [f32; 4],
}

/// Pure glyph-atlas data: shelf packer over an R8 pixel buffer.
///
/// The GL layer uploads `pixels` whenever `dirty` is set (after the
/// first upload it only re-uploads; a full re-upload is one 4 MiB
/// memcpy per overflow, which is rare). When the atlas runs out of
/// space it is reset wholesale and refills on the next frame from the
/// bounded font cache — memory stays bounded by construction.
pub struct Atlas {
    pub size: u32,
    /// Row-major R8 coverage buffer, size*size bytes.
    pub pixels: Vec<u8>,
    pub(crate) entries: HashMap<(char, u8), AtlasEntry>,
    cursor_x: u32,
    cursor_y: u32,
    row_h: u32,
    dirty: bool,
}

impl Default for Atlas {
    fn default() -> Self {
        Self::new(ATLAS_SIZE)
    }
}

impl Atlas {
    pub fn new(size: u32) -> Self {
        Self {
            size,
            pixels: vec![0; (size * size) as usize],
            entries: HashMap::new(),
            cursor_x: 0,
            cursor_y: 0,
            row_h: 0,
            dirty: false,
        }
    }

    pub fn entry(&self, key: (char, u8)) -> Option<&AtlasEntry> {
        self.entries.get(&key)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn needs_upload(&self) -> bool {
        self.dirty
    }

    /// Consume the dirty flag (the GL layer calls this after the
    /// upload is done).
    pub fn mark_uploaded(&mut self) {
        self.dirty = false;
    }

    /// Pack `bitmap` for `key`. Returns `false` when the atlas is full
    /// (the caller should [`Atlas::reset`] and retry once).
    pub fn insert(&mut self, key: (char, u8), bitmap: &GlyphBitmap) -> bool {
        let (w, h) = (bitmap.width as u32, bitmap.height as u32);
        if w == 0 || h == 0 || w > self.size || h > self.size {
            return false;
        }
        if self.entries.contains_key(&key) {
            return true;
        }
        // Advance to the next shelf row when this one is full.
        if self.cursor_x + w > self.size {
            self.cursor_y += self.row_h;
            self.cursor_x = 0;
            self.row_h = 0;
        }
        if self.cursor_y + h > self.size {
            return false; // full: caller resets and refills
        }

        let (x, y) = (self.cursor_x, self.cursor_y);
        // Blit the coverage rows into the pixel buffer.
        for row in 0..bitmap.height {
            let dst = ((y as usize + row) * self.size as usize) + x as usize;
            let src = row * bitmap.width;
            self.pixels[dst..dst + bitmap.width]
                .copy_from_slice(&bitmap.coverage[src..src + bitmap.width]);
        }
        self.cursor_x += w;
        self.row_h = self.row_h.max(h);
        let s = self.size as f32;
        let entry = AtlasEntry {
            px: [x, y, w, h],
            uv: [
                x as f32 / s,
                y as f32 / s,
                (x + w) as f32 / s,
                (y + h) as f32 / s,
            ],
        };
        self.entries.insert(key, entry);
        self.dirty = true;
        true
    }

    /// Drop everything (atlas overflow): the pixel buffer is zeroed
    /// with `fill` — explicit manual memory hygiene, no stale coverage
    /// is ever left behind.
    pub fn reset(&mut self) {
        self.pixels.fill(0);
        self.entries.clear();
        self.cursor_x = 0;
        self.cursor_y = 0;
        self.row_h = 0;
        self.dirty = true;
    }

    /// GPU + CPU bytes held by this atlas (memory debug output).
    pub fn memory_bytes(&self) -> usize {
        self.pixels.len()
    }
}

// ---------------------------------------------------------------------------
// Scene builder (pure; same cell semantics as render::paint)
// ---------------------------------------------------------------------------

// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! Pure scene representation: the rectangle / glyph draw lists, the
//! glass (translucency) parameters and the byte-to-float color math.
//! No GL state here - everything is plain data so the whole pipeline
//! is testable headless.

use crate::render::blend;

// ---------------------------------------------------------------------------
// Pure scene representation
// ---------------------------------------------------------------------------

/// One solid rectangle in window pixel coordinates (y downward, like
/// the CPU renderer).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RectDraw {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Straight (non-premultiplied) RGBA, components 0.0..=1.0.
    pub color: [f32; 4],
}

/// One textured glyph quad: position/size in window pixels, `uv`
/// inside the atlas texture, RGBA color (straight alpha).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphDraw {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub uv: [f32; 4],
    pub color: [f32; 4],
}

/// Everything the GPU draws in one frame.
#[derive(Debug, Default, PartialEq)]
pub struct Scene {
    pub rects: Vec<RectDraw>,
    pub glyphs: Vec<GlyphDraw>,
}

impl Scene {
    /// Number of vertices the GL layer must upload for this scene
    /// (2 triangles per quad, 3 vertices per triangle).
    pub fn vertex_count(&self) -> usize {
        (self.rects.len() + self.glyphs.len()) * 6
    }
}

/// Glass rendering parameters (mirrors the settings fields so the
/// renderer never touches the settings manager).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlassParams {
    pub enabled: bool,
    /// Background opacity when enabled, 0.10..=1.0.
    pub opacity: f32,
}

impl GlassParams {
    pub fn new(enabled: bool, opacity: f32) -> Self {
        Self {
            enabled,
            opacity: opacity.clamp(
                crate::settings::MIN_GLASS_OPACITY,
                crate::settings::MAX_GLASS_OPACITY,
            ),
        }
    }

    /// Alpha the background is cleared with: the theme background at
    /// glass opacity when glass is on, fully opaque otherwise.
    pub fn bg_alpha(&self) -> f32 {
        if self.enabled {
            self.opacity
        } else {
            1.0
        }
    }

    /// Text ink is always fully opaque so it stays crisp on glass.
    pub fn fg_alpha(&self) -> f32 {
        1.0
    }
}

impl Default for GlassParams {
    fn default() -> Self {
        Self::new(true, crate::settings::DEFAULT_GLASS_OPACITY)
    }
}

/// sRGB8 bytes -> normalized GL color.
pub fn rgb01(c: [u8; 3]) -> [f32; 3] {
    [
        c[0] as f32 / 255.0,
        c[1] as f32 / 255.0,
        c[2] as f32 / 255.0,
    ]
}

/// sRGB8 bytes + alpha -> normalized GL RGBA.
pub fn rgba01(c: [u8; 3], a: f32) -> [f32; 4] {
    let [r, g, b] = rgb01(c);
    [r, g, b, a.clamp(0.0, 1.0)]
}

/// Interpolate two RGB colors and attach alpha (used for the selection
/// highlight, mirroring `render::paint`).
pub fn blend01(a: [u8; 3], b: [u8; 3], t: f32, alpha: f32) -> [f32; 4] {
    rgba01(blend(a, b, t), alpha)
}

// ---------------------------------------------------------------------------

pub(crate) fn bytemuck_slice(v: &[f32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 4) }
}

/// Push one rect quad (6 vertices) into an interleaved pos+color
/// buffer: [x, y, r, g, b, a] per vertex.
pub fn push_rect(out: &mut Vec<f32>, r: &RectDraw) {
    let (x0, y0) = (r.x, r.y);
    let (x1, y1) = (r.x + r.w, r.y + r.h);
    let c = r.color;
    // Two triangles: (x0,y0) (x1,y0) (x0,y1) | (x0,y1) (x1,y0) (x1,y1)
    for (vx, vy) in [(x0, y0), (x1, y0), (x0, y1), (x0, y1), (x1, y0), (x1, y1)] {
        out.extend_from_slice(&[vx, vy, c[0], c[1], c[2], c[3]]);
    }
}

/// Push one glyph quad (6 vertices) into an interleaved pos+uv+color
/// buffer: [x, y, u, v, r, g, b, a] per vertex.
pub fn push_glyph(out: &mut Vec<f32>, g: &GlyphDraw) {
    let (x0, y0) = (g.x, g.y);
    let (x1, y1) = (g.x + g.w, g.y + g.h);
    let [u0, v0, u1, v1] = g.uv;
    let c = g.color;
    for (vx, vy, vu, vv) in [
        (x0, y0, u0, v0),
        (x1, y0, u1, v0),
        (x0, y1, u0, v1),
        (x0, y1, u0, v1),
        (x1, y0, u1, v0),
        (x1, y1, u1, v1),
    ] {
        out.extend_from_slice(&[vx, vy, vu, vv, c[0], c[1], c[2], c[3]]);
    }
}

// ---------------------------------------------------------------------------
// Tests — pure layers only, fully headless
// ---------------------------------------------------------------------------

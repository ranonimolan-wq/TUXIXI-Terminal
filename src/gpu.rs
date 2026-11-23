// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! GPU renderer — OpenGL 3.3 core via `glow`.
//!
//! The old CPU renderer (`render::paint`) walked the grid and wrote
//! every pixel into a `softbuffer` framebuffer: the whole frame was a
//! few MB of plain arithmetic on the CPU on every repaint. The GPU
//! renderer keeps the same visual semantics but turns the grid into
//! *geometry* instead of pixels:
//!
//! * solid cell rectangles (background / cursor / selection /
//!   underline) are drawn with a plain color shader,
//! * glyphs are rasterized on the CPU once (ab_glyph, unchanged) and
//!   uploaded into a **glyph atlas texture** (R8 coverage); drawing is
//!   a textured quad per cell with the color in a vertex attribute,
//! * glass mode clears with a translucent background so a compositor
//!   (KWin blur-behind / ARGB compositor) can blur what is behind the
//!   window; text ink stays fully opaque and readable.
//!
//! Everything that can be tested headless is pure and heavily unit
//! tested: the shelf atlas packer, the per-frame scene builder

mod atlas;
mod draw;
mod renderer;
mod scene;
mod shader;

pub use atlas::{Atlas, AtlasEntry, ATLAS_SIZE};
pub use draw::{
    blend01, push_glyph, push_rect, rgb01, rgba01, GlassParams, GlyphDraw, RectDraw, Scene,
};
pub use renderer::GpuRenderer;
pub use scene::{build_scene, clear_color, SceneInputs};
pub use shader::{
    GLYPH_FRAGMENT_SHADER_BODY, GLYPH_VERTEX_SHADER_BODY, RECT_FRAGMENT_SHADER_BODY,
    RECT_VERTEX_SHADER_BODY,
};

#[cfg(test)]
mod tests;

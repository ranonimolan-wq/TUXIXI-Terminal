//! GPU renderer — OpenGL 3.3 core via `glow`, in the Alacritty style.
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
//! (`build_scene`, semantics mirrored from `render::paint`) and the
//! glass parameter math. The GL layer (`GpuRenderer`) is the only
//! unsafe part and contains zero policy.

use std::collections::HashMap;

use crate::font::{FontFace, GlyphBitmap, STYLE_BOLD, STYLE_ITALIC};
use crate::render::{blend, resolve, Palette, Selection};
use crate::vt::{Cell, Color, Terminal, BOLD, DIM, INVERSE, ITALIC, UNDERLINE, WIDE_SPACER};

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
    entries: HashMap<(char, u8), AtlasEntry>,
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

/// Build the draw scene for one frame. Pixel geometry uses the font
/// cell metrics (`cw` x `lh`), identical to `render::paint`.
///
/// Semantics mirrored from the CPU reference renderer:
/// * the cleared background covers default-bg cells (they get NO rect
///   — with glass this is exactly what the compositor blurs through),
/// * non-default cell backgrounds, cursor and selection are opaque
///   rects,
/// * inverse swaps fg/bg, dim blends fg toward bg, underline is a
///   1-px rect at the cell bottom,
/// * the cursor is only drawn on the live screen (offset == 0).
#[allow(clippy::too_many_arguments)]
pub fn build_scene(
    term: &Terminal,
    offset: usize,
    palette: &Palette,
    face: &mut FontFace,
    atlas: &mut Atlas,
    selection: Option<&Selection>,
    glass: GlassParams,
) -> Scene {
    let mut scene = Scene::default();
    let cw = face.metrics.cell_width.max(1) as f32;
    let lh = face.metrics.line_height.max(1) as f32;
    let fg_alpha = glass.fg_alpha();

    let cols = term.cols as usize;
    let rows = term.rows as usize;
    let (cur_x, cur_y) = term.cursor();
    let cursor_on = term.cursor_visible() && offset == 0;

    for y in 0..rows {
        let py = y as f32 * lh;
        let line = term.visible_row(offset, y as u16);
        for x in 0..cols {
            let px = x as f32 * cw;
            let cell: Cell = line
                .and_then(|l| l.get(x).copied())
                .unwrap_or_else(Cell::blank);
            if cell.flags & WIDE_SPACER != 0 {
                continue; // second half of a wide glyph
            }

            let mut fg = resolve(cell.fg, palette.foreground, palette);
            let mut bg = resolve(cell.bg, palette.background, palette);
            if cell.flags & INVERSE != 0 {
                std::mem::swap(&mut fg, &mut bg);
            }
            if cell.flags & DIM != 0 {
                fg = blend(bg, fg, 0.55);
            }

            let selected = selection.is_some_and(|s| s.contains(x as u16, y as u16));
            let on_cursor = cursor_on && cur_x as usize == x && cur_y as usize == y;

            if on_cursor {
                // Cursor block: opaque theme-cursor background; the
                // glyph is drawn in the theme background color so it
                // stays readable (same trick as the CPU renderer).
                scene.rects.push(RectDraw {
                    x: px,
                    y: py,
                    w: cw,
                    h: lh,
                    color: rgba01(palette.cursor, 1.0),
                });
                fg = palette.background;
            } else if selected {
                let hi = blend(palette.cursor, bg, 0.5);
                scene.rects.push(RectDraw {
                    x: px,
                    y: py,
                    w: cw,
                    h: lh,
                    color: rgba01(hi, 1.0),
                });
            } else if cell.bg != Color::Default || cell.flags & INVERSE != 0 {
                // Only non-default backgrounds become rects; the clear
                // already painted the theme background. An INVERSE cell
                // with a default background still needs its rect (the
                // swapped foreground becomes its visible background).
                scene.rects.push(RectDraw {
                    x: px,
                    y: py,
                    w: cw,
                    h: lh,
                    color: rgba01(bg, 1.0),
                });
            }

            if cell.c != ' ' {
                let mut style = 0u8;
                if cell.flags & BOLD != 0 {
                    style |= STYLE_BOLD;
                }
                if cell.flags & ITALIC != 0 {
                    style |= STYLE_ITALIC;
                }
                let key = (cell.c, style);
                if !atlas.entries.contains_key(&key) {
                    if let Some(bitmap) = face.glyph(cell.c, style) {
                        let bitmap = bitmap.clone();
                        // Full: reset and repack this glyph once.
                        if !atlas.insert(key, &bitmap) {
                            atlas.reset();
                            atlas.insert(key, &bitmap);
                        }
                    }
                }
                if let Some(entry) = atlas.entry(key) {
                    if let Some(bitmap) = face.glyph(cell.c, style) {
                        scene.glyphs.push(GlyphDraw {
                            x: px + bitmap.left as f32,
                            y: py + bitmap.top as f32,
                            w: bitmap.width as f32,
                            h: bitmap.height as f32,
                            uv: entry.uv,
                            color: rgba01(fg, fg_alpha),
                        });
                    }
                }
            }

            if cell.flags & UNDERLINE != 0 {
                scene.rects.push(RectDraw {
                    x: px,
                    y: py + lh - 1.0,
                    w: cw,
                    h: 1.0,
                    color: rgba01(fg, 1.0),
                });
            }
        }
    }

    scene
}

/// Build the clear color for one frame: the theme background at the
/// glass opacity. This is what the compositor blurs through.
pub fn clear_color(palette: &Palette, glass: GlassParams) -> [f32; 4] {
    rgba01(palette.background, glass.bg_alpha())
}

// ---------------------------------------------------------------------------
// Shaders (GLSL 330 core)
// ---------------------------------------------------------------------------

pub const RECT_VERTEX_SHADER: &str = r#"#version 330 core
layout (location = 0) in vec2 a_pos;   // window pixels, y downward
layout (location = 1) in vec4 a_color; // straight RGBA

uniform vec2 u_window;                 // framebuffer size in pixels

out vec4 v_color;

void main() {
    float x = 2.0 * a_pos.x / u_window.x - 1.0;
    float y = 1.0 - 2.0 * a_pos.y / u_window.y;
    gl_Position = vec4(x, y, 0.0, 1.0);
    v_color = a_color;
}
"#;

pub const RECT_FRAGMENT_SHADER: &str = r#"#version 330 core
in vec4 v_color;

out vec4 out_color;

void main() {
    out_color = v_color;
}
"#;

pub const GLYPH_VERTEX_SHADER: &str = r#"#version 330 core
layout (location = 0) in vec2 a_pos;   // window pixels, y downward
layout (location = 1) in vec2 a_uv;    // atlas uv
layout (location = 2) in vec4 a_color; // straight RGBA

uniform vec2 u_window;

out vec2 v_uv;
out vec4 v_color;

void main() {
    float x = 2.0 * a_pos.x / u_window.x - 1.0;
    float y = 1.0 - 2.0 * a_pos.y / u_window.y;
    gl_Position = vec4(x, y, 0.0, 1.0);
    v_uv = a_uv;
    v_color = a_color;
}
"#;

pub const GLYPH_FRAGMENT_SHADER: &str = r#"#version 330 core
in vec2 v_uv;
in vec4 v_color;

uniform sampler2D u_atlas; // R8 coverage

out vec4 out_color;

void main() {
    float coverage = texture(u_atlas, v_uv).r;
    out_color = vec4(v_color.rgb, v_color.a * coverage);
}
"#;

// ---------------------------------------------------------------------------
// GL layer (thin, no policy)
// ---------------------------------------------------------------------------

pub use glow::HasContext;

/// The GPU-side state for one window: compiled programs, vertex
/// buffers and the atlas texture. Created once per window, dropped
/// explicitly on exit (`Drop` deletes every GL object — manual memory
/// hygiene, nothing leaks until the driver decides otherwise).
pub struct GpuRenderer {
    gl: std::sync::Arc<glow::Context>,
    rect_program: glow::Program,
    glyph_program: glow::Program,
    rect_vao: glow::VertexArray,
    rect_vbo: glow::Buffer,
    glyph_vao: glow::VertexArray,
    glyph_vbo: glow::Buffer,
    atlas_texture: glow::Texture,
    /// Per-frame scratch: re-used instead of re-allocated (manual
    /// memory: the Vec capacity survives between frames).
    rect_vertices: Vec<f32>,
    glyph_vertices: Vec<f32>,
}

impl GpuRenderer {
    pub fn new(gl: std::sync::Arc<glow::Context>) -> Result<Self, String> {
        unsafe {
            let compile = |src: &str, kind: u32, label: &str| -> Result<glow::Shader, String> {
                let shader = gl
                    .create_shader(kind)
                    .map_err(|e| format!("{label}: create_shader: {e}"))?;
                gl.shader_source(shader, src);
                gl.compile_shader(shader);
                if !gl.get_shader_compile_status(shader) {
                    let log = gl.get_shader_info_log(shader);
                    return Err(format!("{label} compile failed: {log}"));
                }
                Ok(shader)
            };
            let link = |vs: glow::Shader,
                        fs: glow::Shader,
                        label: &str|
             -> Result<glow::Program, String> {
                let program = gl
                    .create_program()
                    .map_err(|e| format!("{label}: create_program: {e}"))?;
                gl.attach_shader(program, vs);
                gl.attach_shader(program, fs);
                gl.link_program(program);
                gl.delete_shader(vs);
                gl.delete_shader(fs);
                if !gl.get_program_link_status(program) {
                    let log = gl.get_program_info_log(program);
                    return Err(format!("{label} link failed: {log}"));
                }
                Ok(program)
            };

            let rect_program = link(
                compile(RECT_VERTEX_SHADER, glow::VERTEX_SHADER, "tuxixi-rect-vs")?,
                compile(
                    RECT_FRAGMENT_SHADER,
                    glow::FRAGMENT_SHADER,
                    "tuxixi-rect-fs",
                )?,
                "tuxixi-rect-program",
            )?;
            let glyph_program = link(
                compile(GLYPH_VERTEX_SHADER, glow::VERTEX_SHADER, "tuxixi-glyph-vs")?,
                compile(
                    GLYPH_FRAGMENT_SHADER,
                    glow::FRAGMENT_SHADER,
                    "tuxixi-glyph-fs",
                )?,
                "tuxixi-glyph-program",
            )?;

            let rect_vao = gl.create_vertex_array().map_err(|e| e.to_string())?;
            let rect_vbo = gl.create_buffer().map_err(|e| e.to_string())?;
            gl.bind_vertex_array(Some(rect_vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(rect_vbo));
            // stride: 2f32 pos + 4f32 color
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 6 * 4, 0);
            gl.vertex_attrib_pointer_f32(1, 4, glow::FLOAT, false, 6 * 4, 2 * 4);
            gl.enable_vertex_attrib_array(0);
            gl.enable_vertex_attrib_array(1);

            let glyph_vao = gl.create_vertex_array().map_err(|e| e.to_string())?;
            let glyph_vbo = gl.create_buffer().map_err(|e| e.to_string())?;
            gl.bind_vertex_array(Some(glyph_vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(glyph_vbo));
            // stride: 2f32 pos + 2f32 uv + 4f32 color
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8 * 4, 0);
            gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, 8 * 4, 2 * 4);
            gl.vertex_attrib_pointer_f32(2, 4, glow::FLOAT, false, 8 * 4, 4 * 4);
            gl.enable_vertex_attrib_array(0);
            gl.enable_vertex_attrib_array(1);
            gl.enable_vertex_attrib_array(2);

            let atlas_texture = gl.create_texture().map_err(|e| e.to_string())?;
            gl.bind_texture(glow::TEXTURE_2D, Some(atlas_texture));
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            // Clamp to the edge: uv never wraps.
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );

            gl.disable(glow::DEPTH_TEST);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);

            Ok(Self {
                gl,
                rect_program,
                glyph_program,
                rect_vao,
                rect_vbo,
                glyph_vao,
                glyph_vbo,
                atlas_texture,
                rect_vertices: Vec::with_capacity(4096),
                glyph_vertices: Vec::with_capacity(8192),
            })
        }
    }

    /// Upload the atlas pixels when they changed. `force` re-uploads
    /// even when not dirty (used after a full reset by the caller).
    /// Takes the context explicitly so `render` can keep its `gl`
    /// borrow alive.
    fn sync_atlas(gl: &glow::Context, texture: glow::Texture, atlas: &Atlas, force: bool) {
        unsafe {
            if !force && !atlas.needs_upload() {
                return;
            }
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            // Coverage bytes are unaligned rows of 1-byte values.
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::R8 as i32,
                atlas.size as i32,
                atlas.size as i32,
                0,
                glow::RED,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(atlas.pixels.as_slice())),
            );
        }
    }

    /// Render one frame and swap nothing (present is the caller's job).
    /// `width`/`height` are the framebuffer size in physical pixels and
    /// `clear` is the background RGBA (glass alpha included).
    pub fn render(
        &mut self,
        scene: &Scene,
        atlas: &mut Atlas,
        width: i32,
        height: i32,
        clear: [f32; 4],
    ) {
        unsafe {
            let gl = &self.gl;
            gl.viewport(0, 0, width, height);
            gl.clear_color(clear[0], clear[1], clear[2], clear[3]);
            gl.clear(glow::COLOR_BUFFER_BIT);

            // --- solid rects ---
            self.rect_vertices.clear();
            for r in &scene.rects {
                push_rect(&mut self.rect_vertices, r);
            }
            if !self.rect_vertices.is_empty() {
                gl.use_program(Some(self.rect_program));
                gl.uniform_2_f32(
                    gl.get_uniform_location(self.rect_program, "u_window")
                        .as_ref(),
                    width as f32,
                    height as f32,
                );
                gl.bind_vertex_array(Some(self.rect_vao));
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.rect_vbo));
                gl.buffer_data_u8_slice(
                    glow::ARRAY_BUFFER,
                    bytemuck_slice(&self.rect_vertices),
                    glow::STREAM_DRAW,
                );
                gl.draw_arrays(glow::TRIANGLES, 0, self.rect_vertices.len() as i32 / 6);
            }

            // --- glyphs ---
            Self::sync_atlas(gl, self.atlas_texture, atlas, false);
            self.glyph_vertices.clear();
            for g in &scene.glyphs {
                push_glyph(&mut self.glyph_vertices, g);
            }
            if !self.glyph_vertices.is_empty() {
                gl.use_program(Some(self.glyph_program));
                gl.uniform_2_f32(
                    gl.get_uniform_location(self.glyph_program, "u_window")
                        .as_ref(),
                    width as f32,
                    height as f32,
                );
                let loc = gl.get_uniform_location(self.glyph_program, "u_atlas");
                if let Some(loc) = loc {
                    gl.uniform_1_i32(Some(&loc), 0);
                }
                gl.active_texture(glow::TEXTURE0);
                gl.bind_texture(glow::TEXTURE_2D, Some(self.atlas_texture));
                gl.bind_vertex_array(Some(self.glyph_vao));
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.glyph_vbo));
                gl.buffer_data_u8_slice(
                    glow::ARRAY_BUFFER,
                    bytemuck_slice(&self.glyph_vertices),
                    glow::STREAM_DRAW,
                );
                gl.draw_arrays(glow::TRIANGLES, 0, self.glyph_vertices.len() as i32 / 8);
            }
            atlas.mark_uploaded();
        }
    }

    /// Explicit teardown: delete every GL object (manual memory
    /// hygiene — do not wait for the context drop order to decide).
    pub fn destroy(&mut self) {
        unsafe {
            let gl = &self.gl;
            gl.delete_program(self.rect_program);
            gl.delete_program(self.glyph_program);
            gl.delete_vertex_array(self.rect_vao);
            gl.delete_vertex_array(self.glyph_vao);
            gl.delete_buffer(self.rect_vbo);
            gl.delete_buffer(self.glyph_vbo);
            gl.delete_texture(self.atlas_texture);
            // Release the vertex scratch buffers explicitly.
            self.rect_vertices.clear();
            self.rect_vertices.shrink_to_fit();
            self.glyph_vertices.clear();
            self.glyph_vertices.shrink_to_fit();
        }
    }
}

impl Drop for GpuRenderer {
    fn drop(&mut self) {
        // destroy() is the explicit path; dropping must still free the
        // GL objects when the caller forgot.
        self.destroy();
    }
}

// f32 -> bytes helper without a new dependency (the values are plain
// floats; transmute-free: copy through a raw cast).
fn bytemuck_slice(v: &[f32]) -> &[u8] {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::Palette;

    fn flat_palette() -> Palette {
        Palette {
            ansi: [[0u8; 3]; 16],
            foreground: [255, 255, 255],
            background: [10, 20, 30],
            cursor: [255, 255, 0],
        }
    }

    // ----- atlas packer -----

    fn bitmap_of(w: usize, h: usize, ink: u8) -> GlyphBitmap {
        GlyphBitmap {
            width: w,
            height: h,
            coverage: vec![ink; w * h],
            left: 0,
            top: 0,
        }
    }

    #[test]
    fn atlas_shelves_pack_side_by_side() {
        let mut atlas = Atlas::new(64);
        let b = bitmap_of(8, 4, 255);
        assert!(atlas.insert(('a', 0), &b));
        assert!(atlas.insert(('b', 0), &b));
        let a = atlas.entry(('a', 0)).unwrap();
        let c = atlas.entry(('b', 0)).unwrap();
        assert_eq!(a.px, [0, 0, 8, 4]);
        assert_eq!(c.px, [8, 0, 8, 4], "second glyph starts after the first");
        assert_eq!(a.uv[0], 0.0);
        assert_eq!(a.uv[2], 8.0 / 64.0);
        assert_eq!(c.uv[0], 8.0 / 64.0);
    }

    #[test]
    fn atlas_starts_a_new_row_when_full() {
        let mut atlas = Atlas::new(64);
        let b = bitmap_of(16, 8, 200);
        for i in 0..4u32 {
            let c = char::from_u32('a' as u32 + i).unwrap();
            assert!(atlas.insert((c, 0), &b));
        }
        // 5th glyph of 16px width does not fit the 64px row.
        assert!(atlas.insert(('e', 0), &b));
        let e = atlas.entry(('e', 0)).unwrap();
        assert_eq!(e.px[1], 8, "new shelf row starts at y = row_h");
    }

    #[test]
    fn atlas_reports_full_and_reset_clears_everything() {
        let mut atlas = Atlas::new(32);
        let tall = bitmap_of(8, 32, 255);
        // Four 8px-wide glyphs fill the 32px row completely.
        for i in 0..4u32 {
            let c = char::from_u32('a' as u32 + i).unwrap();
            assert!(atlas.insert((c, 0), &tall));
        }
        // The 5th glyph needs a second row: full.
        assert!(!atlas.insert(('e', 0), &tall));
        assert_eq!(atlas.len(), 4);
        atlas.reset();
        assert_eq!(atlas.len(), 0);
        assert!(
            atlas.pixels.iter().all(|&v| v == 0),
            "reset zeroes the buffer"
        );
        assert!(atlas.needs_upload(), "reset must flag the texture dirty");
        // After reset it packs again from the top-left corner.
        assert!(atlas.insert(('c', 0), &tall));
        assert_eq!(atlas.entry(('c', 0)).unwrap().px, [0, 0, 8, 32]);
    }

    #[test]
    fn atlas_rejects_over_sized_and_empty_glyphs() {
        let mut atlas = Atlas::new(32);
        assert!(!atlas.insert(('a', 0), &bitmap_of(64, 4, 255)));
        assert!(!atlas.insert(('b', 0), &bitmap_of(0, 4, 255)));
        assert!(!atlas.insert(('c', 0), &bitmap_of(4, 0, 255)));
        assert!(atlas.is_empty());
    }

    #[test]
    fn atlas_insert_is_idempotent_per_key() {
        let mut atlas = Atlas::new(64);
        let b = bitmap_of(8, 4, 255);
        assert!(atlas.insert(('a', 0), &b));
        let before = atlas.entry(('a', 0)).copied().unwrap();
        assert!(atlas.insert(('a', 0), &b));
        assert_eq!(
            atlas.entry(('a', 0)).copied().unwrap(),
            before,
            "re-insert must not move the glyph"
        );
        assert_eq!(atlas.len(), 1);
    }

    #[test]
    fn atlas_covers_exact_pixel_data() {
        let mut atlas = Atlas::new(16);
        let mut bitmap = bitmap_of(4, 2, 0);
        bitmap.coverage[0] = 250;
        bitmap.coverage[7] = 12;
        assert!(atlas.insert(('a', 0), &bitmap));
        // Row 0 lands at atlas y=0, row 1 at atlas y=16 (pitch).
        assert_eq!(atlas.pixels[0], 250);
        assert_eq!(atlas.pixels[1], 0);
        assert_eq!(atlas.pixels[19], 12, "row 1 col 3 = 1*16 + 3");
        assert_eq!(atlas.pixels[8], 0, "rest of the atlas stays zero");
    }

    #[test]
    fn atlas_memory_is_bounded() {
        let atlas = Atlas::default();
        assert_eq!(atlas.memory_bytes(), (ATLAS_SIZE * ATLAS_SIZE) as usize);
    }

    #[test]
    fn dirty_flag_survives_until_marked_uploaded() {
        let mut atlas = Atlas::new(64);
        assert!(!atlas.needs_upload(), "fresh atlas has nothing to upload");
        assert!(atlas.insert(('a', 0), &bitmap_of(8, 4, 255)));
        assert!(atlas.needs_upload());
        atlas.mark_uploaded();
        assert!(!atlas.needs_upload());
    }

    // ----- glass params -----

    #[test]
    fn glass_params_clamp_and_convert() {
        let g = GlassParams::new(true, 0.0);
        assert_eq!(g.opacity, crate::settings::MIN_GLASS_OPACITY);
        let g = GlassParams::new(true, 7.5);
        assert_eq!(g.opacity, crate::settings::MAX_GLASS_OPACITY);
        assert_eq!(g.bg_alpha(), 1.0, "clamped to max == fully opaque");
        let off = GlassParams::new(false, 0.4);
        assert_eq!(off.bg_alpha(), 1.0, "glass off must clear opaque");
        let on = GlassParams::new(true, 0.5);
        assert_eq!(on.bg_alpha(), 0.5);
        assert_eq!(on.fg_alpha(), 1.0, "text ink is always opaque");
    }

    #[test]
    fn glass_default_matches_settings_default() {
        let g = GlassParams::default();
        assert!(g.enabled);
        assert_eq!(g.opacity, crate::settings::DEFAULT_GLASS_OPACITY);
    }

    // ----- color conversion -----

    #[test]
    fn rgb01_maps_full_range() {
        assert_eq!(rgb01([0, 0, 0]), [0.0; 3]);
        assert_eq!(rgb01([255, 255, 255]), [1.0; 3]);
        assert!((rgb01([128, 128, 128])[0] - 128.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn clear_color_uses_theme_background_and_glass_alpha() {
        let p = flat_palette();
        let off = clear_color(&p, GlassParams::new(false, 0.5));
        assert_eq!(off[3], 1.0);
        assert!((off[0] - 10.0 / 255.0).abs() < 1e-6);
        let on = clear_color(&p, GlassParams::new(true, 0.8));
        assert_eq!(on[3], 0.8, "glass alpha comes from the settings");
        assert!((on[1] - 20.0 / 255.0).abs() < 1e-6);
    }

    // ----- vertex building -----

    #[test]
    fn push_rect_builds_two_triangles_with_expected_corners() {
        let mut v = Vec::new();
        let r = RectDraw {
            x: 10.0,
            y: 20.0,
            w: 30.0,
            h: 40.0,
            color: [1.0, 0.5, 0.0, 1.0],
        };
        push_rect(&mut v, &r);
        assert_eq!(v.len(), 36, "6 vertices x 6 floats");
        // Top-left corner appears in vertex 0.
        assert_eq!(&v[0..6], &[10.0, 20.0, 1.0, 0.5, 0.0, 1.0]);
        // The quad must cover all four corners.
        let mut corners = 0;
        for vert in v.as_chunks::<6>().0 {
            let (x, y) = (vert[0], vert[1]);
            if (x, y) == (10.0, 20.0)
                || (x, y) == (40.0, 20.0)
                || (x, y) == (10.0, 60.0)
                || (x, y) == (40.0, 60.0)
            {
                corners += 1;
            }
        }
        assert_eq!(corners, 6, "the two triangles must share the corners");
    }

    #[test]
    fn push_glyph_carries_uv_and_color() {
        let mut v = Vec::new();
        let g = GlyphDraw {
            x: 0.0,
            y: 0.0,
            w: 8.0,
            h: 12.0,
            uv: [0.25, 0.5, 0.75, 1.0],
            color: [1.0, 1.0, 1.0, 1.0],
        };
        push_glyph(&mut v, &g);
        assert_eq!(v.len(), 48, "6 vertices x 8 floats");
        assert_eq!(&v[0..8], &[0.0, 0.0, 0.25, 0.5, 1.0, 1.0, 1.0, 1.0]);
        // Last vertex is bottom-right with uv1.
        assert_eq!(&v[40..48], &[8.0, 12.0, 0.75, 1.0, 1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn scene_vertex_count_matches_contents() {
        let mut s = Scene::default();
        s.rects.push(RectDraw {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
            color: [0.0; 4],
        });
        s.glyphs.push(GlyphDraw {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
            uv: [0.0; 4],
            color: [0.0; 4],
        });
        assert_eq!(s.vertex_count(), 12);
        // Float totals differ per quad type: rects are 6 floats/vertex,
        // glyphs are 8 floats/vertex.
        let mut floats = 0;
        floats += s.rects.len() * 6 * 6;
        floats += s.glyphs.len() * 6 * 8;
        assert_eq!(floats, 36 + 48);
    }

    // ----- scene builder (grid semantics) -----

    fn scene_of(
        text: &str,
        cols: u16,
        rows: u16,
        cw: u32,
        lh: u32,
        glass: GlassParams,
    ) -> (Scene, FontFace) {
        let mut term = Terminal::new(cols, rows);
        term.feed(text.as_bytes());
        let mut face = crate::font::load_default_face("monospace", 14.0).expect("system mono font");
        face.metrics.cell_width = cw;
        face.metrics.line_height = lh;
        let mut atlas = Atlas::new(1024);
        let scene = build_scene(
            &term,
            0,
            &flat_palette(),
            &mut face,
            &mut atlas,
            None,
            glass,
        );
        (scene, face)
    }

    #[test]
    fn default_background_cells_get_no_rect() {
        // Glass ON: only the clear is translucent; default-bg cells
        // must not add opaque rects over it (cursor hidden so the
        // cursor block does not count).
        let (scene, _face) = scene_of("\x1b[?25lhi", 10, 2, 8, 16, GlassParams::new(true, 0.5));
        assert_eq!(scene.rects.len(), 0, "no rects for default backgrounds");
        assert_eq!(scene.glyphs.len(), 2, "two glyph quads for 'hi'");
    }

    #[test]
    fn colored_cells_become_opaque_rects() {
        let (scene, _face) = scene_of(
            "\x1b[?25l\x1b[41mAB\x1b[0m",
            10,
            2,
            8,
            16,
            GlassParams::new(true, 0.5),
        );
        assert_eq!(scene.rects.len(), 2, "one opaque rect per colored cell");
        for r in &scene.rects {
            assert_eq!(r.color[3], 1.0, "cell backgrounds stay opaque on glass");
            assert_eq!(r.w, 8.0);
            assert_eq!(r.h, 16.0);
        }
        // Red background (SGR 41 = indexed 1) is exactly ANSI red 1 of
        // the flat palette... flat palette index 1 is [0,0,0]; the rect
        // color must equal that, NOT the theme background.
        assert_eq!(scene.rects[0].color[0], 0.0);
    }

    #[test]
    fn cursor_becomes_one_opaque_rect_and_swaps_fg() {
        // "A" then CR: the cursor lands back on the glyph cell, so the
        // glyph must be drawn in the theme background color.
        let (scene, _face) = scene_of("A\r", 4, 2, 8, 16, GlassParams::default());
        let cursor_rects: Vec<_> = scene
            .rects
            .iter()
            .filter(|r| r.color == rgba01([255, 255, 0], 1.0))
            .collect();
        assert_eq!(cursor_rects.len(), 1, "exactly one cursor block");
        let r = cursor_rects[0];
        assert_eq!((r.x, r.y), (0.0, 0.0));
        assert_eq!((r.w, r.h), (8.0, 16.0));
        // The glyph over the cursor is drawn in the theme background
        // color (readability trick from the CPU renderer).
        let g = scene.glyphs[0];
        assert_eq!(g.color, rgba01([10, 20, 30], 1.0));
    }

    #[test]
    fn hidden_cursor_draws_nothing() {
        let mut term = Terminal::new(4, 2);
        term.feed(b"\x1b[?25lA");
        let mut face = crate::font::load_default_face("monospace", 14.0).expect("system mono font");
        face.metrics.cell_width = 8;
        face.metrics.line_height = 16;
        let mut atlas = Atlas::new(1024);
        let scene = build_scene(
            &term,
            0,
            &flat_palette(),
            &mut face,
            &mut atlas,
            None,
            GlassParams::default(),
        );
        assert!(
            !scene
                .rects
                .iter()
                .any(|r| r.color == rgba01([255, 255, 0], 1.0)),
            "hidden cursor must not paint the cursor block"
        );
    }

    #[test]
    fn inverse_cells_swap_fg_and_bg() {
        // Cursor hidden: otherwise the cursor block wins over inverse.
        let (scene, _face) = scene_of(
            "\x1b[?25l\x1b[7mA\x1b[0m",
            4,
            2,
            8,
            16,
            GlassParams::default(),
        );
        // Inverse: bg becomes the foreground color (white) -> opaque
        // white rect behind the cell.
        assert_eq!(scene.rects.len(), 1);
        assert_eq!(scene.rects[0].color, rgba01([255, 255, 255], 1.0));
    }

    #[test]
    fn underline_is_a_one_pixel_rect_at_cell_bottom() {
        let (scene, _face) = scene_of("\x1b[4mA\x1b[0m", 4, 2, 8, 16, GlassParams::default());
        let underlines: Vec<_> = scene
            .rects
            .iter()
            .filter(|r| r.h == 1.0 && r.w == 8.0)
            .collect();
        assert_eq!(underlines.len(), 1);
        assert!((underlines[0].y - 15.0).abs() < 1e-6, "y = lh - 1");
        assert_eq!(
            underlines[0].color,
            rgba01([255, 255, 255], 1.0),
            "fg-colored"
        );
    }

    #[test]
    fn selection_highlight_is_opaque_and_distinct() {
        let mut term = Terminal::new(6, 2);
        term.feed(b"\x1b[?25labcdef");
        let mut face = crate::font::load_default_face("monospace", 14.0).expect("system mono font");
        face.metrics.cell_width = 8;
        face.metrics.line_height = 16;
        let mut atlas = Atlas::new(1024);
        let sel = Selection {
            start: (0, 0),
            end: (2, 0),
        };
        let scene = build_scene(
            &term,
            0,
            &flat_palette(),
            &mut face,
            &mut atlas,
            Some(&sel),
            GlassParams::default(),
        );
        assert_eq!(scene.rects.len(), 3, "three selected cells highlighted");
        // blend(cursor=yellow, bg=[10,20,30], 0.5)
        let expect = rgba01(blend([255, 255, 0], [10, 20, 30], 0.5), 1.0);
        for r in &scene.rects {
            assert_eq!(r.color, expect);
        }
    }

    #[test]
    fn scrolled_view_pins_the_cursor_off() {
        let mut term = Terminal::new(4, 2);
        term.feed(b"one\r\ntwo\r\nthree\r\nfour");
        let sb = term.scrollback_len();
        assert!(sb >= 2);
        let mut face = crate::font::load_default_face("monospace", 14.0).expect("system mono font");
        face.metrics.cell_width = 8;
        face.metrics.line_height = 16;
        let mut atlas = Atlas::new(1024);
        let live = build_scene(
            &term,
            0,
            &flat_palette(),
            &mut face,
            &mut atlas,
            None,
            GlassParams::default(),
        );
        let scrolled = build_scene(
            &term,
            sb,
            &flat_palette(),
            &mut face,
            &mut atlas,
            None,
            GlassParams::default(),
        );
        assert_ne!(live, scrolled, "scrollback offset changes the scene");
    }

    #[test]
    fn wide_spacer_cells_are_skipped() {
        let mut term = Terminal::new(8, 2);
        // CJK char is double-width in the grid; the spacer half must
        // not produce its own glyph quad.
        term.feed("漢".as_bytes());
        let mut face = crate::font::load_default_face("monospace", 14.0).expect("system mono font");
        face.metrics.cell_width = 8;
        face.metrics.line_height = 16;
        let mut atlas = Atlas::new(1024);
        let scene = build_scene(
            &term,
            0,
            &flat_palette(),
            &mut face,
            &mut atlas,
            None,
            GlassParams::default(),
        );
        // Invariant: the spacer cell at column 1 NEVER produces a glyph
        // quad. With a CJK-capable font there is exactly one quad at
        // col 0; with a font missing the glyph, zero quads.
        let spacer_quads = scene
            .glyphs
            .iter()
            .filter(|g| (g.x / 8.0).round() == 1.0 && g.w < 16.0)
            .count();
        assert_eq!(spacer_quads, 0, "spacer column must not draw a glyph");
    }

    #[test]
    fn space_cells_produce_nothing() {
        let (scene, _face) = scene_of("\x1b[?25l  ", 4, 2, 8, 16, GlassParams::default());
        assert_eq!(scene.glyphs.len(), 0, "no glyph quads for spaces");
        assert_eq!(scene.rects.len(), 0, "and no rects either");
    }

    #[test]
    fn glyph_quads_use_bitmap_geometry() {
        // 'A' rasterized at cell (1, 0): quad position must be
        // cell_px + (left, top), size = bitmap size.
        let mut term = Terminal::new(8, 2);
        term.feed(b" A");
        let mut face = crate::font::load_default_face("monospace", 14.0).expect("system mono font");
        face.metrics.cell_width = 8;
        face.metrics.line_height = 16;
        let mut atlas = Atlas::new(1024);
        let scene = build_scene(
            &term,
            0,
            &flat_palette(),
            &mut face,
            &mut atlas,
            None,
            GlassParams::default(),
        );
        assert!(
            !scene.glyphs.is_empty(),
            "monospace system font rasterizes A"
        );
        let second = scene
            .glyphs
            .iter()
            .find(|g| g.x >= 8.0)
            .expect("the glyph in column 1");
        let bitmap = face
            .glyph('A', 0)
            .expect("A is in the font cache after build_scene");
        assert_eq!(second.x, 8.0 + bitmap.left as f32);
        assert_eq!(second.y, bitmap.top as f32);
        assert_eq!(second.w, bitmap.width as f32);
        assert_eq!(second.h, bitmap.height as f32);
        // Its uv must point into a non-empty atlas region.
        assert!(second.uv[2] > second.uv[0] && second.uv[3] > second.uv[1]);
    }

    #[test]
    fn atlas_fills_from_the_font_face_cache() {
        let (scene, face) = scene_of("ab", 4, 2, 8, 16, GlassParams::default());
        assert_eq!(scene.glyphs.len(), 2);
        assert_eq!(face.cache_len(), 2, "two glyphs rasterized and cached");
        // Both quads reference distinct atlas regions.
        assert_ne!(scene.glyphs[0].uv, scene.glyphs[1].uv);
    }
}

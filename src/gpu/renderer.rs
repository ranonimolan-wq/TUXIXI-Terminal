// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! The OpenGL renderer: compiles the programs, uploads the atlas and
//! per-frame vertex buffers, and draws a frame. This is the only GL
//! stateful layer; all policy lives upstream in `build_scene`.

use glow::HasContext;

use super::atlas::Atlas;
use super::draw::{bytemuck_slice, push_glyph, push_rect, Scene};
use super::shader::{
    shader_source, GLYPH_FRAGMENT_SHADER_BODY, GLYPH_VERTEX_SHADER_BODY, RECT_FRAGMENT_SHADER_BODY,
    RECT_VERTEX_SHADER_BODY,
};

// GL layer (thin, no policy)
// ---------------------------------------------------------------------------

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
            // ES contexts report an embedded version string ("OpenGL ES
            // 3.x"); the shaders need the ES header for those.
            let is_embedded = gl.version().is_embedded;
            eprintln!(
                "GPU renderer: {} mode",
                if is_embedded {
                    "OpenGL ES 3.0"
                } else {
                    "desktop OpenGL 3.3"
                }
            );
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
                compile(
                    &shader_source(RECT_VERTEX_SHADER_BODY, is_embedded),
                    glow::VERTEX_SHADER,
                    "tuxixi-rect-vs",
                )?,
                compile(
                    &shader_source(RECT_FRAGMENT_SHADER_BODY, is_embedded),
                    glow::FRAGMENT_SHADER,
                    "tuxixi-rect-fs",
                )?,
                "tuxixi-rect-program",
            )?;
            let glyph_program = link(
                compile(
                    &shader_source(GLYPH_VERTEX_SHADER_BODY, is_embedded),
                    glow::VERTEX_SHADER,
                    "tuxixi-glyph-vs",
                )?,
                compile(
                    &shader_source(GLYPH_FRAGMENT_SHADER_BODY, is_embedded),
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

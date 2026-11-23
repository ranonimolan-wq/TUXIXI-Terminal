// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! GLSL shader sources. The same bodies are compiled for desktop GL
//! 3.3 core and OpenGL ES 3.0; [`shader_source`] prepends the matching
//! version header (and precision qualifiers for ES).

// ---------------------------------------------------------------------------
// Shaders — written once, compiled for BOTH desktop GL 3.3 core and
// OpenGL ES 3.0 (the only per-context difference is the header: ES
// needs `#version 300 es` + an explicit precision qualifier, desktop
// needs `#version 330 core`). Old Intel GPUs (i915/gen3) have no core
// profile at all and answer `EGL_BAD_MATCH` to every desktop GL 3.3
// request — ES 3.0 is their only modern context (same fallback
// alacritty uses), so the renderer must speak both dialects.
// ---------------------------------------------------------------------------

pub const RECT_VERTEX_SHADER_BODY: &str = r#"
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

pub const RECT_FRAGMENT_SHADER_BODY: &str = r#"
in vec4 v_color;

out vec4 out_color;

void main() {
    out_color = v_color;
}
"#;

pub const GLYPH_VERTEX_SHADER_BODY: &str = r#"
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

pub const GLYPH_FRAGMENT_SHADER_BODY: &str = r#"
in vec2 v_uv;
in vec4 v_color;

uniform sampler2D u_atlas; // R8 coverage

out vec4 out_color;

void main() {
    float coverage = texture(u_atlas, v_uv).r;
    out_color = vec4(v_color.rgb, v_color.a * coverage);
}
"#;

/// Build the full shader source for the context's GL flavor.
///
/// ES needs `#version 300 es` + an explicit precision qualifier
/// (`highp` is mandatory in ES 3.0 fragment shaders, so it is safe in
/// every stage); desktop GLSL 330 core rejects the ES headers.
pub(crate) fn shader_source(body: &str, is_embedded: bool) -> String {
    if is_embedded {
        format!("#version 300 es\nprecision highp float;\n{body}")
    } else {
        format!("#version 330 core\n{body}")
    }
}

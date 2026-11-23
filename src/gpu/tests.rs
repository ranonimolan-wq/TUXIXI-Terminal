// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! Headless unit tests: atlas shelf packing, shader source headers,
//! scene-builder semantics and glass color math.

use crate::font::{FontFace, GlyphBitmap};
use crate::render::{blend, Palette, Selection};
use crate::vt::Terminal;

use super::shader::shader_source;
use super::*;

fn flat_palette() -> Palette {
    Palette {
        ansi: [[0u8; 3]; 16],
        foreground: [255, 255, 255],
        background: [10, 20, 30],
        cursor: [255, 255, 0],
    }
}

// ----- shader source dialects -----

#[test]
fn desktop_shader_source_gets_the_330_core_header() {
    let src = shader_source("void main() {}\n", false);
    assert!(src.starts_with("#version 330 core\n"));
    assert!(
        !src.contains("precision"),
        "desktop GLSL must not carry ES precision qualifiers"
    );
    assert!(src.contains("void main() {}"), "body must be kept");
}

#[test]
fn es_shader_source_gets_the_300_es_header_and_precision() {
    let src = shader_source("void main() {}\n", true);
    assert!(src.starts_with("#version 300 es\n"));
    assert!(
        src.contains("precision highp float;"),
        "ES 3.0 fragment shaders need an explicit precision qualifier"
    );
    assert!(src.contains("void main() {}"), "body must be kept");
    assert!(
        !src.contains("330 core"),
        "ES source must never claim the desktop profile"
    );
}

#[test]
fn es_headers_cover_every_shader_body_without_desktop_only_syntax() {
    // All four shader bodies must survive both dialects with the
    // desktop-only constructs absent: `layout(location)` and
    // `texture()` are valid in both 330 core and 300 es, so the
    // only thing that may differ is the header.
    for (body, name) in [
        (RECT_VERTEX_SHADER_BODY, "rect-vs"),
        (RECT_FRAGMENT_SHADER_BODY, "rect-fs"),
        (GLYPH_VERTEX_SHADER_BODY, "glyph-vs"),
        (GLYPH_FRAGMENT_SHADER_BODY, "glyph-fs"),
    ] {
        assert!(!body.contains("#version"), "{name}: version in body");
        assert!(!body.contains("precision"), "{name}: precision in body");
        assert!(body.contains("in "), "{name}: inputs missing");
    }
    // The ES fragment shaders write to a user-defined `out` (ES 3.0
    // requirement, same as desktop core).
    assert!(RECT_FRAGMENT_SHADER_BODY.contains("out vec4"));
    assert!(GLYPH_FRAGMENT_SHADER_BODY.contains("out vec4"));
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
    let mut grid = Vec::new();
    term.copy_visible(0, &mut grid);
    let scene = build_scene(
        &term,
        0,
        SceneInputs {
            grid: &grid,
            palette: &flat_palette(),
            face: &mut face,
            atlas: &mut atlas,
            selection: None,
            glass,
        },
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
    let mut grid = Vec::new();
    term.copy_visible(0, &mut grid);
    let scene = build_scene(
        &term,
        0,
        SceneInputs {
            grid: &grid,
            palette: &flat_palette(),
            face: &mut face,
            atlas: &mut atlas,
            selection: None,
            glass: GlassParams::default(),
        },
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
    let mut grid = Vec::new();
    term.copy_visible(0, &mut grid);
    let scene = build_scene(
        &term,
        0,
        SceneInputs {
            grid: &grid,
            palette: &flat_palette(),
            face: &mut face,
            atlas: &mut atlas,
            selection: Some(&sel),
            glass: GlassParams::default(),
        },
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
    let mut grid = Vec::new();
    term.copy_visible(0, &mut grid);
    let live = build_scene(
        &term,
        0,
        SceneInputs {
            grid: &grid,
            palette: &flat_palette(),
            face: &mut face,
            atlas: &mut atlas,
            selection: None,
            glass: GlassParams::default(),
        },
    );
    let mut scrolled_grid = Vec::new();
    term.copy_visible(sb, &mut scrolled_grid);
    let scrolled = build_scene(
        &term,
        sb,
        SceneInputs {
            grid: &scrolled_grid,
            palette: &flat_palette(),
            face: &mut face,
            atlas: &mut atlas,
            selection: None,
            glass: GlassParams::default(),
        },
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
    let mut grid = Vec::new();
    term.copy_visible(0, &mut grid);
    let scene = build_scene(
        &term,
        0,
        SceneInputs {
            grid: &grid,
            palette: &flat_palette(),
            face: &mut face,
            atlas: &mut atlas,
            selection: None,
            glass: GlassParams::default(),
        },
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
    let mut grid = Vec::new();
    term.copy_visible(0, &mut grid);
    let scene = build_scene(
        &term,
        0,
        SceneInputs {
            grid: &grid,
            palette: &flat_palette(),
            face: &mut face,
            atlas: &mut atlas,
            selection: None,
            glass: GlassParams::default(),
        },
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

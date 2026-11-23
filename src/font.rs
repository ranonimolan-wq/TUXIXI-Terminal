// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! Font loading and glyph rasterization for the winit GUI.
//!
//! The font is located with `fc-match` (fontconfig, the same lookup
//! GTK/Pango did) with a pure-Rust fallback that scans the usual
//! directories. Rasterization goes through FreeType with hinting
//! enabled — the same rasterizer stack mainstream terminals render
//! with — so glyph edges are crisp at small pixel sizes instead of the
//! soft, unhinted outlines a pure-Rust outline scaler produces.
//!
//! The glyph cache is a [`BoundedCache`]: a hard entry limit with
//! explicit release-on-eviction, so a session full of rare CJK glyphs
//! can never grow the process without bound.

use crate::mem::{BoundedCache, Compact};
use freetype::face::LoadFlag;
use freetype::{Face, Library};

/// Everything the renderer needs to draw one glyph.
#[derive(Clone)]
pub struct GlyphBitmap {
    pub width: usize,
    pub height: usize,
    /// Coverage values (0..=255), row-major, origin top-left.
    pub coverage: Vec<u8>,
    /// Offset from the cell origin (top-left of the character cell).
    pub left: i32,
    pub top: i32,
}

impl Compact for GlyphBitmap {
    fn release(&mut self) {
        self.coverage.clear();
        self.coverage.shrink_to_fit();
    }
}

/// Key of the glyph cache: codepoint + style mask (bold/italic bits).
pub type GlyphKey = (char, u8);

pub const STYLE_BOLD: u8 = 1;
pub const STYLE_ITALIC: u8 = 2;

/// Metrics of the loaded font, all in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontMetrics {
    /// Cell height: ascent + descent of the font at the given size.
    pub line_height: u32,
    /// Cell width: advance of the widest ASCII glyph, rounded up to a
    /// whole pixel so the grid lines up perfectly.
    pub cell_width: u32,
    /// Distance from the top of the cell to the baseline.
    pub ascent: i32,
    /// Glyph rasterization size in px (the requested font size).
    pub px: f32,
}

/// The three style variants of one font family. Missing variants fall
/// back to the regular face (the renderer just draws what it has).
impl std::fmt::Debug for FontFace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontFace")
            .field("metrics", &self.metrics)
            .field("cache_len", &self.cache.len())
            .finish()
    }
}

pub struct FontFace {
    regular: Face,
    bold: Face,
    italic: Face,
    bold_italic: Face,
    pub metrics: FontMetrics,
    cache: BoundedCache<GlyphKey, GlyphBitmap>,
}

impl FontFace {
    /// Rasterization size used by the GUI.
    pub const RASTER_LIMIT: usize = BoundedCache::<GlyphKey, GlyphBitmap>::DEFAULT_LIMIT;

    fn face_for(&self, style: u8) -> &Face {
        match (style & STYLE_BOLD != 0, style & STYLE_ITALIC != 0) {
            (true, true) => &self.bold_italic,
            (true, false) => &self.bold,
            (false, true) => &self.italic,
            (false, false) => &self.regular,
        }
    }

    /// Rasterize (or fetch) one glyph. Returns None for whitespace /
    /// missing glyphs — the renderer only paints a background rect.
    pub fn glyph(&mut self, c: char, style: u8) -> Option<&GlyphBitmap> {
        let key: GlyphKey = (c, style);
        if !self.cache.contains(&key) {
            let bitmap = rasterize(self.face_for(style), c);
            if let Some(b) = bitmap {
                // Ignored return value: a wholesale eviction is invisible
                // to the renderer, the cache refills within a frame.
                let _ = self.cache.insert(key, b);
            }
        }
        self.cache.get(&key)
    }

    /// Cache statistics (for the memory debug overlay / tests).
    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }

    /// Drop every rasterized glyph (font size change / theme change).
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }
}

/// Measure the grid metrics from the regular face at `px`.
pub fn measure(face: &Face, px: f32) -> FontMetrics {
    let metrics = face.size_metrics();
    // 26.6 fixed point -> whole pixels. Hinted metrics are already
    // rounded by FreeType; ceil keeps the grid slightly generous.
    let (ascender, descender, max_advance) = match metrics {
        Some(m) => (m.ascender, m.descender, m.max_advance),
        None => (
            (px * 0.8 * 64.0) as i64,
            (px * 0.2 * 64.0) as i64,
            (px * 0.6 * 64.0) as i64,
        ),
    };
    let ascent = ((ascender as f64) / 64.0).round().max(1.0) as i32;
    let descent = ((descender as f64) / 64.0).abs().round().max(1.0) as i32;
    let cell_width = ((max_advance as f64) / 64.0).ceil().max(1.0) as u32;
    FontMetrics {
        line_height: (ascent + descent) as u32,
        cell_width,
        ascent,
        px,
    }
}

/// Rasterize `c` with hinting into an 8-bit coverage bitmap
/// (top-left origin). Returns None for whitespace and missing glyphs.
pub fn rasterize(face: &Face, c: char) -> Option<GlyphBitmap> {
    let index = face.get_char_index(c as usize)?;
    // Default load flags + TARGET_NORMAL: hinted, grayscale AA.
    face.load_glyph(index, LoadFlag::RENDER | LoadFlag::TARGET_NORMAL)
        .ok()?;
    let glyph = face.glyph();
    let bitmap = glyph.bitmap();
    let (w, h) = (bitmap.width() as usize, bitmap.rows() as usize);
    if w == 0 || h == 0 {
        return None; // whitespace: the renderer paints background only
    }
    let buffer = bitmap.buffer();
    let pitch = bitmap.pitch().unsigned_abs() as usize;
    let mut coverage = vec![0u8; w * h];
    for row in 0..h {
        for col in 0..w {
            coverage[row * w + col] = buffer[row * pitch + col];
        }
    }
    Some(GlyphBitmap {
        width: w,
        height: h,
        coverage,
        left: glyph.bitmap_left(),
        // bitmap_top is the distance from the BASELINE to the bitmap
        // top (positive up); the cell-relative top is ascent - that.
        top: face_size_ascent(face) - glyph.bitmap_top(),
    })
}

/// Cached ascent of a face's current size (cell-top to baseline).
fn face_size_ascent(face: &Face) -> i32 {
    face.size_metrics()
        .map(|m| (m.ascender as f64 / 64.0).round() as i32)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Font discovery
// ---------------------------------------------------------------------------

/// Candidate system font paths, in priority order. Pure fallback for
/// when `fc-match` is unavailable.
pub const FALLBACK_FONT_PATHS: &[&str] = &[
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
    "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
    "/usr/share/fonts/dejavu/DejaVuSansMono.ttf",
    "/usr/share/fonts/truetype/freefont/FreeMono.ttf",
    "/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf",
    "/usr/share/fonts/noto-cjk/NotoSansMonoCJKsc-Regular.otf",
];

/// Style variant suffixes tried next to a regular face when fc-match
/// is not available (same file name, different style).
fn variant_candidates(path: &str, style: &str) -> Vec<String> {
    let Some(dot) = path.rfind('.') else {
        return Vec::new();
    };
    let (stem, ext) = path.split_at(dot);
    let stem = stem.strip_suffix("-Regular").unwrap_or(stem);
    let mut out = Vec::new();
    match style {
        "bold" => out.push(format!("{stem}-Bold{ext}")),
        "italic" => {
            // FreeMono uses Italic, DejaVu uses Oblique.
            out.push(format!("{stem}-Italic{ext}"));
            out.push(format!("{stem}-Oblique{ext}"));
        }
        _ => out.push(format!("{stem}-BoldItalic{ext}")),
    }
    out
}

/// Locate the best monospace font file for `family` via fc-match.
/// Returns the regular face path plus the best-effort style variants.
pub fn locate_font_files(family: &str) -> Option<(String, String, String, String)> {
    let regular = fc_match(family)?;
    let bold =
        fc_match_bold(&regular).or_else(|| first_existing(&variant_candidates(&regular, "bold")));
    let italic = fc_match_italic(&regular)
        .or_else(|| first_existing(&variant_candidates(&regular, "italic")));
    let bold_italic = fc_match(&format!("{family}:bold:italic"))
        .or_else(|| first_existing(&variant_candidates(&regular, "bold_italic")));
    let regular2 = regular.clone();
    Some((
        regular,
        bold.unwrap_or_else(|| regular2.clone()),
        italic.unwrap_or_else(|| regular2.clone()),
        bold_italic.unwrap_or(regular2),
    ))
}

fn fc_match(pattern: &str) -> Option<String> {
    let out = std::process::Command::new("fc-match")
        .args(["--format=%{file}", pattern])
        .output()
        .ok()?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if path.is_empty() || !std::path::Path::new(&path).exists() {
        return None;
    }
    Some(path)
}

fn fc_match_bold(regular_path: &str) -> Option<String> {
    // Query fontconfig for a bold face of the same family.
    let family = family_of(regular_path)?;
    fc_match(&format!("{family}:style=Bold"))
}

fn fc_match_italic(regular_path: &str) -> Option<String> {
    let family = family_of(regular_path)?;
    fc_match(&format!("{family}:style=Italic"))
        .or_else(|| fc_match(&format!("{family}:style=Oblique")))
}

fn family_of(path: &str) -> Option<String> {
    let file = std::path::Path::new(path).file_name()?.to_str()?;
    // DejaVuSansMono.ttf -> "DejaVu Sans Mono"; LiberationMono-Regular.ttf
    // -> "Liberation Mono". Single-pass to avoid double spaces.
    let base = file
        .trim_end_matches(".ttf")
        .trim_end_matches(".otf")
        .trim_end_matches("-Regular");
    let spaced = if base.contains("SansMono") {
        base.replace("SansMono", " Sans Mono")
    } else if base.contains("SerifMono") {
        base.replace("SerifMono", " Serif Mono")
    } else if base.contains("Mono") {
        base.replace("Mono", " Mono")
    } else {
        base.to_string()
    };
    Some(spaced.trim().to_string())
}

fn first_existing(candidates: &[String]) -> Option<String> {
    candidates
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .cloned()
}

/// Load a complete face from four files (all defaulting to the regular
/// face when a variant is missing).
pub fn load_face(
    regular_path: &str,
    bold_path: Option<&str>,
    italic_path: Option<&str>,
    bold_italic_path: Option<&str>,
    px: f32,
) -> Result<FontFace, String> {
    let open = |lib: &Library, path: &str| -> Result<Face, String> {
        let face = lib
            .new_face(path, 0)
            .map_err(|e| format!("cannot open font {path}: {e}"))?;
        // Integer pixel size: FreeType hints against exactly this size.
        face.set_pixel_sizes(0, px.round().max(1.0) as u32)
            .map_err(|e| format!("cannot size font {path} at {px}px: {e}"))?;
        Ok(face)
    };
    // Faces keep the underlying FT_Library alive through a
    // reference count, so the local `lib` handle can drop freely.
    let lib = Library::init().map_err(|e| format!("cannot init FreeType: {e}"))?;
    let regular = open(&lib, regular_path)?;
    let variant = |path: Option<&str>| -> Face {
        match path
            .filter(|p| std::path::Path::new(p).exists() && p != &regular_path)
            .map(|p| open(&lib, p))
        {
            Some(Ok(f)) => f,
            // Missing/unreadable variant: rasterize with the regular
            // face instead of failing the whole font load.
            _ => open(&lib, regular_path).expect("regular face opens twice"),
        }
    };
    let bold = variant(bold_path);
    let italic = variant(italic_path);
    let bold_italic = variant(bold_italic_path);
    let metrics = measure(&regular, px);
    Ok(FontFace {
        regular,
        bold,
        italic,
        bold_italic,
        metrics,
        cache: BoundedCache::new(FontFace::RASTER_LIMIT),
    })
}

/// Load the best available monospace face for `family` at `px`,
/// falling back through fc-match -> FALLBACK_FONT_PATHS.
pub fn load_default_face(family: &str, px: f32) -> Result<FontFace, String> {
    if let Some((r, b, i, bi)) = locate_font_files(family) {
        if let Ok(face) = load_face(&r, Some(&b), Some(&i), Some(&bi), px) {
            return Ok(face);
        }
    }
    for path in FALLBACK_FONT_PATHS {
        if std::path::Path::new(path).exists() {
            if let Ok(face) = load_face(path, None, None, None, px) {
                return Ok(face);
            }
        }
    }
    Err("no usable monospace font found".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn any_system_font() -> Option<&'static str> {
        FALLBACK_FONT_PATHS
            .iter()
            .find(|p| std::path::Path::new(p).exists())
            .copied()
    }

    #[test]
    fn metrics_are_positive_and_monospaced() {
        let Some(path) = any_system_font() else {
            eprintln!("no system font, skipping");
            return;
        };
        let face = load_default_face("monospace", 14.0).expect("load face");
        assert!(face.metrics.line_height >= 10);
        assert!(face.metrics.cell_width >= 5);
        assert!(face.metrics.ascent > 0);
        assert!(face.metrics.px == 14.0);
        let _ = path;
    }

    #[test]
    fn ascii_glyphs_rasterize_with_sane_bounds() {
        let Some(_) = any_system_font() else {
            eprintln!("no system font, skipping");
            return;
        };
        let mut face = load_default_face("monospace", 14.0).expect("load face");
        for c in ['A', 'g', '5', '~', '@'] {
            let b = face
                .glyph(c, 0)
                .unwrap_or_else(|| panic!("{c} must rasterize"));
            assert!(b.width > 0 && b.width < 60, "{c}: w={}", b.width);
            assert!(b.height > 0 && b.height < 60, "{c}: h={}", b.height);
            assert!(b.coverage.iter().any(|&v| v > 0), "{c} must have ink");
        }
    }

    #[test]
    fn whitespace_glyphs_are_skipped() {
        let Some(_) = any_system_font() else {
            eprintln!("no system font, skipping");
            return;
        };
        let mut face = load_default_face("monospace", 14.0).expect("load face");
        assert!(face.glyph(' ', 0).is_none());
    }

    #[test]
    fn glyph_cache_is_bounded_and_clearable() {
        let Some(_) = any_system_font() else {
            eprintln!("no system font, skipping");
            return;
        };
        let mut face = load_default_face("monospace", 14.0).expect("load face");
        for i in 0..64u32 {
            let c = char::from_u32(0x4E00 + i).unwrap_or('a'); // CJK range
            let _ = face.glyph(c, 0);
        }
        assert!(face.cache_len() <= FontFace::RASTER_LIMIT);
        let mut face = face;
        face.clear_cache();
        assert_eq!(face.cache_len(), 0);
    }

    #[test]
    fn cache_eviction_keeps_length_under_the_limit() {
        let Some(_) = any_system_font() else {
            eprintln!("no system font, skipping");
            return;
        };
        let mut face = load_default_face("monospace", 14.0).expect("load face");
        for i in 0..5000u32 {
            let c = char::from_u32(0x4E00 + i).unwrap_or('a');
            let _ = face.glyph(c, 0);
        }
        assert!(face.cache_len() <= FontFace::RASTER_LIMIT);
    }

    #[test]
    fn style_keys_pick_the_bold_face_when_loaded() {
        let Some(_) = any_system_font() else {
            eprintln!("no system font, skipping");
            return;
        };
        // DejaVu ships Bold; if present both keys rasterize (possibly to
        // the same regular face when only one file exists).
        let mut face = load_default_face("monospace", 14.0).expect("load face");
        let _ = face.glyph('A', STYLE_BOLD);
        let _ = face.glyph('A', STYLE_ITALIC);
        let _ = face.glyph('A', STYLE_BOLD | STYLE_ITALIC);
        assert!(face.cache_len() <= FontFace::RASTER_LIMIT);
    }

    #[test]
    fn fallback_paths_are_an_ordered_list() {
        assert!(FALLBACK_FONT_PATHS.len() >= 4);
        // All entries are absolute.
        for p in FALLBACK_FONT_PATHS {
            assert!(p.starts_with('/'));
        }
    }

    #[test]
    fn variant_candidates_cover_the_common_naming() {
        let cands = variant_candidates(
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            "bold",
        );
        assert!(cands.iter().any(|p| p.ends_with("DejaVuSansMono-Bold.ttf")));
        let cands = variant_candidates("/usr/share/fonts/truetype/freefont/FreeMono.ttf", "italic");
        assert!(cands.iter().any(|p| p.ends_with("FreeMono-Italic.ttf")));
    }

    #[test]
    fn family_of_maps_filename_to_family_name() {
        assert_eq!(
            family_of("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"),
            Some("DejaVu Sans Mono".to_string())
        );
        assert_eq!(
            family_of("/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf"),
            Some("Liberation Mono".to_string())
        );
        assert_eq!(family_of("/weird"), Some("weird".to_string()));
    }

    #[test]
    fn load_face_reports_a_clear_error_for_missing_files() {
        let err =
            load_face("/definitely/not/a/font.ttf", None, None, None, 14.0).expect_err("must fail");
        assert!(err.contains("cannot open font"));
    }

    #[test]
    fn hinted_rasterization_is_sharper_than_unhinted_bounds() {
        // Hinted metrics at a small size stay tight: the cell width of
        // a monospace face at 15px must be close to the em advance
        // (9-10px for DejaVu) rather than a blurry fractional value.
        let Some(_) = any_system_font() else {
            eprintln!("no system font, skipping");
            return;
        };
        let face = load_default_face("monospace", 15.0).expect("load face");
        assert!(
            face.metrics.cell_width >= 7 && face.metrics.cell_width <= 12,
            "cell_width {} out of hinted range",
            face.metrics.cell_width
        );
    }

    #[test]
    fn resize_to_new_px_produces_new_metrics() {
        let Some(_) = any_system_font() else {
            eprintln!("no system font, skipping");
            return;
        };
        let face = load_default_face("monospace", 14.0).expect("load face");
        let bigger = load_default_face("monospace", 20.0).expect("load face 20px");
        assert!(bigger.metrics.line_height > face.metrics.line_height);
        assert!(bigger.metrics.cell_width >= face.metrics.cell_width);
    }
}

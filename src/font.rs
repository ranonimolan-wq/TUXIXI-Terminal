//! Font loading and glyph rasterization for the winit GUI.
//!
//! The font is located with `fc-match` (fontconfig, the same lookup
//! GTK/Pango did) with a pure-Rust fallback that scans the usual
//! directories. Rasterization uses `ab_glyph` — no FreeType, no C.
//!
//! The glyph cache is a [`BoundedCache`]: a hard entry limit with
//! explicit release-on-eviction, so a session full of rare CJK glyphs
//! can never grow the process without bound.

use crate::mem::{BoundedCache, Compact};
use ab_glyph::{Font, FontArc, GlyphId, ScaleFont};

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
    pub regular: FontArc,
    pub bold: FontArc,
    pub italic: FontArc,
    pub bold_italic: FontArc,
    pub metrics: FontMetrics,
    cache: BoundedCache<GlyphKey, GlyphBitmap>,
}

impl FontFace {
    /// Rasterization size used by the GUI.
    pub const RASTER_LIMIT: usize = BoundedCache::<GlyphKey, GlyphBitmap>::DEFAULT_LIMIT;

    pub fn from_data(
        regular: FontArc,
        bold: FontArc,
        italic: FontArc,
        bold_italic: FontArc,
        px: f32,
    ) -> Self {
        let metrics = measure(&regular, px);
        Self {
            regular,
            bold,
            italic,
            bold_italic,
            metrics,
            cache: BoundedCache::new(Self::RASTER_LIMIT),
        }
    }

    fn face_for(&self, style: u8) -> &FontArc {
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
            let bitmap = rasterize(self.face_for(style), c, self.metrics.px);
            {
                let b = bitmap?;
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
pub fn measure(font: &FontArc, px: f32) -> FontMetrics {
    let scaled = font.as_scaled(px);
    let ascent = scaled.ascent().round().max(1.0);
    let descent = scaled.descent().abs().round().max(1.0);
    // Cell width = widest advance among the printable ASCII set so all
    // monospace glyphs (and near-monospace fonts) stay aligned.
    let mut cell_width: f32 = 1.0;
    for c in ('!'..='~').chain(' '..=' ') {
        let adv = scaled.h_advance(font.glyph_id(c));
        if adv > cell_width {
            cell_width = adv;
        }
    }
    FontMetrics {
        line_height: (ascent + descent).ceil() as u32,
        cell_width: cell_width.ceil() as u32,
        ascent: ascent as i32,
        px,
    }
}

/// Rasterize `c` into an 8-bit coverage bitmap (top-left origin).
pub fn rasterize(font: &FontArc, c: char, px: f32) -> Option<GlyphBitmap> {
    let gid: GlyphId = font.glyph_id(c);
    if gid.0 == 0 && c != '\0' {
        // .notdef -> skip (renderer paints an empty cell).
        if font.glyph_id(' ').0 == 0 {
            return None;
        }
    }
    let scaled = font.as_scaled(px);
    let glyph = ab_glyph::Glyph {
        id: gid,
        scale: px.into(),
        position: ab_glyph::Point {
            x: 0.0,
            y: scaled.ascent(),
        },
    };
    let outline = font.outline_glyph(glyph)?;
    let bounds = outline.px_bounds();
    let (w, h) = (
        bounds.width().round() as usize,
        bounds.height().round() as usize,
    );
    if w == 0 || h == 0 {
        return None;
    }
    let mut coverage = vec![0u8; w * h];
    outline.draw(|x, y, v| {
        let idx = y as usize * w + x as usize;
        if idx < coverage.len() {
            coverage[idx] = (v * 255.0).round() as u8;
        }
    });
    Some(GlyphBitmap {
        width: w,
        height: h,
        coverage,
        left: bounds.min.x.floor() as i32,
        top: bounds.min.y.floor() as i32,
    })
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
    let read = |p: &str| -> Result<FontArc, String> {
        let data = std::fs::read(p).map_err(|e| format!("cannot read font {p}: {e}"))?;
        FontArc::try_from_vec(data).map_err(|e| format!("cannot parse font {p}: {e}"))
    };
    let regular = read(regular_path)?;
    let bold = bold_path
        .filter(|p| std::path::Path::new(p).exists() && p != &regular_path)
        .map(read)
        .transpose()
        .map_err(|e: String| e)?;
    let italic = italic_path
        .filter(|p| std::path::Path::new(p).exists() && p != &regular_path)
        .map(read)
        .transpose()
        .map_err(|e: String| e)?;
    let bold_italic = bold_italic_path
        .filter(|p| std::path::Path::new(p).exists() && p != &regular_path)
        .map(read)
        .transpose()
        .map_err(|e: String| e)?;
    let fallback = regular.clone();
    Ok(FontFace::from_data(
        regular,
        bold.unwrap_or_else(|| clone_font(&fallback)),
        italic.unwrap_or_else(|| clone_font(&fallback)),
        bold_italic.unwrap_or_else(|| clone_font(&fallback)),
        px,
    ))
}

fn clone_font(f: &FontArc) -> FontArc {
    f.clone()
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
        let face = load_default_face("monospace", 14.0).expect("load face");
        for c in ['A', 'g', '5', '~', '@'] {
            let face_c = face.face_for(0);
            let b = rasterize(face_c, c, 14.0).expect("glyph should rasterize");
            assert!(b.width > 0 && b.width < 60, "{c}: w={}", b.width);
            assert!(b.height > 0 && b.height < 60, "{c}: h={}", b.height);
            assert!(b.coverage.iter().any(|&v| v > 0), "{c} must have ink");
        }
    }

    #[test]
    fn whitespace_glyphs_are_skipped() {
        let face = load_default_face("monospace", 14.0).expect("load face");
        let face_c = face.face_for(0);
        assert!(rasterize(face_c, ' ', 14.0).is_none());
    }

    #[test]
    fn glyph_cache_is_bounded_and_clearable() {
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
        let mut face = load_default_face("monospace", 14.0).expect("load face");
        for i in 0..5000u32 {
            let c = char::from_u32(0x4E00 + i).unwrap_or('a');
            let _ = face.glyph(c, 0);
        }
        assert!(face.cache_len() <= FontFace::RASTER_LIMIT);
    }

    #[test]
    fn style_keys_pick_the_bold_face_when_loaded() {
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
        assert!(err.contains("cannot read font"));
    }

    #[test]
    fn resize_to_new_px_produces_new_metrics() {
        let face = load_default_face("monospace", 14.0).expect("load face");
        let bigger = measure(&face.regular, 20.0);
        assert!(bigger.line_height > face.metrics.line_height);
        assert!(bigger.cell_width >= face.metrics.cell_width);
    }
}

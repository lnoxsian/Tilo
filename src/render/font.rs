//! Pure Rust font discovery, loading, and rasterization using `fontdue`.
//!
//! Provides zero-C-dependency TrueType and OpenType font support for Tilo.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Parses a font specification string (e.g. `"monospace 10"`, `"pango:DejaVu Sans Mono 11"`)
/// into a font family/path string and a font size in points.
pub fn parse_font_spec(font_name: &str) -> (String, f32) {
    let trimmed = font_name.trim();
    let clean = if let Some(stripped) = trimmed.strip_prefix("pango:") {
        stripped.trim()
    } else if let Some(stripped) = trimmed.strip_prefix("core:") {
        stripped.trim()
    } else {
        trimmed
    };

    // Look for a trailing size number (e.g. "monospace 10", "DejaVu Sans Mono 11.5")
    if let Some(last_space_idx) = clean.rfind(|c: char| c.is_whitespace()) {
        let (family_part, size_part) = clean.split_at(last_space_idx);
        let trimmed_size = size_part.trim();
        if let Ok(size) = trimmed_size.parse::<f32>()
            && size > 0.0 {
                return (family_part.trim().to_string(), size);
            }
    }

    // Default to size 10.0 if not specified
    (clean.to_string(), 10.0)
}

/// Standard directories to search for font files.
const FONT_SEARCH_PATHS: &[&str] = &[
    "/usr/share/fonts/truetype",
    "/usr/share/fonts/TTF",
    "/usr/share/fonts/OTF",
    "/usr/share/fonts",
    "/usr/local/share/fonts",
];

/// Known standard monospace fonts.
const MONOSPACE_FALLBACKS: &[&str] = &[
    "LiberationMono-Regular.ttf",
    "DejaVuSansMono.ttf",
    "Hack-Regular.ttf",
    "NotoSansMono-Regular.ttf",
    "FreeMono.ttf",
];

/// Known standard sans-serif fonts.
const SANS_FALLBACKS: &[&str] = &[
    "LiberationSans-Regular.ttf",
    "DejaVuSans.ttf",
    "NotoSans-Regular.ttf",
    "FreeSans.ttf",
];

/// Known standard serif fonts.
const SERIF_FALLBACKS: &[&str] = &[
    "LiberationSerif-Regular.ttf",
    "DejaVuSerif.ttf",
    "NotoSerif-Regular.ttf",
    "FreeSerif.ttf",
];

/// Recursively searches a directory for a font file by name or substring.
fn search_dir_for_font(dir: &Path, target_sub: &str, depth: usize) -> Option<PathBuf> {
    if depth > 4 || !dir.is_dir() {
        return None;
    }

    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                let ext_lower = ext.to_lowercase();
                if (ext_lower == "ttf" || ext_lower == "otf")
                    && let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        let stem_clean = stem.to_lowercase().replace([' ', '-', '_'], "");
                        if stem_clean.contains(target_sub) {
                            return Some(path);
                        }
                    }
            } else if path.is_dir()
            && let Some(found) = search_dir_for_font(&path, target_sub, depth + 1) {
                return Some(found);
            }
    }
    None
}

/// Attempts to find a font file matching a given fallback candidate in search directories.
fn find_in_search_paths(filename: &str) -> Option<PathBuf> {
    let clean_target = filename
        .trim_end_matches(".ttf")
        .trim_end_matches(".otf")
        .to_lowercase()
        .replace([' ', '-', '_'], "");

    // 1. Check user local font directories
    if let Ok(home) = std::env::var("HOME") {
        let user_paths = [
            format!("{}/.local/share/fonts", home),
            format!("{}/.fonts", home),
        ];
        for p in &user_paths {
            let path = Path::new(p);
            if let Some(found) = search_dir_for_font(path, &clean_target, 0) {
                return Some(found);
            }
        }
    }

    // 2. Check system font directories
    for p in FONT_SEARCH_PATHS {
        let path = Path::new(p);
        if let Some(found) = search_dir_for_font(path, &clean_target, 0) {
            return Some(found);
        }
    }

    None
}

/// Resolves a font family name or pattern to an absolute file path.
pub fn resolve_font_file(family: &str) -> Option<PathBuf> {
    let trimmed = family.trim();
    if trimmed.is_empty() {
        return None;
    }

    // 1. If it is already a direct file path that exists, return it
    let direct_path = Path::new(trimmed);
    if direct_path.is_file() {
        return Some(direct_path.to_path_buf());
    }

    let lower = trimmed.to_lowercase();

    // 2. Generic aliases
    let candidates: &[&str] = match lower.as_str() {
        "monospace" | "mono" => MONOSPACE_FALLBACKS,
        "sans" | "sans-serif" | "sans_serif" => SANS_FALLBACKS,
        "serif" => SERIF_FALLBACKS,
        _ => &[],
    };

    for candidate in candidates {
        if let Some(path) = find_in_search_paths(candidate) {
            return Some(path);
        }
    }

    // 3. Search for the family name directly
    if let Some(path) = find_in_search_paths(trimmed) {
        return Some(path);
    }

    // 4. Default fallback: search for ANY monospace font
    for candidate in MONOSPACE_FALLBACKS {
        if let Some(path) = find_in_search_paths(candidate) {
            return Some(path);
        }
    }

    // 5. Ultimate fallback: find ANY .ttf or .otf in system fonts
    for p in FONT_SEARCH_PATHS {
        let path = Path::new(p);
        if let Some(found) = search_dir_for_font(path, "", 0) {
            return Some(found);
        }
    }

    None
}

/// Rasterized glyph metrics and 8-bit coverage bitmap.
pub type RasterizedGlyph = (fontdue::Metrics, Vec<u8>);
type AsciiGlyphCache = Box<[Option<RasterizedGlyph>; 128]>;

/// Pure Rust font engine backed by `fontdue`.
pub struct FontEngine {
    /// Parsed fontdue font instance.
    pub font: fontdue::Font,
    /// Render size in pixels.
    pub size_px: f32,
    /// Baseline ascent in pixels.
    pub ascent: i16,
    /// Baseline descent in pixels.
    pub descent: i16,
    /// Typical character width in pixels (based on 'M').
    pub char_width: u16,
    /// Fast direct-indexed cache for standard 7-bit ASCII characters (0..128) avoiding hash map overhead.
    ascii_cache: AsciiGlyphCache,
    /// Fallback dynamic glyph bitmap and metrics cache for Unicode characters >= 128.
    glyph_cache: RefCell<HashMap<char, RasterizedGlyph>>,
}

// `fontdue::Font` and `RefCell` are safe to transfer across threads since Renderer is strictly owned.
unsafe impl Send for FontEngine {}

impl std::fmt::Debug for FontEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontEngine")
            .field("size_px", &self.size_px)
            .field("ascent", &self.ascent)
            .field("descent", &self.descent)
            .field("char_width", &self.char_width)
            .finish()
    }
}

impl FontEngine {
    /// Loads a TrueType or OpenType font from a specification string.
    pub fn load(font_spec: &str) -> Result<Self, String> {
        let (family, size_pt) = parse_font_spec(font_spec);
        let path = resolve_font_file(&family)
            .ok_or_else(|| format!("Could not find font file for family '{}'", family))?;

        let font_bytes = std::fs::read(&path)
            .map_err(|e| format!("Failed to read font file {:?}: {}", path, e))?;

        let font = fontdue::Font::from_bytes(font_bytes, fontdue::FontSettings::default())
            .map_err(|e| format!("Failed to parse font file {:?}: {}", path, e))?;

        // Standard typography: 1 pt = 96 / 72 px = 1.3333 px at 96 DPI
        let size_px = (size_pt * (96.0 / 72.0)).max(6.0);

        let (ascent, descent) = if let Some(lm) = font.horizontal_line_metrics(size_px) {
            (
                lm.ascent.round().max(1.0) as i16,
                (-lm.descent).round().max(0.0) as i16,
            )
        } else {
            ((size_px * 0.8).round() as i16, (size_px * 0.2).round() as i16)
        };

        let char_width = font.metrics('M', size_px).advance_width.round().max(1.0) as u16;

        // Pre-warm direct ASCII cache for printable ASCII characters (32..=126) for instant statusbar rendering
        let mut ascii_cache: AsciiGlyphCache = Box::new(std::array::from_fn(|_| None));
        for c in 32u8..=126u8 {
            let ch = c as char;
            let (metrics, bitmap) = font.rasterize(ch, size_px);
            ascii_cache[c as usize] = Some((metrics, bitmap));
        }

        Ok(Self {
            font,
            size_px,
            ascent,
            descent,
            char_width,
            ascii_cache,
            glyph_cache: RefCell::new(HashMap::with_capacity(32)),
        })
    }

    /// Invokes a closure with the metrics and rasterized coverage bitmap of a character without cloning the bitmap.
    #[inline]
    pub fn with_glyph<R>(&self, ch: char, f: impl FnOnce(&fontdue::Metrics, &[u8]) -> R) -> R {
        let code = ch as u32;
        if code < 128
            && let Some((ref metrics, ref bitmap)) = self.ascii_cache[code as usize] {
                return f(metrics, bitmap);
            }
        let mut cache = self.glyph_cache.borrow_mut();
        let (metrics, bitmap) = cache
            .entry(ch)
            .or_insert_with(|| self.font.rasterize(ch, self.size_px));
        f(metrics, bitmap)
    }

    /// Returns the horizontal advance width of a character in pixels.
    #[inline]
    pub fn char_advance(&self, ch: char) -> f32 {
        let code = ch as u32;
        if code < 128
            && let Some((ref metrics, _)) = self.ascii_cache[code as usize] {
                return metrics.advance_width;
            }
        let mut cache = self.glyph_cache.borrow_mut();
        if let Some((metrics, _)) = cache.get(&ch) {
            return metrics.advance_width;
        }
        let (metrics, bitmap) = self.font.rasterize(ch, self.size_px);
        let adv = metrics.advance_width;
        cache.insert(ch, (metrics, bitmap));
        adv
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_font_spec() {
        let (fam1, sz1) = parse_font_spec("monospace 10");
        assert_eq!(fam1, "monospace");
        assert_eq!(sz1, 10.0);

        let (fam2, sz2) = parse_font_spec("pango:DejaVu Sans Mono 11.5");
        assert_eq!(fam2, "DejaVu Sans Mono");
        assert_eq!(sz2, 11.5);

        let (fam3, sz3) = parse_font_spec("JetBrainsMono Nerd Font");
        assert_eq!(fam3, "JetBrainsMono Nerd Font");
        assert_eq!(sz3, 10.0);

        let (fam4, sz4) = parse_font_spec("core:fixed");
        assert_eq!(fam4, "fixed");
        assert_eq!(sz4, 10.0);
    }

    #[test]
    fn test_resolve_font_file() {
        let resolved = resolve_font_file("monospace");
        assert!(resolved.is_some(), "Monospace font should resolve on system");
        let path = resolved.unwrap();
        assert!(path.exists());
    }

    #[test]
    fn test_font_engine_load() {
        let engine_res = FontEngine::load("monospace 10");
        assert!(engine_res.is_ok(), "FontEngine should load monospace");
        let engine = engine_res.unwrap();
        assert!(engine.ascent > 0);
        assert!(engine.char_width > 0);

        // Test with_glyph
        engine.with_glyph('A', |metrics, bitmap| {
            assert!(metrics.width > 0);
            assert!(metrics.height > 0);
            assert!(!bitmap.is_empty());
        });

        // Test char_advance
        let adv = engine.char_advance('M');
        assert!(adv > 0.0);
    }
}

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
    let mut clean = if let Some(stripped) = trimmed.strip_prefix("pango:") {
        stripped.trim()
    } else if let Some(stripped) = trimmed.strip_prefix("core:") {
        stripped.trim()
    } else {
        trimmed
    };

    // Strip enclosing quotes if present (e.g. "\"DejaVu Sans Mono\" 10")
    if (clean.starts_with('"') && clean.ends_with('"'))
        || (clean.starts_with('\'') && clean.ends_with('\''))
    {
        clean = &clean[1..clean.len() - 1].trim();
    }

    // Look for fontconfig size syntax, e.g. "monospace:size=10"
    if let Some(idx) = clean.find(":size=") {
        let (family_part, rest) = clean.split_at(idx);
        let size_str = &rest[6..];
        let end_idx = size_str.find(':').unwrap_or(size_str.len());
        if let Ok(size) = size_str[..end_idx].parse::<f32>()
            && size > 0.0
        {
            let fam = family_part.trim().trim_matches('"').trim_matches('\'');
            return (fam.to_string(), size);
        }
    }

    // Look for a trailing size number (e.g. "monospace 10", "DejaVu Sans Mono 11.5")
    if let Some(last_space_idx) = clean.rfind(|c: char| c.is_whitespace()) {
        let (family_part, size_part) = clean.split_at(last_space_idx);
        let trimmed_size = size_part.trim();
        if let Ok(size) = trimmed_size.parse::<f32>()
            && size > 0.0
        {
            let fam = family_part.trim().trim_matches('"').trim_matches('\'');
            return (fam.to_string(), size);
        }
    }

    let fam = clean.trim_matches('"').trim_matches('\'');
    // Default to size 10.0 if not specified
    (fam.to_string(), 10.0)
}

/// Standard directories to search for font files.
const FONT_SEARCH_PATHS: &[&str] = &[
    "/usr/share/fonts/truetype",
    "/usr/share/fonts/opentype",
    "/usr/share/fonts/TTF",
    "/usr/share/fonts/OTF",
    "/usr/share/fonts",
    "/usr/local/share/fonts",
];

/// Known standard monospace fonts.
const MONOSPACE_FALLBACKS: &[&str] = &[
    "LiberationMono-Regular.ttf",
    "LiberationMono.ttf",
    "DejaVuSansMono.ttf",
    "NotoSansMono-Regular.ttf",
    "Hack-Regular.ttf",
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

/// Scores a candidate font stem against a target font name and query context.
///
/// Higher scores represent better matches. Returns `None` if the candidate does not match target.
pub fn score_font_candidate(stem: &str, target: &str, query: &str) -> Option<i32> {
    let stem_lower = stem.to_lowercase();
    let target_lower = target.to_lowercase();
    let query_lower = query.to_lowercase();

    let stem_clean = stem_lower.replace([' ', '-', '_'], "");
    let target_clean = target_lower.replace([' ', '-', '_'], "");

    // Check if user/query explicitly requested italic/oblique
    let query_wants_italic = query_lower.contains("italic")
        || query_lower.contains("oblique")
        || query_lower.contains("slanted")
        || target_lower.contains("italic")
        || target_lower.contains("oblique");

    // Check if user/query explicitly requested bold
    let query_wants_bold = query_lower.contains("bold")
        || query_lower.contains("black")
        || query_lower.contains("heavy")
        || target_lower.contains("bold")
        || target_lower.contains("black");

    // Candidate attributes
    let is_italic = stem_lower.contains("italic")
        || stem_lower.contains("oblique")
        || stem_lower.contains("slanted")
        || stem_lower.ends_with("ita")
        || stem_lower.ends_with("-i")
        || stem_lower.ends_with("_i")
        || stem_lower.contains("bdita");

    let is_bold = stem_lower.contains("bold")
        || stem_lower.contains("black")
        || stem_lower.contains("heavy")
        || stem_lower.contains("semibold")
        || stem_lower.contains("demi");

    let is_light = stem_lower.contains("light")
        || stem_lower.contains("thin")
        || stem_lower.contains("extralight");

    let is_regular = stem_lower.contains("regular")
        || stem_lower.contains("book")
        || stem_lower.contains("roman")
        || stem_lower.contains("normal")
        || stem_clean == target_clean;

    // Matching check:
    // If target is empty, we match any font (for ultimate fallback).
    // Otherwise, require stem to contain target or target to contain stem.
    if !target_clean.is_empty()
        && !stem_clean.contains(&target_clean)
        && !target_clean.contains(&stem_clean)
    {
        return None;
    }

    let mut score: i32 = 1000;

    // Exact match is top priority (only if candidate matches the desired italic/regular style)
    if stem_clean == target_clean && (!query_wants_italic || is_italic) {
        score += 5000;
    }

    // Explicit regular style (only rewarded when not requesting italic)
    if !query_wants_italic && is_regular {
        score += 2000;
    }

    // Italic penalty / reward
    if query_wants_italic {
        if is_italic {
            score += 4000;
        } else {
            // User requested italic: penalize non-italic fonts
            score -= 8000;
        }
    } else if is_italic {
        // User did NOT ask for italic: heavy penalty so regular fonts always win
        score -= 10000;
    }

    // Bold penalty / reward
    if query_wants_bold {
        if is_bold {
            score += 2000;
        } else {
            score -= 1000;
        }
    } else if is_bold {
        score -= 2000;
    }

    if is_light {
        score -= 1000;
    }

    // Closeness in length (fewer unwanted extra variant words)
    let len_diff = stem_clean.len().abs_diff(target_clean.len()) as i32;
    score -= (len_diff * 10).min(500);

    Some(score)
}

/// Recursively searches a directory for the highest scoring font file matching `target`.
fn search_dir_for_best_font(
    dir: &Path,
    target: &str,
    query: &str,
    depth: usize,
) -> Option<(PathBuf, i32)> {
    if depth > 5 || !dir.is_dir() {
        return None;
    }

    let entries = std::fs::read_dir(dir).ok()?;
    let mut best: Option<(PathBuf, i32)> = None;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                let ext_lower = ext.to_lowercase();
                if (ext_lower == "ttf" || ext_lower == "otf")
                    && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                    && let Some(score) = score_font_candidate(stem, target, query)
                {
                    if best.as_ref().map_or(true, |(_, s)| score > *s) {
                        best = Some((path, score));
                    }
                }
            }
        } else if path.is_dir()
            && let Some((sub_path, sub_score)) =
                search_dir_for_best_font(&path, target, query, depth + 1)
        {
            if best.as_ref().map_or(true, |(_, s)| sub_score > *s) {
                best = Some((sub_path, sub_score));
            }
        }
    }

    best
}

/// Attempts to find a font file matching a given fallback candidate in search directories.
fn find_in_search_paths(target: &str, query: &str) -> Option<PathBuf> {
    let clean_target = target
        .trim_end_matches(".ttf")
        .trim_end_matches(".otf");

    // 1. Check user local font directories
    if let Ok(home) = std::env::var("HOME") {
        let user_paths = [
            format!("{}/.local/share/fonts", home),
            format!("{}/.fonts", home),
        ];
        for p in &user_paths {
            let path = Path::new(p);
            if let Some((found, score)) = search_dir_for_best_font(path, clean_target, query, 0) {
                if score >= 4000 {
                    return Some(found);
                }
            }
        }
    }

    // 2. Check system font directories
    let mut best_system: Option<(PathBuf, i32)> = None;
    for p in FONT_SEARCH_PATHS {
        let path = Path::new(p);
        if let Some((found, score)) = search_dir_for_best_font(path, clean_target, query, 0) {
            if score >= 4000 {
                return Some(found);
            }
            if best_system.as_ref().map_or(true, |(_, s)| score > *s) {
                best_system = Some((found, score));
            }
        }
    }

    if let Some((found, score)) = best_system {
        if score > -5000 {
            return Some(found);
        }
    }

    None
}

/// Attempts to resolve a font pattern using fontconfig's `fc-match`.
fn resolve_via_fc_match(family: &str) -> Option<PathBuf> {
    let lower = family.to_lowercase();
    let is_generic = matches!(
        lower.as_str(),
        "monospace" | "mono" | "sans" | "sans-serif" | "sans_serif" | "serif"
    );
    let wants_italic = lower.contains("italic")
        || lower.contains("oblique")
        || lower.contains("slanted");
    let wants_bold = lower.contains("bold")
        || lower.contains("black")
        || lower.contains("heavy");

    // If family already specifies :style= or :slant=, use as-is.
    // Otherwise, append style specifiers to guarantee fontconfig matches regular/non-italic.
    let pattern = if family.contains(':') {
        family.to_string()
    } else if wants_italic && wants_bold {
        format!("{family}:style=Bold Italic")
    } else if wants_italic {
        format!("{family}:style=Italic")
    } else if wants_bold {
        format!("{family}:style=Bold")
    } else {
        format!("{family}:style=Regular:slant=0")
    };

    let output = std::process::Command::new("fc-match")
        .arg("-f")
        .arg("%{file}|%{family}|%{slant}\n")
        .arg(&pattern)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().next()?.trim();
    let parts: Vec<&str> = line.split('|').collect();
    if parts.len() < 3 {
        return None;
    }

    let file_str = parts[0].trim();
    let matched_family = parts[1].trim();
    let slant_str = parts[2].trim();
    let slant: i32 = slant_str.parse().unwrap_or(0);

    let path = PathBuf::from(file_str);
    if !path.is_file() {
        return None;
    }

    // If user did not request italic, ensure slant is 0 (roman/regular, not oblique/italic)
    if !wants_italic && slant > 0 {
        return None;
    }

    // For generic aliases (e.g. "monospace"), fontconfig's choice is authoritative
    if is_generic {
        return Some(path);
    }

    // For specific family requests, verify fontconfig didn't fall back to an unrelated family
    let req_clean = lower
        .split(':')
        .next()
        .unwrap_or(&lower)
        .replace([' ', '-', '_'], "");
    let matched_clean = matched_family.to_lowercase().replace([' ', '-', '_'], "");
    if matched_clean.contains(&req_clean) || req_clean.contains(&matched_clean) {
        return Some(path);
    }

    None
}

/// Fallback font resolution using pure-Rust filesystem directory search.
fn resolve_via_pure_rust_search(family: &str) -> Option<PathBuf> {
    let lower = family.to_lowercase();
    let clean_family = lower.split(':').next().unwrap_or(&lower).trim();

    // 1. Generic aliases
    let candidates: &[&str] = match clean_family {
        "monospace" | "mono" => MONOSPACE_FALLBACKS,
        "sans" | "sans-serif" | "sans_serif" => SANS_FALLBACKS,
        "serif" => SERIF_FALLBACKS,
        _ => &[],
    };

    for candidate in candidates {
        if let Some(path) = find_in_search_paths(candidate, &lower) {
            return Some(path);
        }
    }

    // 2. Search for the family name directly
    if let Some(path) = find_in_search_paths(clean_family, &lower) {
        return Some(path);
    }

    // 3. Default fallback: search for ANY monospace font
    for candidate in MONOSPACE_FALLBACKS {
        if let Some(path) = find_in_search_paths(candidate, &lower) {
            return Some(path);
        }
    }

    // 4. Ultimate fallback: find ANY .ttf or .otf in system fonts (preferring non-italic regular)
    for p in FONT_SEARCH_PATHS {
        let path = Path::new(p);
        if let Some((found, score)) = search_dir_for_best_font(path, "", &lower, 0) {
            if score > -5000 {
                return Some(found);
            }
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

    // 2. Query system fontconfig (fc-match) if available.
    // This gives the exact, system-configured font file matching the requested family and style (regular/non-italic).
    if let Some(path) = resolve_via_fc_match(trimmed) {
        return Some(path);
    }

    // 3. Fallback: Pure-Rust directory search with style scoring.
    // If fc-match is absent or fails, search filesystem directories.
    resolve_via_pure_rust_search(trimmed)
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
        let path_str = path.to_string_lossy().to_lowercase();
        assert!(
            !path_str.contains("italic") && !path_str.contains("oblique"),
            "Resolved monospace font must not be italic or oblique, got: {:?}",
            path
        );
    }

    #[test]
    fn test_score_font_candidate_prefers_regular_over_oblique() {
        // Target: DejaVuSansMono (query: monospace)
        let regular_score = score_font_candidate("DejaVuSansMono", "DejaVuSansMono", "monospace").unwrap();
        let oblique_score = score_font_candidate("DejaVuSansMono-Oblique", "DejaVuSansMono", "monospace").unwrap();
        let bold_oblique_score = score_font_candidate("DejaVuSansMono-BoldOblique", "DejaVuSansMono", "monospace").unwrap();
        let bold_score = score_font_candidate("DejaVuSansMono-Bold", "DejaVuSansMono", "monospace").unwrap();

        assert!(regular_score > bold_score, "Regular must score higher than bold");
        assert!(bold_score > oblique_score, "Bold must score higher than oblique");
        assert!(oblique_score > bold_oblique_score, "Oblique must score higher than bold oblique");
        assert!(regular_score - oblique_score >= 10000, "Regular must have a huge lead over oblique");
    }

    #[test]
    fn test_score_font_candidate_honors_explicit_italic() {
        let regular_score = score_font_candidate("DejaVuSansMono", "DejaVuSansMono", "monospace italic").unwrap();
        let oblique_score = score_font_candidate("DejaVuSansMono-Oblique", "DejaVuSansMono", "monospace italic").unwrap();

        assert!(oblique_score > regular_score, "Oblique must score higher when italic is requested");
    }

    #[test]
    fn test_search_dir_for_best_font_avoids_oblique_regardless_of_order() {
        use std::fs::File;
        let temp_dir = std::env::temp_dir().join(format!("tilo_font_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);

        // Intentionally create Oblique first so it appears earlier in directory listing
        let oblique_file = temp_dir.join("DejaVuSansMono-Oblique.ttf");
        let bold_file = temp_dir.join("DejaVuSansMono-Bold.ttf");
        let regular_file = temp_dir.join("DejaVuSansMono.ttf");

        File::create(&oblique_file).unwrap();
        File::create(&bold_file).unwrap();
        File::create(&regular_file).unwrap();

        let (chosen_path, score) = search_dir_for_best_font(&temp_dir, "DejaVuSansMono", "monospace", 0)
            .expect("Should find matching font in directory");

        assert_eq!(chosen_path, regular_file, "Must choose regular file over oblique/bold");
        assert!(score >= 4000);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_parse_font_spec_quoted_and_colons() {
        let (fam1, sz1) = parse_font_spec("\"DejaVu Sans Mono\" 12");
        assert_eq!(fam1, "DejaVu Sans Mono");
        assert_eq!(sz1, 12.0);

        let (fam2, sz2) = parse_font_spec("monospace:size=11");
        assert_eq!(fam2, "monospace");
        assert_eq!(sz2, 11.0);
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

//! Text measurement for the SVG renderer.
//!
//! The renderer asks a [`TextMeasurer`] how wide a string is. Callers with real font metrics
//! (e.g., a browser canvas or a parsed TTF) plug them in. Everyone else gets
//! [`HeuristicMeasurer`], which sorts characters into width classes. Wrapping is a provided
//! method so every implementation shares the same greedy algorithm.

use crate::svg::model::FontSpec;

/// What one measured string occupies.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TextMetrics {
    /// Advance width of the whole string.
    pub width: f64,
    /// Height above the baseline.
    pub ascent: f64,
    /// Depth below the baseline.
    pub descent: f64,
}

impl TextMetrics {
    /// Total line height, `ascent + descent`.
    pub fn height(&self) -> f64 {
        self.ascent + self.descent
    }
}

/// Measures text for the renderer's bounding box, wrapping and label chips.
pub trait TextMeasurer {
    /// Width and vertical extent of `text` set in `font`, in scene units.
    fn measure(&self, text: &str, font: &FontSpec) -> TextMetrics;

    /// Greedy word wrap into at most `max_lines` lines of at most `max_w`, ellipsising the last
    /// line when the text does not fit.
    ///
    /// Breaks after whitespace and after `-`. Counts characters, not bytes, so multi-byte labels
    /// wrap by their measured width.
    fn wrap(&self, text: &str, font: &FontSpec, max_w: f64, max_lines: usize) -> Vec<String> {
        if text.is_empty() || max_lines == 0 {
            return Vec::new();
        }
        let tokens = tokenize(text);
        let mut lines: Vec<String> = Vec::new();
        let mut cur = String::new();
        for token in tokens {
            if cur.is_empty() {
                cur = token;
                continue;
            }
            let joined = join(&cur, &token);
            // The last allowed line keeps taking tokens and gets ellipsised, so overflow shows.
            if self.measure(&joined, font).width > max_w && lines.len() + 1 < max_lines {
                lines.push(std::mem::take(&mut cur));
                cur = token;
            } else {
                cur = joined;
            }
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
        if let Some(last) = lines.last_mut() {
            if self.measure(last, font).width > max_w {
                *last = self.ellipsize(last, font, max_w);
            }
        }
        lines
    }

    /// Trim `text` (by characters) until it plus an ellipsis fits `max_w`.
    fn ellipsize(&self, text: &str, font: &FontSpec, max_w: f64) -> String {
        const ELLIPSIS: char = '…';
        let mut chars: Vec<char> = text.chars().collect();
        while !chars.is_empty() {
            let mut candidate: String = chars.iter().collect();
            let trimmed_len = candidate.trim_end().len();
            candidate.truncate(trimmed_len);
            candidate.push(ELLIPSIS);
            if self.measure(&candidate, font).width <= max_w {
                return candidate;
            }
            chars.pop();
        }
        ELLIPSIS.to_string()
    }
}

/// Split into wrappable tokens: runs ending after whitespace (dropped) or after a `-` (kept).
fn tokenize(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            continue;
        }
        cur.push(ch);
        if ch == '-' {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Tokens that ended on a hyphen glue straight onto the next one. Everything else takes a space.
fn join(left: &str, right: &str) -> String {
    if left.ends_with('-') {
        format!("{left}{right}")
    } else {
        format!("{left} {right}")
    }
}

/// Per-character width classes, as fractions of the font size.
#[derive(Debug, Clone, PartialEq)]
pub struct WidthTable {
    /// Narrow glyphs such as `i`, `l`, `.` and `'`.
    pub narrow: f64,
    /// Lower-case letters and digits.
    pub normal: f64,
    /// Upper-case letters and wide glyphs such as `m`, `w` and `@`.
    pub wide: f64,
    /// Full-width scripts (CJK, Hangul, fullwidth forms).
    pub cjk: f64,
    /// Whitespace.
    pub space: f64,
    /// Extra width per 100 units of font weight above 400, as a fraction.
    pub weight_gain: f64,
    /// Height above the baseline, as a fraction of the font size.
    pub ascent: f64,
    /// Depth below the baseline, as a fraction of the font size.
    pub descent: f64,
}

impl Default for WidthTable {
    fn default() -> Self {
        WidthTable {
            narrow: 0.32,
            normal: 0.55,
            wide: 0.72,
            cjk: 1.0,
            space: 0.28,
            weight_gain: 0.015,
            ascent: 0.8,
            descent: 0.2,
        }
    }
}

/// The default measurer. It scales per-character width classes by the font size and needs no
/// font files.
///
/// It is within about 10% for Latin text in a humanist sans, which is enough for a bounding box
/// and a wrap decision. It gives the same result on every platform.
#[derive(Debug, Clone, Default)]
pub struct HeuristicMeasurer {
    /// The width classes and vertical proportions it measures with.
    pub table: WidthTable,
}

impl HeuristicMeasurer {
    /// A measurer with the default [`WidthTable`].
    pub fn new() -> Self {
        Self::default()
    }

    fn char_ratio(&self, ch: char) -> f64 {
        let t = &self.table;
        if ch.is_whitespace() {
            return t.space;
        }
        if is_full_width(ch) {
            return t.cjk;
        }
        match ch {
            'i' | 'j' | 'l' | 'I' | 't' | 'f' | 'r' | '.' | ',' | ':' | ';' | '\'' | '"' | '!'
            | '|' | '(' | ')' | '[' | ']' | '{' | '}' | '`' | '-' => t.narrow,
            'm' | 'w' | 'M' | 'W' | '@' | '%' | '—' => t.wide,
            c if c.is_ascii_uppercase() => t.wide,
            _ => t.normal,
        }
    }
}

/// Codepoint ranges whose glyphs are (near enough) one em wide.
fn is_full_width(ch: char) -> bool {
    matches!(ch as u32,
        0x1100..=0x115F      // Hangul Jamo
        | 0x2E80..=0x303E    // CJK radicals, Kangxi, CJK symbols
        | 0x3041..=0x33FF    // Kana, Hangul compat, CJK compat
        | 0x3400..=0x4DBF    // CJK ext A
        | 0x4E00..=0x9FFF    // CJK unified
        | 0xA000..=0xA4CF    // Yi
        | 0xAC00..=0xD7A3    // Hangul syllables
        | 0xF900..=0xFAFF    // CJK compat ideographs
        | 0xFE30..=0xFE6F    // CJK compat forms
        | 0xFF00..=0xFF60    // Fullwidth forms
        | 0xFFE0..=0xFFE6
        | 0x20000..=0x2FA1F) // CJK ext B..
}

impl TextMeasurer for HeuristicMeasurer {
    fn measure(&self, text: &str, font: &FontSpec) -> TextMetrics {
        let units: f64 = text.chars().map(|c| self.char_ratio(c)).sum();
        let weight_factor = 1.0 + ((font.weight - 400.0).max(0.0) / 100.0) * self.table.weight_gain;
        TextMetrics {
            width: units * font.size * weight_factor,
            ascent: self.table.ascent * font.size,
            descent: self.table.descent * font.size,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn font() -> FontSpec {
        FontSpec {
            size: 10.0,
            ..FontSpec::default()
        }
    }

    #[test]
    fn measure_scales_with_size_and_char_class() {
        let m = HeuristicMeasurer::new();
        let narrow = m.measure("iii", &font()).width;
        let wide = m.measure("MMM", &font()).width;
        assert!(wide > narrow * 2.0, "wide glyphs must measure wider");
        let big = FontSpec {
            size: 20.0,
            ..font()
        };
        assert!((m.measure("MMM", &big).width - 2.0 * wide).abs() < 1e-9);
    }

    #[test]
    fn wrap_splits_on_words_and_keeps_the_line_budget() {
        let m = HeuristicMeasurer::new();
        let lines = m.wrap("Confirm Customer Order Details", &font(), 60.0, 2);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].starts_with("Confirm"), "{lines:?}");
        assert!(
            lines[1].ends_with('…'),
            "overflow must ellipsize: {lines:?}"
        );
    }

    /// Wrapping measures characters, so accented and CJK labels do not wrap early.
    #[test]
    fn wrap_counts_characters_not_bytes() {
        let m = HeuristicMeasurer::new();
        let f = font();
        // 12 bytes but 6 chars, about 33 units at 0.55em.
        let lines = m.wrap("éééééé", &f, 40.0, 2);
        assert_eq!(lines, vec!["éééééé".to_string()], "{lines:?}");
        // Full-width text measures one em per character.
        assert_eq!(m.measure("日本語", &f).width, 30.0);
        let cjk = m.wrap("日本語 の ラベル", &f, 40.0, 2);
        assert_eq!(
            cjk,
            vec!["日本語".to_string(), "の ラベ…".to_string()],
            "{cjk:?}"
        );
    }

    #[test]
    fn wrap_hyphenates_without_inserting_a_space() {
        let m = HeuristicMeasurer::new();
        let lines = m.wrap("multi-part", &font(), 1000.0, 2);
        assert_eq!(lines, vec!["multi-part".to_string()]);
    }

    #[test]
    fn ellipsize_never_returns_more_than_it_was_asked_for() {
        let m = HeuristicMeasurer::new();
        let f = font();
        let out = m.ellipsize("a very long label indeed", &f, 30.0);
        assert!(out.ends_with('…'));
        assert!(m.measure(&out, &f).width <= 30.0, "{out}");
    }

    #[test]
    fn empty_text_wraps_to_nothing() {
        let m = HeuristicMeasurer::new();
        assert!(m.wrap("", &font(), 100.0, 2).is_empty());
        assert!(m.wrap("x", &font(), 100.0, 0).is_empty());
    }
}

//! Colours and font defaults a [`crate::svg::Scene`] is drawn against.
//!
//! A scene names colours only where it wants to override the theme, so the same scene can be
//! rendered light, dark or brand-coloured by swapping the [`Theme`].

use serde::{Deserialize, Serialize};

/// The six colours every drawable falls back to.
///
/// Pass the on-screen theme's resolved colours here so an export matches the screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Palette {
    /// Node fill, where a node names none. Default `#ffffff`.
    pub node_bg: String,
    /// Node border, where a node names no stroke. Default `#1f2937`.
    pub node_border: String,
    /// Text, token and numeral fill, where a run names no colour. Default `#111827`.
    pub node_text: String,
    /// Fallback for edge dots, separators, symbol swatches and a builtin marker on a gradient
    /// stroke. Default `#374151`.
    pub arc_color: String,
    /// Background behind an edge label (its halo or chip), where the label names none. Default
    /// `#ffffff`.
    pub arc_label_bg: String,
    /// Fill of a hollow edge dot. Default `#ffffff`.
    pub export_bg: String,
}

impl Default for Palette {
    fn default() -> Self {
        Palette {
            node_bg: "#ffffff".into(),
            node_border: "#1f2937".into(),
            node_text: "#111827".into(),
            arc_color: "#374151".into(),
            arc_label_bg: "#ffffff".into(),
            export_bg: "#ffffff".into(),
        }
    }
}

/// The colours a scene falls back to. The default font is the scene's own
/// ([`crate::svg::Scene::font`]).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct Theme {
    /// The colours drawables fall back to when they name none.
    pub palette: Palette,
}

impl Theme {
    /// A theme with `palette`.
    pub fn new(palette: Palette) -> Self {
        Theme { palette }
    }
}

impl From<Palette> for Theme {
    fn from(palette: Palette) -> Self {
        Theme::new(palette)
    }
}

impl From<&Palette> for Theme {
    fn from(palette: &Palette) -> Self {
        Theme::new(palette.clone())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn palette_defaults_are_the_light_theme() {
        let p = Palette::default();
        assert_eq!(p.export_bg, "#ffffff");
        assert_eq!(Theme::from(p.clone()).palette, p);
    }
}

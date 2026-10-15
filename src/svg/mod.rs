//! SVG output: a declarative [`Scene`], a pure-draw renderer, and pluggable text measurement.
//!
//! [`render`](render()) takes a [`Scene`], a [`Theme`] and a [`TextMeasurer`] and returns the
//! SVG as a `String`. It makes no layout decisions and draws everything at the coordinates the
//! caller computed. It works out two things itself, where a wrapped label breaks and how a
//! [`Panel`]'s entries stack, and asks the [`TextMeasurer`] for both.
//!
//! * [`model`]: the scene (nodes, edges, overlays) plus the escape hatches [`RawSvg`],
//!   [`SymbolDef`] and [`Attrs`] for output no enum variant covers.
//! * [`render`](render()): draw, escape and assemble. [`Scene::bounds`] gives the bounding box.
//! * [`text`]: [`TextMeasurer`] and the built-in [`HeuristicMeasurer`].
//! * [`theme`]: [`Theme`] and the six-colour [`Palette`].
//! * [`sketch()`]: a plain drawing of a graph and its layout, as a [`Scene`] or as SVG.
//!
//! ```
//! use graphlay::svg::{render, HeuristicMeasurer, Scene, SceneNode, Shape, Theme, Rect, TextRun, NodeContent};
//!
//! let scene = Scene {
//!     background: Some("#ffffff".into()),
//!     padding: 8.0,
//!     nodes: vec![SceneNode {
//!         rect: Rect::new(0.0, 0.0, 90.0, 40.0),
//!         shape: Shape::Rect { corner_radius: 6.0 },
//!         content: vec![NodeContent::Text(TextRun::new("hello"))],
//!         ..SceneNode::default()
//!     }],
//!     ..Scene::default()
//! };
//! let svg = render(&scene, &Theme::default(), &HeuristicMeasurer::new());
//! assert!(svg.contains("hello"));
//! ```

pub mod model;
pub mod render;
pub mod sketch;
pub mod text;
pub mod theme;

pub use model::{
    Attrs, Baseline, BuiltinMarker, Chip, Color, Decoration, DrawOrder, EdgeAnchor, EdgeLabel,
    ExactLayout, FitLayout, FontSpec, Glyph, GlyphCluster, GlyphLayout, GlyphRow, Gradient,
    GradientStop, Halo, LegendItem, LineCap, MarkerDef, MarkerRef, NodeContent, NumeralStyle,
    Overlay, OverlayContent, Paint, Panel, PanelSection, PanelStyle, Point, RawSvg, Rect, Ring,
    Scene, SceneEdge, SceneNode, Separator, Shape, Size, Stroke, Swatch, SymbolDef, TextAnchor,
    TextRun, TextWrap, ZOrder,
};
pub use render::{
    builtin_symbols, marker_size_for, render, SYMBOL_MAPSTO, SYMBOL_PLAY_TRIANGLE,
    SYMBOL_STOP_SQUARE, SYMBOL_SWATCH_DOT, SYMBOL_SWATCH_LOOP, SYMBOL_SWATCH_MAPSTO,
};
pub use sketch::{sketch, sketch_scene, SketchOptions};
pub use text::{HeuristicMeasurer, TextMeasurer, TextMetrics, WidthTable};
pub use theme::{Palette, Theme};

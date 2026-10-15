//! The scene model: a declarative description of one drawing.
//!
//! Everything here is plain data. A [`Scene`] carries already-laid-out geometry plus its styling.
//! [`render`](crate::svg::render()) turns it into SVG text. The only layout it does is what the
//! model asks for: text wrapping via [`TextWrap`]/[`crate::svg::TextMeasurer`], and the top-down
//! stacking of a [`Panel`].
//!
//! Three escape hatches keep the model from having to grow a case for every diagram:
//!
//! * [`RawSvg`]: markup the caller vouches for, emitted verbatim. Its `bounds` are mandatory so
//!   the scene bounding box stays correct.
//! * [`SymbolDef`] / [`NodeContent::Use`]: a fragment defined once and referenced many times.
//! * [`Attrs`]: `id`, `class`, `data-*`, `style` and free-form attributes on every drawable.

use serde::{Deserialize, Serialize};

/// An `f64` that may be infinite, as `null` in JSON.
mod unbounded {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &f64, s: S) -> Result<S::Ok, S::Error> {
        if v.is_finite() {
            s.serialize_f64(*v)
        } else {
            s.serialize_none()
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
        Ok(Option::<f64>::deserialize(d)?.unwrap_or(f64::INFINITY))
    }
}

/// A CSS colour, passed through as written (e.g., `#rrggbb`, `rgb(..)` or `currentColor`).
pub type Color = String;

/// A point in scene coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schemars", schemars(rename = "ScenePoint"))]
pub struct Point {
    /// Horizontal coordinate, growing to the right.
    pub x: f64,
    /// Vertical coordinate, growing downwards (SVG's convention).
    pub y: f64,
}

impl Point {
    /// The point `(x, y)`.
    pub const fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }
}

impl From<(f64, f64)> for Point {
    fn from((x, y): (f64, f64)) -> Self {
        Point { x, y }
    }
}

impl From<Point> for (f64, f64) {
    fn from(p: Point) -> Self {
        (p.x, p.y)
    }
}

/// A width/height pair.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schemars", schemars(rename = "SceneSize"))]
pub struct Size {
    /// Width, in scene units.
    pub w: f64,
    /// Height, in scene units.
    pub h: f64,
}

impl Size {
    /// The size `w` by `h`.
    pub const fn new(w: f64, h: f64) -> Self {
        Size { w, h }
    }
}

/// An axis-aligned rectangle, given by its top-left corner and its size.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schemars", schemars(rename = "SceneRect"))]
pub struct Rect {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub w: f64,
    /// Height.
    pub h: f64,
}

impl Rect {
    /// The rectangle with top-left corner `(x, y)` and size `w` by `h`.
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Rect { x, y, w, h }
    }

    /// The rectangle of `size` centred on `center`.
    pub fn from_center(center: Point, size: Size) -> Self {
        Rect {
            x: center.x - size.w / 2.0,
            y: center.y - size.h / 2.0,
            w: size.w,
            h: size.h,
        }
    }

    /// The right edge, `x + w`.
    pub fn max_x(&self) -> f64 {
        self.x + self.w
    }

    /// The bottom edge, `y + h`.
    pub fn max_y(&self) -> f64 {
        self.y + self.h
    }

    /// The centre point.
    pub fn center(&self) -> Point {
        Point::new(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    /// The width and height as a [`Size`].
    pub fn size(&self) -> Size {
        Size::new(self.w, self.h)
    }

    /// Grow (or, for a negative `d`, shrink) the rectangle by `d` on every side.
    pub fn inflate(&self, d: f64) -> Self {
        Rect {
            x: self.x - d,
            y: self.y - d,
            w: self.w + 2.0 * d,
            h: self.h + 2.0 * d,
        }
    }

    /// The smallest rectangle containing both.
    pub fn union(&self, other: &Rect) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Rect {
            x,
            y,
            w: self.max_x().max(other.max_x()) - x,
            h: self.max_y().max(other.max_y()) - y,
        }
    }
}

/// Escape-hatch attributes available on every drawable.
///
/// The renderer drops attribute *names* that do not match the XML name shape
/// `[A-Za-z_:][A-Za-z0-9_.:-]*` and escapes every value, so neither can break out of the
/// attribute and inject markup.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct Attrs {
    /// The element's `id`.
    pub id: Option<String>,
    /// Class names, joined with spaces into one `class` attribute.
    pub class: Vec<String>,
    /// `(name, value)` pairs, each emitted as `data-<name>="<value>"`.
    pub data: Vec<(String, String)>,
    /// Inline CSS, emitted as the `style` attribute.
    pub style: Option<String>,
    /// Free-form attributes, escaped but otherwise untouched.
    pub extra: Vec<(String, String)>,
}

impl Attrs {
    /// Whether no attribute is set. Drawables that are wrapped in a `<g>` for their attributes
    /// are wrapped only when this is false.
    pub fn is_empty(&self) -> bool {
        self.id.is_none()
            && self.class.is_empty()
            && self.data.is_empty()
            && self.style.is_none()
            && self.extra.is_empty()
    }

    /// Set [`Attrs::id`], builder style.
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Add one class name, builder style.
    pub fn class(mut self, class: impl Into<String>) -> Self {
        self.class.push(class.into());
        self
    }

    /// Add one `data-<name>` attribute, builder style. Pass `name` without the `data-` prefix.
    pub fn data(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.data.push((name.into(), value.into()));
        self
    }

    /// Set [`Attrs::style`], builder style.
    pub fn style(mut self, style: impl Into<String>) -> Self {
        self.style = Some(style.into());
        self
    }

    /// Add one free-form attribute ([`Attrs::extra`]), builder style.
    pub fn attr(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra.push((name.into(), value.into()));
        self
    }
}

/// Raw SVG the caller vouches for, emitted verbatim exactly once.
///
/// `bounds` is mandatory because the renderer cannot parse the markup. It is the only way the
/// fragment takes part in [`Scene::bounds`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct RawSvg {
    /// SVG markup, emitted unescaped. The caller is responsible for it being well-formed and safe.
    pub markup: String,
    /// The area the markup draws over, in scene coordinates, for [`Scene::bounds`].
    pub bounds: Rect,
    /// When non-empty, the markup is wrapped in a `<g>` carrying these attributes.
    #[serde(default)]
    pub attrs: Attrs,
}

impl RawSvg {
    /// Markup covering `bounds`, with no attributes.
    pub fn new(markup: impl Into<String>, bounds: Rect) -> Self {
        RawSvg {
            markup: markup.into(),
            bounds,
            attrs: Attrs::default(),
        }
    }
}

/// A reusable fragment, emitted once as `<symbol>` and drawn with `<use>`.
///
/// The markup is in the symbol's own `view_box` coordinates. A fragment that should take the
/// colour of the place it is used paints with `currentColor`, since the renderer sets `color` on
/// the `<use>` element.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SymbolDef {
    /// The id that [`NodeContent::Use`], [`Glyph::Symbol`], [`Separator::symbol`],
    /// [`Decoration::Glyph`] and [`Swatch::Symbol`] refer to it by. A scene symbol with the id of
    /// a builtin one ([`crate::svg::builtin_symbols`]) replaces it.
    pub id: String,
    /// The coordinate box the markup is written in. It is scaled to whatever box it is used at.
    pub view_box: Rect,
    /// SVG markup, emitted unescaped inside the `<symbol>`.
    pub markup: String,
    /// Attributes on the `<symbol>` element.
    #[serde(default)]
    pub attrs: Attrs,
}

impl SymbolDef {
    /// A symbol with no attributes.
    pub fn new(id: impl Into<String>, view_box: Rect, markup: impl Into<String>) -> Self {
        SymbolDef {
            id: id.into(),
            view_box,
            markup: markup.into(),
            attrs: Attrs::default(),
        }
    }
}

/// Node outline shape.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Shape {
    /// Rounded rectangle filling the node's box. The radius is clamped like CSS `border-radius`.
    Rect {
        /// Corner radius in scene units. `0` draws sharp corners. Clamped to half the shorter
        /// side.
        corner_radius: f64,
    },
    /// Ellipse inscribed in the node's box.
    Ellipse,
    /// Circle of diameter `rect.w` (the height is ignored).
    Circle,
}

impl Default for Shape {
    fn default() -> Self {
        Shape::Rect { corner_radius: 0.0 }
    }
}

/// How a stroked line ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum LineCap {
    /// The stroke stops flat at the endpoint (SVG's default).
    #[default]
    Butt,
    /// A half-disc of the stroke width past the endpoint.
    Round,
    /// A half-square of the stroke width past the endpoint.
    Square,
}

impl LineCap {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            LineCap::Butt => "butt",
            LineCap::Round => "round",
            LineCap::Square => "square",
        }
    }
}

/// One stop of a [`Gradient`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct GradientStop {
    /// Fraction (0..1) along the gradient axis.
    pub offset: f64,
    /// The colour at that point.
    pub color: Color,
}

/// A linear gradient in scene coordinates (`gradientUnits="userSpaceOnUse"`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Gradient {
    /// Where offset `0` lies.
    pub from: Point,
    /// Where offset `1` lies.
    pub to: Point,
    /// The colour stops, in offset order.
    pub stops: Vec<GradientStop>,
}

/// What a stroke or fill is painted with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum Paint {
    /// A flat colour.
    Color(Color),
    /// The renderer emits one `<linearGradient>` per distinct gradient and references it.
    Gradient(Gradient),
}

impl Paint {
    /// A flat colour paint.
    pub fn color(c: impl Into<Color>) -> Self {
        Paint::Color(c.into())
    }
}

impl Default for Paint {
    fn default() -> Self {
        Paint::Color("currentColor".into())
    }
}

/// A stroke: what to paint with, how wide, dashed how, capped how.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct Stroke {
    /// What the stroke is painted with. Defaults to `currentColor`.
    pub paint: Paint,
    /// Stroke width in scene units. Defaults to `1`.
    pub width: f64,
    /// SVG `stroke-dasharray`, e.g., `"4 2"`. `None` draws a solid line.
    pub dash: Option<String>,
    /// How the open ends of the stroke are drawn.
    pub linecap: LineCap,
    /// `stroke-opacity` in `0..=1`. `None` leaves it out (fully opaque).
    pub opacity: Option<f64>,
}

impl Default for Stroke {
    fn default() -> Self {
        Stroke {
            paint: Paint::default(),
            width: 1.0,
            dash: None,
            linecap: LineCap::Butt,
            opacity: None,
        }
    }
}

impl Stroke {
    /// A solid stroke of `width` in a flat colour.
    pub fn new(color: impl Into<Color>, width: f64) -> Self {
        Stroke {
            paint: Paint::Color(color.into()),
            width,
            ..Stroke::default()
        }
    }

    /// This stroke with [`Stroke::dash`] set, builder style.
    pub fn dashed(mut self, dash: impl Into<String>) -> Self {
        self.dash = Some(dash.into());
        self
    }
}

/// The font a text run is measured and drawn with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct FontSpec {
    /// CSS `font-family` list. Defaults to `Inter, system-ui, -apple-system, sans-serif`.
    pub family: String,
    /// Font size in scene units. Defaults to `12.5`.
    pub size: f64,
    /// CSS numeric font weight (`400` regular, `700` bold). Defaults to `400`.
    pub weight: f64,
    /// Italic style, passed to the [`crate::svg::TextMeasurer`] and drawn with
    /// `font-style="italic"`.
    pub italic: bool,
}

impl Default for FontSpec {
    fn default() -> Self {
        FontSpec {
            family: "Inter, system-ui, -apple-system, sans-serif".into(),
            size: 12.5,
            weight: 400.0,
            italic: false,
        }
    }
}

/// Horizontal alignment of a text run around its anchor point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum TextAnchor {
    /// The text starts at the anchor.
    Start,
    /// The text is centred on the anchor.
    #[default]
    Middle,
    /// The text ends at the anchor.
    End,
}

impl TextAnchor {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            TextAnchor::Start => "start",
            TextAnchor::Middle => "middle",
            TextAnchor::End => "end",
        }
    }
}

/// Vertical alignment of a text run around its anchor point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Baseline {
    /// The anchor is on the text's own baseline (`<text y=..>` as SVG defines it).
    Alphabetic,
    /// The anchor is the vertical middle of the glyphs (`dominant-baseline="central"`).
    #[default]
    Central,
}

/// A background-coloured stroke drawn under the glyphs (`paint-order="stroke"`), so text stays
/// legible over an arc without a hard chip around it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Halo {
    /// Colour of the halo, usually the background the text sits on.
    pub color: Color,
    /// Stroke width of the halo. It reaches half of this beyond each glyph's outline.
    pub width: f64,
}

/// Opt-in word wrapping. The renderer asks the [`crate::svg::TextMeasurer`] where to split.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TextWrap {
    /// Widest a line may be, in scene units, as the measurer reports it.
    pub max_width: f64,
    /// Most lines the text is split into. The last one is ellipsised when the text does not fit.
    /// `0` draws nothing.
    pub max_lines: usize,
    /// Baseline-to-baseline distance between wrapped lines.
    pub line_height: f64,
}

/// One run of text: a single line, or several when [`TextRun::wrap`] is set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct TextRun {
    /// The text, escaped on output.
    pub text: String,
    /// Horizontal offset from the anchor the run belongs to (a node's centre, an edge's label
    /// anchor, a panel's origin).
    pub dx: f64,
    /// Vertical offset from that same anchor.
    pub dy: f64,
    /// Font size in scene units. `None` takes the scene's [`Scene::font`] size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    /// CSS numeric font weight. `None` takes the scene's [`Scene::font`] weight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    /// Font family. Falls back to the scene's [`Scene::font`].
    pub family: Option<String>,
    /// Fill colour. Falls back to the theme's text colour.
    pub color: Option<Color>,
    /// Horizontal alignment around the anchor. Defaults to [`TextAnchor::Middle`].
    pub anchor: TextAnchor,
    /// Vertical alignment around the anchor. Defaults to [`Baseline::Central`].
    pub baseline: Baseline,
    /// Word wrapping. `None` draws [`TextRun::text`] on one line as given. Wrapped lines are
    /// stacked [`TextWrap::line_height`] apart and centred as a block on the anchor.
    pub wrap: Option<TextWrap>,
    /// A background-coloured outline under the glyphs. `None` draws the glyphs bare.
    pub halo: Option<Halo>,
    /// Attributes on each emitted `<text>` element.
    pub attrs: Attrs,
}

impl Default for TextRun {
    fn default() -> Self {
        TextRun {
            text: String::new(),
            dx: 0.0,
            dy: 0.0,
            size: None,
            weight: None,
            family: None,
            color: None,
            anchor: TextAnchor::Middle,
            baseline: Baseline::Central,
            wrap: None,
            halo: None,
            attrs: Attrs::default(),
        }
    }
}

impl TextRun {
    /// A run of `text` with every other field at its default. It is centred on its anchor, set in
    /// the scene font and not wrapped.
    pub fn new(text: impl Into<String>) -> Self {
        TextRun {
            text: text.into(),
            ..TextRun::default()
        }
    }

    /// This run offset by `(dx, dy)` from its anchor, builder style.
    pub fn at(mut self, dx: f64, dy: f64) -> Self {
        self.dx = dx;
        self.dy = dy;
        self
    }

    /// This run at font size `size`, builder style.
    pub fn size(mut self, size: f64) -> Self {
        self.size = Some(size);
        self
    }

    /// This run at font weight `weight`, builder style.
    pub fn weight(mut self, weight: f64) -> Self {
        self.weight = Some(weight);
        self
    }

    /// The [`FontSpec`] this run is measured with, given the scene default.
    pub fn font(&self, default: &FontSpec) -> FontSpec {
        FontSpec {
            family: self
                .family
                .clone()
                .unwrap_or_else(|| default.family.clone()),
            size: self.size.unwrap_or(default.size),
            weight: self.weight.unwrap_or(default.weight),
            italic: default.italic,
        }
    }
}

/// A concentric ring drawn just inside a node's border. `inset` is the distance from the border
/// to the ring's outer edge. Rings follow the node's own shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Ring {
    /// Stroke colour.
    pub color: Color,
    /// Stroke width.
    pub width: f64,
    /// Distance from the node's border inwards to the ring's outer edge. A ring with no room left
    /// inside the node is not drawn.
    pub inset: f64,
}

/// The shape one counted glyph takes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Glyph {
    /// Filled circle of the cluster's radius.
    Dot,
    /// Rounded square inscribed in the cluster's diameter.
    Square {
        /// Side length as a fraction of the diameter.
        scale: f64,
        /// Corner radius of the square.
        corner_radius: f64,
        /// Opacity of the whole glyph, `0..=1`.
        opacity: f64,
    },
    /// A [`SymbolDef`] drawn in the cluster's box.
    Symbol {
        /// The symbol's [`SymbolDef::id`].
        id: String,
    },
}

/// A separator glyph drawn in the gap before a cluster, binding it to the cluster before it.
/// The bound pair is never split across rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Separator {
    /// Id of a [`SymbolDef`]. See [`crate::svg::builtin_symbols`] for the builtin ones.
    pub symbol: String,
    /// Width of the box the symbol is drawn in.
    pub width: f64,
    /// Height of that box. It is centred on the row.
    pub height: f64,
    /// Space between the previous cluster and the separator. Together with
    /// [`Separator::width`] and [`Separator::gap_after`] it takes the place of
    /// [`ExactLayout::cluster_gap`] before this cluster.
    pub gap_before: f64,
    /// Space between the separator and this cluster.
    pub gap_after: f64,
    /// Tint for the symbol. Falls back to the theme's arc colour.
    pub color: Option<Color>,
}

/// One run of `count` identical glyphs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct GlyphCluster {
    /// The shape of every glyph in the cluster.
    pub glyph: Glyph,
    /// How many glyphs. Defaults to `1`.
    pub count: u64,
    /// Glyph fill (the tint, for [`Glyph::Symbol`]). Falls back to the theme's text colour.
    pub color: Option<Color>,
    /// Glyph radius. Ignored under [`GlyphLayout::Fit`], which sizes glyphs to the row.
    pub radius: f64,
    /// Centre-to-centre spacing inside the cluster. Below `2 * radius` the glyphs overlap into a
    /// tight cluster. `None` uses `2 * radius + 2`.
    pub step: Option<f64>,
    /// Outline stroked around each glyph.
    pub outline: Option<Stroke>,
    /// Glyph drawn in the gap before this cluster, tying it to the previous one. Only
    /// [`GlyphLayout::Exact`] draws it.
    pub separator: Option<Separator>,
    /// Attributes on each glyph's element.
    pub attrs: Attrs,
}

impl Default for GlyphCluster {
    fn default() -> Self {
        GlyphCluster {
            glyph: Glyph::Dot,
            count: 1,
            color: None,
            radius: 4.33,
            step: None,
            outline: None,
            separator: None,
            attrs: Attrs::default(),
        }
    }
}

impl GlyphCluster {
    /// Centre-to-centre spacing actually used.
    pub fn step(&self) -> f64 {
        self.step.unwrap_or(self.radius * 2.0 + 2.0)
    }

    /// Width of the cluster at its declared radius.
    pub fn width(&self) -> f64 {
        2.0 * self.radius + self.step() * self.count.saturating_sub(1) as f64
    }
}

/// Style of the numeral a [`GlyphLayout::Fit`] row collapses to when it cannot fit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct NumeralStyle {
    /// Font size of the numeral.
    pub size: f64,
    /// CSS numeric font weight of the numeral.
    pub weight: f64,
    /// Fill colour. Falls back to the theme's text colour.
    pub color: Option<Color>,
}

/// Fixed-size clusters packed left to right, wrapping into further rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct ExactLayout {
    /// Rows wrap once they would grow past this width. A single cluster wider than this still
    /// gets a row of its own. Unbounded by default, `null` in JSON.
    #[serde(with = "unbounded")]
    #[cfg_attr(feature = "schemars", schemars(with = "Option<f64>"))]
    pub wrap_width: f64,
    /// Space between two neighbouring clusters (unless the second has a [`Separator`]).
    pub cluster_gap: f64,
    /// Space between two rows, on top of the largest glyph diameter.
    pub row_gap: f64,
    /// When several rows would run past this y, they are packed tighter (down to touching)
    /// instead of overflowing, so no badge is dropped.
    pub max_bottom: Option<f64>,
}

impl Default for ExactLayout {
    fn default() -> Self {
        ExactLayout {
            wrap_width: f64::INFINITY,
            cluster_gap: 6.0,
            row_gap: 3.0,
            max_bottom: None,
        }
    }
}

/// One row scaled to fit `max_width`, collapsing to a numeral when even the smallest glyphs
/// would not fit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct FitLayout {
    /// Width the row is scaled to fit. Unbounded by default, `null` in JSON.
    #[serde(with = "unbounded")]
    #[cfg_attr(feature = "schemars", schemars(with = "Option<f64>"))]
    pub max_width: f64,
    /// Edge-to-edge gap between glyphs.
    pub gap: f64,
    /// Smallest glyph diameter. A row that would need smaller glyphs collapses to the numeral.
    pub min_diameter: f64,
    /// Largest glyph diameter, however much room the row has.
    pub max_diameter: f64,
    /// `None` draws the glyphs at `min_diameter` and lets the row overflow instead of collapsing.
    pub numeral: Option<NumeralStyle>,
}

impl Default for FitLayout {
    fn default() -> Self {
        FitLayout {
            max_width: f64::INFINITY,
            gap: 2.0,
            min_diameter: 4.0,
            max_diameter: 11.0,
            numeral: None,
        }
    }
}

/// How a [`GlyphRow`] arranges its clusters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GlyphLayout {
    /// Glyphs at their clusters' own radius and step, wrapped into rows. The default.
    Exact(ExactLayout),
    /// Every glyph at one size chosen to fit the width. The clusters' radius and step are
    /// ignored.
    Fit(FitLayout),
}

impl Default for GlyphLayout {
    fn default() -> Self {
        GlyphLayout::Exact(ExactLayout::default())
    }
}

/// A counted-glyph row drawn inside a node: multiplicities, cardinalities, tokens, capacity
/// markers. Clusters run left to right, centred on the row's anchor.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct GlyphRow {
    /// The clusters, left to right.
    pub clusters: Vec<GlyphCluster>,
    /// Horizontal offset of the row's centre from the node centre.
    pub dx: f64,
    /// Vertical offset of the row's first line from the node centre.
    pub dy: f64,
    /// How the clusters are sized and arranged.
    pub layout: GlyphLayout,
    /// When non-empty, the row is wrapped in a `<g>` carrying these attributes.
    pub attrs: Attrs,
}

/// What is drawn inside a node, in list order, over its fill and border.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeContent {
    /// Text anchored at the node centre.
    Text(TextRun),
    /// A counted-glyph row.
    Glyphs(GlyphRow),
    /// Concentric rings inside the node's border, outermost first.
    Rings {
        /// The rings.
        rings: Vec<Ring>,
    },
    /// A [`SymbolDef`] drawn in a box of `size` centred on `at` (relative to the node centre).
    Use {
        /// The symbol's [`SymbolDef::id`].
        symbol: String,
        /// Centre of the symbol's box, relative to the node centre.
        at: Point,
        /// Size of the symbol's box.
        size: Size,
        /// Tint, set as `color` on the `<use>` so `currentColor` in the symbol picks it up.
        /// `None` inherits.
        #[serde(default)]
        color: Option<Color>,
        /// Attributes on the `<use>` element.
        #[serde(default)]
        attrs: Attrs,
    },
    /// Caller markup, in scene coordinates (not relative to the node).
    Raw(RawSvg),
}

/// One node: a shape at a position, plus whatever is drawn inside it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct SceneNode {
    /// The node's box in scene coordinates. The content is anchored at its centre.
    pub rect: Rect,
    /// Outline drawn in [`SceneNode::rect`].
    pub shape: Shape,
    /// Fill colour. Falls back to the theme's node background.
    pub fill: Option<Color>,
    /// Border stroke. `None` draws a 1-unit border in the theme's node border colour.
    pub stroke: Option<Stroke>,
    /// What is drawn inside, in list order, over the fill and border.
    pub content: Vec<NodeContent>,
    /// When non-empty, the node is wrapped in a `<g>` carrying these attributes.
    pub attrs: Attrs,
}

impl Default for SceneNode {
    fn default() -> Self {
        SceneNode {
            rect: Rect::new(0.0, 0.0, 1.0, 1.0),
            shape: Shape::default(),
            fill: None,
            stroke: None,
            content: Vec::new(),
            attrs: Attrs::default(),
        }
    }
}

impl SceneNode {
    /// Half-width and half-height of the drawn outline (a circle uses `rect.w` for both).
    pub fn half_extent(&self) -> (f64, f64) {
        match self.shape {
            Shape::Circle => (self.rect.w / 2.0, self.rect.w / 2.0),
            _ => (self.rect.w / 2.0, self.rect.h / 2.0),
        }
    }

    /// The node's drawn outline box.
    pub fn outline_rect(&self) -> Rect {
        let (hx, hy) = self.half_extent();
        let c = self.rect.center();
        Rect::new(c.x - hx, c.y - hy, 2.0 * hx, 2.0 * hy)
    }
}

/// The marker glyphs the renderer can draw without a user definition.
///
/// `Arrow`, `ArrowCentered`, `Ball` and `Bar` are the basic shapes. The four composites combine
/// them and could equally be written as a user [`MarkerDef`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum BuiltinMarker {
    /// Arrowhead anchored at its back, so the tip lands on the path's endpoint.
    Arrow,
    /// Arrowhead anchored at its centre, half-embedded past the endpoint.
    ArrowCentered,
    /// Filled circle centred on the endpoint.
    Ball,
    /// Perpendicular bar centred on the endpoint.
    Bar,
    /// Arrowhead whose tip sits at a ball centred on the endpoint.
    ArrowBall,
    /// `ArrowCentered` with a perpendicular bar behind the tail.
    ArrowBar,
    /// Ball on the endpoint plus an arrowhead pointing back along the path.
    BallArrow,
    /// `BallArrow` with a bar behind the arrow's tail.
    BallBarArrow,
}

/// Which marker an edge end uses.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MarkerRef {
    /// No marker.
    #[default]
    None,
    /// One of the renderer's own markers, emitted once per distinct marker, colour and size.
    Builtin {
        /// Which marker.
        marker: BuiltinMarker,
        /// Marker fill. Falls back to the edge's own stroke colour.
        #[serde(default)]
        color: Option<Color>,
        /// Side of the marker's 12-unit box in scene units. `None` scales with the stroke width.
        #[serde(default)]
        size: Option<f64>,
    },
    /// A [`MarkerDef`] by id.
    Custom {
        /// The [`MarkerDef::id`].
        id: String,
    },
}

impl MarkerRef {
    /// A builtin marker in the edge's own colour, sized for its stroke width
    /// ([`crate::svg::marker_size_for`]).
    pub fn builtin(marker: BuiltinMarker) -> Self {
        MarkerRef::Builtin {
            marker,
            color: None,
            size: None,
        }
    }

    /// Whether this is [`MarkerRef::None`].
    pub fn is_none(&self) -> bool {
        matches!(self, MarkerRef::None)
    }
}

/// A user-defined `<marker>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct MarkerDef {
    /// The id [`MarkerRef::Custom`] refers to it by. A reference to an id that no marker has
    /// draws no marker.
    pub id: String,
    /// Markup in `view_box` coordinates, emitted verbatim inside the `<marker>`.
    pub markup: String,
    /// The coordinate box the markup is written in. Defaults to `0 0 12 12`.
    pub view_box: Rect,
    /// The point of the marker that lands on the path's endpoint, in `view_box` coordinates.
    pub ref_point: Point,
    /// `markerWidth`/`markerHeight` in scene units.
    pub size: Size,
    /// Rotate the marker to follow the path's direction at its end (`orient="auto"`). `false`
    /// draws it unrotated. Defaults to `true`.
    pub orient_auto: bool,
    /// How far the path end is pulled back so the marker's visible tip lands on the endpoint, in
    /// scene units. 0 leaves the path alone.
    pub tip_gap: f64,
    /// Attributes on the `<marker>` element.
    pub attrs: Attrs,
}

impl Default for MarkerDef {
    fn default() -> Self {
        MarkerDef {
            id: String::new(),
            markup: String::new(),
            view_box: Rect::new(0.0, 0.0, 12.0, 12.0),
            ref_point: Point::new(6.0, 6.0),
            size: Size::new(12.0, 12.0),
            orient_auto: true,
            tip_gap: 0.0,
            attrs: Attrs::default(),
        }
    }
}

/// Where along an edge something is anchored.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum EdgeAnchor {
    /// A fraction (0..1) of the polyline's length.
    Fraction(f64),
    /// An explicit point in scene coordinates.
    At(Point),
}

impl Default for EdgeAnchor {
    fn default() -> Self {
        EdgeAnchor::Fraction(0.5)
    }
}

/// A filled box drawn behind an edge label.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct Chip {
    /// Fill colour. Defaults to white.
    pub fill: Color,
    /// Space between the measured text box and the chip's border, on every side.
    pub padding: f64,
    /// Corner radius of the chip.
    pub corner_radius: f64,
    /// Border stroke. `None` draws no border.
    pub stroke: Option<Stroke>,
}

impl Default for Chip {
    fn default() -> Self {
        Chip {
            fill: "#ffffff".into(),
            padding: 2.0,
            corner_radius: 3.0,
            stroke: None,
        }
    }
}

/// A text label riding on an edge.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct EdgeLabel {
    /// The text, anchored at [`EdgeLabel::at`] and then offset by its own `dx`/`dy`.
    pub text: TextRun,
    /// Where along the edge the label is anchored. Defaults to the middle of the polyline.
    pub at: EdgeAnchor,
    /// A background chip. For a glyph-hugging outline instead, set [`TextRun::halo`].
    pub chip: Option<Chip>,
    /// When non-empty, the chip and text are wrapped in a `<g>` carrying these attributes.
    pub attrs: Attrs,
}

/// Something drawn along an edge. `Band`s are painted under the edge's own path, in list order.
/// Everything else is painted over it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Decoration {
    /// A circle centred on the anchor.
    Dot {
        /// Where along the edge.
        at: EdgeAnchor,
        /// Circle radius.
        radius: f64,
        /// Fill colour. Falls back to the theme's arc colour.
        #[serde(default)]
        fill: Option<Color>,
        /// Outline. `None` draws none.
        #[serde(default)]
        stroke: Option<Stroke>,
        /// Attributes on the `<circle>`.
        #[serde(default)]
        attrs: Attrs,
    },
    /// A bar perpendicular to the path.
    Tick {
        /// Where along the edge. The bar is perpendicular to the path's direction there.
        at: EdgeAnchor,
        /// Length of the bar, centred on the anchor.
        length: f64,
        /// How the bar is stroked.
        stroke: Stroke,
        /// Attributes on the `<line>`.
        #[serde(default)]
        attrs: Attrs,
    },
    /// A [`SymbolDef`] centred on the anchor.
    Glyph {
        /// Where along the edge.
        at: EdgeAnchor,
        /// The symbol's [`SymbolDef::id`].
        symbol: String,
        /// Size of the box the symbol is drawn in. The symbol is not rotated with the path.
        size: Size,
        /// Tint, set as `color` on the `<use>`. `None` inherits.
        #[serde(default)]
        color: Option<Color>,
        /// Attributes on the `<use>`.
        #[serde(default)]
        attrs: Attrs,
    },
    /// A lane riding beside (or under) the edge, traced from its own already-routed points.
    Band {
        /// The band's own polyline, in scene coordinates. Fewer than two points draws nothing.
        points: Vec<Point>,
        /// How the band is stroked.
        stroke: Stroke,
        /// Radius its interior joins are rounded with. `0` draws straight segments.
        #[serde(default)]
        corner_radius: f64,
        /// Attributes on the band's `<path>`.
        #[serde(default)]
        attrs: Attrs,
    },
}

/// One edge: an already-routed polyline plus its styling, markers, labels and decorations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct SceneEdge {
    /// The polyline in scene coordinates, source first. Fewer than two points draws nothing.
    pub points: Vec<Point>,
    /// How the path is stroked. A builtin marker without a colour of its own takes the stroke's
    /// colour, or the theme's arc colour when the stroke is a gradient.
    pub stroke: Stroke,
    /// Radius the interior joins are rounded with. 0 draws straight segments.
    pub corner_radius: f64,
    /// Radius of the semicircle a horizontal run of this edge draws where it crosses another
    /// edge's vertical run. 0 draws crossings flat. Only crossings well inside both runs hop, so
    /// a T junction or a crossing near a corner stays flat.
    pub hop_radius: f64,
    /// Ramer-Douglas-Peucker tolerance applied to the *drawn* path only (0 = draw as given).
    /// Labels and decorations still anchor on the points as passed in.
    pub simplify: f64,
    /// Marker at the first point. The drawn path is pulled back by the marker's tip gap so the
    /// tip lands on the endpoint. See [`MarkerDef::tip_gap`]. Builtin markers compute their own.
    pub marker_start: MarkerRef,
    /// Marker at the last point, with the same pull-back.
    pub marker_end: MarkerRef,
    /// Text labels, drawn over the path and its decorations.
    pub labels: Vec<EdgeLabel>,
    /// Dots, ticks, glyphs and bands along the edge. See [`Decoration`] for the paint order.
    pub decorations: Vec<Decoration>,
    /// When non-empty, the edge is wrapped in a `<g>` carrying these attributes.
    pub attrs: Attrs,
}

impl Default for SceneEdge {
    fn default() -> Self {
        SceneEdge {
            points: Vec::new(),
            stroke: Stroke::default(),
            corner_radius: 0.0,
            hop_radius: 0.0,
            simplify: 0.0,
            marker_start: MarkerRef::None,
            marker_end: MarkerRef::None,
            labels: Vec::new(),
            decorations: Vec::new(),
            attrs: Attrs::default(),
        }
    }
}

/// The swatch drawn left of a [`LegendItem`]'s text.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Swatch {
    /// No swatch. The text starts at the item's text column anyway.
    None,
    /// A plain rounded square in this colour.
    Color {
        /// The colour.
        color: Color,
    },
    /// A [`SymbolDef`], drawn in a box of `size` and tinted with `color` via `currentColor`.
    Symbol {
        /// The symbol's [`SymbolDef::id`].
        id: String,
        /// Size of the symbol's box.
        size: Size,
        /// Tint. Falls back to the theme's arc colour.
        #[serde(default)]
        color: Option<Color>,
    },
    /// Caller markup, in scene coordinates.
    Raw(RawSvg),
}

/// One labelled entry of a [`Panel`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct LegendItem {
    /// What is drawn left of the text.
    pub swatch: Swatch,
    /// The entry's text.
    pub text: String,
    /// Horizontal space reserved for the swatch before the text. `None` uses
    /// [`PanelStyle::swatch_column`].
    pub swatch_advance: Option<f64>,
    /// When non-empty, the entry is wrapped in a `<g>` carrying these attributes.
    pub attrs: Attrs,
}

impl Default for LegendItem {
    fn default() -> Self {
        LegendItem {
            swatch: Swatch::None,
            text: String::new(),
            swatch_advance: None,
            attrs: Attrs::default(),
        }
    }
}

/// A titled group of legend entries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct PanelSection {
    /// Heading line drawn above the entries. `None` draws none.
    pub title: Option<String>,
    /// The entries, top to bottom.
    pub items: Vec<LegendItem>,
    /// When non-empty, the section is wrapped in a `<g>` carrying these attributes.
    pub attrs: Attrs,
}

/// Sizes and colours a [`Panel`] stacks with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct PanelStyle {
    /// Font size of a section title.
    pub title_size: f64,
    /// CSS numeric font weight of a section title.
    pub title_weight: f64,
    /// Font size of an entry. Entries are always drawn at weight 400.
    pub item_size: f64,
    /// Baseline-to-baseline distance between entries (and after a title).
    pub line_height: f64,
    /// Extra space after a section.
    pub section_gap: f64,
    /// Default horizontal space reserved for a swatch.
    pub swatch_column: f64,
    /// Height of the swatch box. It is centred on the entry's text.
    pub swatch_height: f64,
    /// Offset of the swatch box's centre from the entry's baseline (negative = above it).
    pub swatch_dy: f64,
    /// Corner radius of a [`Swatch::Color`] square.
    pub swatch_corner_radius: f64,
    /// Colour of titles and entry text. Falls back to the theme's text colour.
    pub text_color: Option<Color>,
}

impl Default for PanelStyle {
    fn default() -> Self {
        PanelStyle {
            title_size: 11.0,
            title_weight: 600.0,
            item_size: 11.0,
            line_height: 16.0,
            section_gap: 6.0,
            swatch_column: 20.0,
            swatch_height: 10.0,
            swatch_dy: -3.0,
            swatch_corner_radius: 2.0,
            text_color: None,
        }
    }
}

/// A block of legend entries, stacked top-down from `origin`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct Panel {
    /// Top-left of the panel. The first baseline sits at `origin.y`.
    pub origin: Point,
    /// The sections, top to bottom.
    pub sections: Vec<PanelSection>,
    /// Sizes and colours.
    pub style: PanelStyle,
    /// When non-empty, the panel is wrapped in a `<g>` carrying these attributes.
    pub attrs: Attrs,
}

/// Whether an overlay is drawn under or over the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ZOrder {
    /// Under the nodes and edges, over the background.
    Under,
    /// Over the nodes and edges.
    #[default]
    Over,
}

/// What an [`Overlay`] draws.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OverlayContent {
    /// Caller markup.
    Raw(RawSvg),
    /// A legend panel, stacked by the renderer.
    Panel(Panel),
}

/// A fragment or panel drawn outside the node/edge stack.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Overlay {
    /// Whether the overlay is drawn under or over the graph.
    #[serde(default)]
    pub z: ZOrder,
    /// What is drawn.
    pub content: OverlayContent,
    /// When non-empty, the overlay is wrapped in a `<g>` carrying these attributes.
    #[serde(default)]
    pub attrs: Attrs,
}

impl Overlay {
    /// A panel drawn over the graph.
    pub fn panel(panel: Panel) -> Self {
        Overlay {
            z: ZOrder::Over,
            content: OverlayContent::Panel(panel),
            attrs: Attrs::default(),
        }
    }

    /// Raw markup drawn over the graph.
    pub fn raw(raw: RawSvg) -> Self {
        Overlay {
            z: ZOrder::Over,
            content: OverlayContent::Raw(raw),
            attrs: Attrs::default(),
        }
    }
}

/// Whether edges are painted before or after nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum DrawOrder {
    /// Edges first, so the nodes cover any part of an edge that reaches into them.
    #[default]
    EdgesUnderNodes,
    /// Nodes first, so markers centred on a node border sit on top of it.
    EdgesOverNodes,
}

/// One complete drawing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct Scene {
    /// Colour of a rectangle filling the whole view box, padding included. `None` leaves the
    /// background transparent.
    pub background: Option<Color>,
    /// Space kept around the scene's bounding box.
    pub padding: f64,
    /// User `<defs>` content, emitted verbatim.
    pub defs: Vec<RawSvg>,
    /// Reusable fragments, emitted whether or not anything uses them.
    pub symbols: Vec<SymbolDef>,
    /// Custom markers, for [`MarkerRef::Custom`].
    pub markers: Vec<MarkerDef>,
    /// The nodes, painted in list order.
    pub nodes: Vec<SceneNode>,
    /// The edges, painted in list order.
    pub edges: Vec<SceneEdge>,
    /// Fragments and panels drawn under or over the graph.
    pub overlays: Vec<Overlay>,
    /// Whether edges are painted under or over the nodes.
    pub order: DrawOrder,
    /// Attributes for the root `<svg>` element.
    pub attrs: Attrs,
    /// The default font of every text run. Its family is set on the root `<svg>`, and a run that
    /// names no family, size or weight of its own takes this one's. The italic flag applies to
    /// every run.
    pub font: FontSpec,
}

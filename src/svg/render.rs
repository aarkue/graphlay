//! Pure draw: turn a [`Scene`] into SVG text.
//!
//! Nothing here decides where anything goes, with two exceptions that the model declares and the
//! [`TextMeasurer`] drives: [`TextRun::wrap`] splits one string into lines, and a [`Panel`]
//! stacks its entries top-down. Everything else is arithmetic on the caller's coordinates.
//!
//! Every attribute value and text node is escaped, and attribute names that are not XML names
//! are dropped, so no field of the model can inject markup. [`RawSvg`] is the one deliberate
//! exception, and the caller vouches for it.

use std::collections::BTreeSet;

use crate::geometry::polyline::{clean_path, polyline_point_at, shorten_end};
use crate::geometry::EPS;
use crate::sort::SortDyn;
use crate::svg::model::*;
use crate::svg::text::{TextMeasurer, TextMetrics};
use crate::svg::theme::Theme;

// -- formatting and escaping -------------------------------------------------------------------

/// 2-decimal precision keeps the emitted SVG compact and byte-identical across platforms.
pub(crate) fn fmt(v: f64) -> String {
    if !v.is_finite() {
        return "0.00".to_string();
    }
    format!("{v:.2}")
}

/// Escape the five XML metacharacters, for attribute values and text nodes alike.
pub(crate) fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// XML `Name` as far as this renderer is concerned: `[A-Za-z_:][A-Za-z0-9_.:-]*`.
pub(crate) fn is_valid_attr_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' || c == ':' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'))
}

fn push_attr(out: &mut String, name: &str, value: &str) {
    if !is_valid_attr_name(name) {
        return;
    }
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    out.push_str(&xml_escape(value));
    out.push('"');
}

/// Render the escape-hatch attributes of one drawable.
fn attrs_str(a: &Attrs) -> String {
    let mut out = String::new();
    if let Some(id) = &a.id {
        push_attr(&mut out, "id", id);
    }
    if !a.class.is_empty() {
        push_attr(&mut out, "class", &a.class.join(" "));
    }
    for (name, value) in &a.data {
        push_attr(&mut out, &format!("data-{name}"), value);
    }
    if let Some(style) = &a.style {
        push_attr(&mut out, "style", style);
    }
    for (name, value) in &a.extra {
        push_attr(&mut out, name, value);
    }
    out
}

// -- builtin symbols ---------------------------------------------------------------------------

/// The "maps to" glyph: a tail bar plus an arrowhead, shaped like `\u{21A6}`. It sits between two
/// badge clusters to say the left one's type carries the right one.
pub const SYMBOL_MAPSTO: &str = "gl-mapsto";
/// Filled dot legend swatch.
pub const SYMBOL_SWATCH_DOT: &str = "gl-swatch-dot";
/// Dashed-ring legend swatch.
pub const SYMBOL_SWATCH_LOOP: &str = "gl-swatch-loop";
/// Dot-arrow-dot legend swatch (the legend-sized [`SYMBOL_MAPSTO`]).
pub const SYMBOL_SWATCH_MAPSTO: &str = "gl-swatch-mapsto";
/// "Play" triangle, for start-terminal chrome.
pub const SYMBOL_PLAY_TRIANGLE: &str = "gl-play-triangle";
/// "Stop" square, for end-terminal chrome.
pub const SYMBOL_STOP_SQUARE: &str = "gl-stop-square";

/// The builtin symbols. All paint with `currentColor`, so a `<use>` tints them.
///
/// They are emitted only when something in the scene references them, and a scene-level
/// [`SymbolDef`] with the same id wins.
pub fn builtin_symbols() -> Vec<SymbolDef> {
    vec![
        SymbolDef::new(
            SYMBOL_MAPSTO,
            Rect::new(0.0, 0.0, 9.0, 8.0),
            concat!(
                "<line x1=\"0.65\" y1=\"1.00\" x2=\"0.65\" y2=\"7.00\" stroke=\"currentColor\" stroke-width=\"1.3\" stroke-linecap=\"round\"/>\n",
                "<line x1=\"0.65\" y1=\"4.00\" x2=\"5.50\" y2=\"4.00\" stroke=\"currentColor\" stroke-width=\"1.3\" stroke-linecap=\"round\"/>\n",
                "<polygon points=\"5.50,1.80 5.50,6.20 9.00,4.00\" fill=\"currentColor\"/>\n",
            ),
        ),
        SymbolDef::new(
            SYMBOL_SWATCH_DOT,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            "<circle cx=\"5\" cy=\"5\" r=\"3.5\" fill=\"currentColor\"/>\n",
        ),
        SymbolDef::new(
            SYMBOL_SWATCH_LOOP,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            "<circle cx=\"5\" cy=\"5\" r=\"4.35\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.3\" stroke-dasharray=\"1.6 1.3\"/>\n",
        ),
        SymbolDef::new(
            SYMBOL_SWATCH_MAPSTO,
            Rect::new(0.0, 0.0, 24.0, 8.0),
            concat!(
                "<circle cx=\"2.60\" cy=\"4.00\" r=\"2.6\" fill=\"currentColor\"/>\n",
                "<circle cx=\"21.40\" cy=\"4.00\" r=\"2.6\" fill=\"currentColor\"/>\n",
                "<line x1=\"6.40\" y1=\"1.00\" x2=\"6.40\" y2=\"7.00\" stroke=\"currentColor\" stroke-width=\"1.3\" stroke-linecap=\"round\"/>\n",
                "<line x1=\"6.40\" y1=\"4.00\" x2=\"15.10\" y2=\"4.00\" stroke=\"currentColor\" stroke-width=\"1.3\" stroke-linecap=\"round\"/>\n",
                "<polygon points=\"15.10,1.80 15.10,6.20 18.60,4.00\" fill=\"currentColor\"/>\n",
            ),
        ),
        SymbolDef::new(
            SYMBOL_PLAY_TRIANGLE,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            "<polygon points=\"0,0 0,10 10,5\" fill=\"currentColor\"/>\n",
        ),
        SymbolDef::new(
            SYMBOL_STOP_SQUARE,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            "<rect x=\"0\" y=\"0\" width=\"10\" height=\"10\" rx=\"1\" fill=\"currentColor\"/>\n",
        ),
    ]
}

/// Marker size for a stroke width, when the caller does not pin one.
pub fn marker_size_for(stroke_width: f64) -> f64 {
    f64::max(12.0, stroke_width * 2.5)
}

/// `(body, ref_x, tip_x, view_box_width)` in the marker's own 12-unit-high box.
fn builtin_marker_geometry(marker: BuiltinMarker, col: &str) -> (String, f64, f64, f64) {
    match marker {
        BuiltinMarker::Arrow => (
            format!(
                r#"<path d="M 1,1 L 11,6 L 1,11 Z" fill="{col}" stroke="{col}" stroke-linejoin="round"/>"#
            ),
            1.0,
            11.0,
            12.0,
        ),
        BuiltinMarker::ArrowCentered => (
            format!(r#"<path d="M 1,1 L 11,6 L 1,11 Z" fill="{col}"/>"#),
            6.0,
            6.0,
            12.0,
        ),
        BuiltinMarker::Ball => (
            format!(r#"<circle cx="6" cy="6" r="5" fill="{col}"/>"#),
            6.0,
            6.0,
            12.0,
        ),
        BuiltinMarker::Bar => (
            format!(r#"<rect x="5.25" y="1" width="1.5" height="10" fill="{col}"/>"#),
            6.0,
            6.0,
            12.0,
        ),
        BuiltinMarker::ArrowBall => (
            format!(
                r#"<path d="M 2,1 L 12,6 L 2,11 Z" fill="{col}"/><circle cx="12" cy="6" r="5" fill="{col}"/>"#
            ),
            12.0,
            12.0,
            18.0,
        ),
        BuiltinMarker::ArrowBar => (
            format!(
                r#"<rect x="1.5" y="1" width="1.5" height="10" fill="{col}"/><path d="M 4,1 L 14,6 L 4,11 Z" fill="{col}"/>"#
            ),
            9.0,
            9.0,
            16.0,
        ),
        BuiltinMarker::BallArrow => (
            format!(
                r#"<circle cx="5" cy="6" r="5" fill="{col}"/><path d="M 20,1 L 10,6 L 20,11 Z" fill="{col}"/>"#
            ),
            5.0,
            5.0,
            20.0,
        ),
        BuiltinMarker::BallBarArrow => (
            format!(
                r#"<circle cx="5" cy="6" r="5" fill="{col}"/><rect x="21" y="1" width="1.5" height="10" fill="{col}"/><path d="M 20,1 L 10,6 L 20,11 Z" fill="{col}"/>"#
            ),
            5.0,
            5.0,
            23.0,
        ),
    }
}

// -- path emission -----------------------------------------------------------------------------

type Pt = (f64, f64);

/// Straight `M`/`L` path through the points.
fn straight_path(pts: &[Pt]) -> String {
    let Some(&(x0, y0)) = pts.first() else {
        return String::new();
    };
    let mut d = format!("M {},{}", fmt(x0), fmt(y0));
    for &(x, y) in pts.iter().skip(1) {
        d.push_str(&format!(" L {},{}", fmt(x), fmt(y)));
    }
    d
}

/// Path through the points with interior joins rounded to radius `r` by quadratic corners.
fn rounded_path(pts: &[Pt], r: f64) -> String {
    if pts.len() < 3 {
        return straight_path(pts);
    }
    let mut d = match pts.first() {
        Some(&(x, y)) => format!("M {},{}", fmt(x), fmt(y)),
        None => return String::new(),
    };
    let mut pen = pts[0];
    for i in 1..pts.len() - 1 {
        if collinear(pts[i - 1], pts[i], pts[i + 1]) {
            continue;
        }
        let (a, b) = corner_ends(pts[i - 1], pts[i], pts[i + 1], r);
        line_to(&mut d, &mut pen, a);
        d.push_str(&format!(
            " Q {},{} {},{}",
            fmt(pts[i].0),
            fmt(pts[i].1),
            fmt(b.0),
            fmt(b.1)
        ));
        pen = b;
    }
    if let Some(&last) = pts.last() {
        line_to(&mut d, &mut pen, last);
    }
    d
}

/// Whether `p0`, `p1` and `p2` lie on one line, so `p1` is no corner.
fn collinear(p0: Pt, p1: Pt, p2: Pt) -> bool {
    let cross = (p1.0 - p0.0) * (p2.1 - p1.1) - (p1.1 - p0.1) * (p2.0 - p1.0);
    cross.abs()
        < 1e-6
            * f64::hypot(p1.0 - p0.0, p1.1 - p0.1).max(1.0)
            * f64::hypot(p2.0 - p1.0, p2.1 - p1.1).max(1.0)
}

/// Where the rounded corner at `p1` starts and ends. Each end is `r` along its leg, or half the
/// leg where that is shorter.
fn corner_ends(p0: Pt, p1: Pt, p2: Pt, r: f64) -> (Pt, Pt) {
    let l1 = f64::hypot(p1.0 - p0.0, p1.1 - p0.1).max(1.0);
    let l2 = f64::hypot(p2.0 - p1.0, p2.1 - p1.1).max(1.0);
    let rr = r.min(l1 / 2.0).min(l2 / 2.0);
    (
        (
            p1.0 - (p1.0 - p0.0) / l1 * rr,
            p1.1 - (p1.1 - p0.1) / l1 * rr,
        ),
        (
            p1.0 + (p2.0 - p1.0) / l2 * rr,
            p1.1 + (p2.1 - p1.1) / l2 * rr,
        ),
    )
}

/// Appends ` L to`, unless the pen is already there.
fn line_to(d: &mut String, pen: &mut Pt, to: Pt) {
    if (to.0 - pen.0).abs() < 0.005 && (to.1 - pen.1).abs() < 0.005 {
        return;
    }
    d.push_str(&format!(" L {},{}", fmt(to.0), fmt(to.1)));
    *pen = to;
}

/// One vertical run of a drawn edge, for [`hops_of`]: `(edge index, x, y_min, y_max)`.
type VerticalRun = (usize, f64, f64, f64);

/// Every axis-aligned vertical run of every edge, as drawn (after the edge's own simplification).
fn vertical_runs(edges: &[SceneEdge]) -> Vec<VerticalRun> {
    let mut out = Vec::new();
    for (i, e) in edges.iter().enumerate() {
        let mut pts = pts_of(&e.points);
        if e.simplify > 0.0 {
            pts = clean_path(&pts, e.simplify);
        }
        for w in pts.windows(2) {
            if (w[0].0 - w[1].0).abs() < 0.5 && (w[0].1 - w[1].1).abs() >= 0.5 {
                out.push((i, w[0].0, w[0].1.min(w[1].1), w[0].1.max(w[1].1)));
            }
        }
    }
    out
}

/// Where the horizontal runs of edge `index` cross another edge's vertical run, as
/// `(segment, x)`. Crossings near a run's end or a corner are skipped, so a T draws flat. Two hops
/// closer than a hop's width merge into the first.
fn hops_of(
    pts: &[Pt],
    index: usize,
    verticals: &[VerticalRun],
    hop: f64,
    corner: f64,
) -> Vec<(usize, f64)> {
    let mut out = Vec::new();
    for (k, w) in pts.windows(2).enumerate() {
        let (a, b) = (w[0], w[1]);
        if (a.1 - b.1).abs() >= 0.5 || (a.0 - b.0).abs() < 0.5 {
            continue;
        }
        let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
        let margin = hop + corner + 1.0;
        let mut xs: Vec<f64> = verticals
            .iter()
            .filter(|&&(e, x, y0, y1)| {
                e != index && x > x0 + margin && x < x1 - margin && a.1 > y0 + hop && a.1 < y1 - hop
            })
            .map(|&(_, x, _, _)| x)
            .collect();
        xs.sort_by_dyn(f64::total_cmp);
        if b.0 < a.0 {
            xs.reverse();
        }
        let mut last: Option<f64> = None;
        for x in xs {
            if last.is_some_and(|l| (x - l).abs() < 2.0 * hop + 1.0) {
                continue;
            }
            out.push((k, x));
            last = Some(x);
        }
    }
    out
}

/// [`rounded_path`] (or a straight one at radius 0) with a semicircular hop of radius `h` at each
/// `(segment, x)` of `hops`, bulging up the page whichever way the run is drawn.
fn hopping_path(pts: &[Pt], r: f64, hops: &[(usize, f64)], h: f64) -> String {
    if hops.is_empty() {
        return path_d(pts, r);
    }
    let Some(&(x0, y0)) = pts.first() else {
        return String::new();
    };
    let mut d = format!("M {},{}", fmt(x0), fmt(y0));
    let mut pen = (x0, y0);
    let n = pts.len();
    let corner = |i: usize| corner_ends(pts[i - 1], pts[i], pts[i + 1], r);
    for k in 0..n - 1 {
        // Segment k runs from pts[k] (or its corner's end) to pts[k + 1] (or its corner's start).
        let (y, dir) = (pts[k].1, (pts[k + 1].0 - pts[k].0).signum());
        for &(_, x) in hops.iter().filter(|&&(s, _)| s == k) {
            d.push_str(&format!(
                " L {},{} A {},{} 0 0 {} {},{}",
                fmt(x - dir * h),
                fmt(y),
                fmt(h),
                fmt(h),
                if dir > 0.0 { 1 } else { 0 },
                fmt(x + dir * h),
                fmt(y)
            ));
        }
        if k + 1 < n - 1 && r > 0.0 && !collinear(pts[k], pts[k + 1], pts[k + 2]) {
            let (a, b) = corner(k + 1);
            let p = pts[k + 1];
            line_to(&mut d, &mut pen, a);
            d.push_str(&format!(
                " Q {},{} {},{}",
                fmt(p.0),
                fmt(p.1),
                fmt(b.0),
                fmt(b.1)
            ));
            pen = b;
        } else {
            line_to(&mut d, &mut pen, pts[k + 1]);
        }
    }
    d
}

fn path_d(pts: &[Pt], corner_radius: f64) -> String {
    if corner_radius > 0.0 {
        rounded_path(pts, corner_radius)
    } else {
        straight_path(pts)
    }
}

// -- bounds accumulation -----------------------------------------------------------------------

#[derive(Debug, Default, Clone, Copy)]
struct Bounds {
    rect: Option<Rect>,
}

impl Bounds {
    fn add(&mut self, r: Rect) {
        if !r.x.is_finite() || !r.y.is_finite() || !r.w.is_finite() || !r.h.is_finite() {
            return;
        }
        self.rect = Some(match self.rect {
            Some(a) => a.union(&r),
            None => r,
        });
    }

    fn add_circle(&mut self, c: Point, r: f64) {
        self.add(Rect::new(c.x - r, c.y - r, 2.0 * r, 2.0 * r));
    }
}

// -- defs collection ---------------------------------------------------------------------------

/// Everything that has to end up inside `<defs>`, collected while drawing.
#[derive(Default)]
struct Defs {
    /// `(dedup key, body)`. The id is `gl-mk-<index>`.
    markers: Vec<(String, String)>,
    /// `(dedup key, body)`. The id is `gl-grad-<index>`.
    gradients: Vec<(String, String)>,
    used_symbols: BTreeSet<String>,
}

impl Defs {
    fn use_symbol(&mut self, id: &str) {
        self.used_symbols.insert(id.to_string());
    }

    fn gradient(&mut self, g: &Gradient) -> String {
        let key = format!(
            "{},{},{},{}|{}",
            fmt(g.from.x),
            fmt(g.from.y),
            fmt(g.to.x),
            fmt(g.to.y),
            g.stops
                .iter()
                .map(|s| format!("{}:{}", fmt(s.offset), s.color))
                .collect::<Vec<_>>()
                .join(";")
        );
        if let Some(i) = self.gradients.iter().position(|(k, _)| *k == key) {
            return format!("gl-grad-{i}");
        }
        let id = format!("gl-grad-{}", self.gradients.len());
        let mut body = format!(
            r#"<linearGradient id="{id}" gradientUnits="userSpaceOnUse" x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}">"#,
            x1 = fmt(g.from.x),
            y1 = fmt(g.from.y),
            x2 = fmt(g.to.x),
            y2 = fmt(g.to.y),
        );
        for stop in &g.stops {
            body.push_str(&format!(
                r#"<stop offset="{off}%" stop-color="{col}"/>"#,
                off = fmt(stop.offset * 100.0),
                col = xml_escape(&stop.color),
            ));
        }
        body.push_str("</linearGradient>\n");
        self.gradients.push((key, body));
        id
    }

    /// Register the marker for one edge end. Returns its id and how far the path must pull back
    /// so the marker's visible tip lands on the endpoint.
    fn marker(
        &mut self,
        marker: &MarkerRef,
        scene: &Scene,
        fallback_color: &str,
        stroke_width: f64,
    ) -> Option<(String, f64)> {
        match marker {
            MarkerRef::None => None,
            MarkerRef::Custom { id } => {
                let def = scene.markers.iter().find(|m| &m.id == id)?;
                Some((def.id.clone(), def.tip_gap.max(0.0)))
            }
            MarkerRef::Builtin {
                marker,
                color,
                size,
            } => {
                let col = color.as_deref().unwrap_or(fallback_color);
                let ms = size.unwrap_or_else(|| marker_size_for(stroke_width));
                let scale = ms / 12.0;
                let (body, ref_x, tip_x, vb_w) = builtin_marker_geometry(*marker, &xml_escape(col));
                let key = format!("{marker:?}|{col}|{}", fmt(ms));
                let id = match self.markers.iter().position(|(k, _)| *k == key) {
                    Some(i) => format!("gl-mk-{i}"),
                    None => {
                        let id = format!("gl-mk-{}", self.markers.len());
                        let def = format!(
                            r#"<marker id="{id}" markerWidth="{mw}" markerHeight="{mh}" viewBox="0 0 {vw} 12" orient="auto" refX="{rx}" refY="6" markerUnits="userSpaceOnUse">{body}</marker>
"#,
                            mw = fmt(ms * vb_w / 12.0),
                            mh = fmt(ms),
                            vw = fmt(vb_w),
                            rx = fmt(ref_x),
                        );
                        self.markers.push((key, def));
                        id
                    }
                };
                // Tuck the tip half a stroke into the border so a thick stroke shows no
                // anti-aliased seam at the node.
                let gap = ((tip_x - ref_x) * scale - stroke_width / 2.0).max(0.0);
                Some((id, gap))
            }
        }
    }
}

/// The box a marker occupies around the endpoint it sits on, for the scene bounds.
fn marker_extent(marker: &MarkerRef, scene: &Scene, stroke_width: f64) -> f64 {
    match marker {
        MarkerRef::None => 0.0,
        MarkerRef::Custom { id } => scene
            .markers
            .iter()
            .find(|m| &m.id == id)
            .map(|m| m.size.w.max(m.size.h))
            .unwrap_or(0.0),
        MarkerRef::Builtin { marker, size, .. } => {
            let ms = size.unwrap_or_else(|| marker_size_for(stroke_width));
            let (_, _, _, vb_w) = builtin_marker_geometry(*marker, "");
            ms * vb_w / 12.0
        }
    }
}

// -- paint / stroke attributes -----------------------------------------------------------------

fn paint_str(paint: &Paint, defs: &mut Defs) -> String {
    match paint {
        Paint::Color(c) => xml_escape(c),
        Paint::Gradient(g) => format!("url(#{})", defs.gradient(g)),
    }
}

/// ` stroke=".." stroke-width=".."` plus whatever else the stroke asks for.
fn stroke_attrs(stroke: &Stroke, defs: &mut Defs) -> String {
    let mut out = format!(
        r#" stroke="{}" stroke-width="{}""#,
        paint_str(&stroke.paint, defs),
        fmt(stroke.width)
    );
    if let Some(dash) = &stroke.dash {
        out.push_str(&format!(r#" stroke-dasharray="{}""#, xml_escape(dash)));
    }
    if stroke.linecap != LineCap::Butt {
        out.push_str(&format!(r#" stroke-linecap="{}""#, stroke.linecap.as_str()));
    }
    if let Some(op) = stroke.opacity {
        out.push_str(&format!(r#" stroke-opacity="{}""#, fmt(op)));
    }
    out
}

// -- text --------------------------------------------------------------------------------------

/// The lines one run occupies: its own text, or the wrapped split.
fn run_lines(run: &TextRun, font: &FontSpec, measurer: &dyn TextMeasurer) -> Vec<String> {
    match &run.wrap {
        Some(w) => measurer.wrap(&run.text, font, w.max_width, w.max_lines),
        None if run.text.is_empty() => Vec::new(),
        None => vec![run.text.clone()],
    }
}

fn line_height(run: &TextRun, font: &FontSpec) -> f64 {
    run.wrap
        .as_ref()
        .map(|w| w.line_height)
        .unwrap_or(font.size + 1.5)
}

/// Baseline (or centre, for [`Baseline::Central`]) of the first line.
fn first_line_y(run: &TextRun, font: &FontSpec, anchor: Point, lines: usize) -> f64 {
    let lh = line_height(run, font);
    anchor.y + run.dy - (lines.saturating_sub(1) as f64) * lh / 2.0
}

fn text_box(metrics: &TextMetrics, x: f64, y: f64, run: &TextRun) -> Rect {
    let (x0, w) = match run.anchor {
        TextAnchor::Start => (x, metrics.width),
        TextAnchor::Middle => (x - metrics.width / 2.0, metrics.width),
        TextAnchor::End => (x - metrics.width, metrics.width),
    };
    let (y0, h) = match run.baseline {
        Baseline::Central => (y - metrics.height() / 2.0, metrics.height()),
        Baseline::Alphabetic => (y - metrics.ascent, metrics.height()),
    };
    Rect::new(x0, y0, w, h)
}

/// Draw one run anchored at `anchor` (a node's centre, an edge label's point, a panel origin).
fn render_text(
    run: &TextRun,
    anchor: Point,
    scene: &Scene,
    theme: &Theme,
    measurer: &dyn TextMeasurer,
    out: &mut String,
) {
    let font = run.font(&scene.font);
    let lines = run_lines(run, &font, measurer);
    if lines.is_empty() {
        return;
    }
    let x = anchor.x + run.dx;
    let y0 = first_line_y(run, &font, anchor, lines.len());
    let lh = line_height(run, &font);
    let color = run
        .color
        .clone()
        .unwrap_or_else(|| theme.palette.node_text.clone());
    for (i, line) in lines.iter().enumerate() {
        out.push_str(&format!(
            r#"<text x="{x}" y="{y}" text-anchor="{anchor}""#,
            x = fmt(x),
            y = fmt(y0 + i as f64 * lh),
            anchor = run.anchor.as_str(),
        ));
        if run.baseline == Baseline::Central {
            out.push_str(r#" dominant-baseline="central""#);
        }
        out.push_str(&format!(
            r#" font-size="{fs}" font-weight="{fw}""#,
            fs = fmt(font.size),
            fw = fmt(font.weight),
        ));
        if let Some(family) = &run.family {
            out.push_str(&format!(r#" font-family="{}""#, xml_escape(family)));
        }
        if font.italic {
            out.push_str(r#" font-style="italic""#);
        }
        if let Some(halo) = &run.halo {
            out.push_str(&format!(
                r#" paint-order="stroke" stroke="{col}" stroke-width="{w}" stroke-linejoin="round""#,
                col = xml_escape(&halo.color),
                w = fmt(halo.width),
            ));
        }
        out.push_str(&format!(r#" fill="{}""#, xml_escape(&color)));
        out.push_str(&attrs_str(&run.attrs));
        out.push_str(&format!(">{}</text>\n", xml_escape(line)));
    }
}

fn text_bounds(
    run: &TextRun,
    anchor: Point,
    scene: &Scene,
    measurer: &dyn TextMeasurer,
    bounds: &mut Bounds,
) {
    let font = run.font(&scene.font);
    let lines = run_lines(run, &font, measurer);
    let x = anchor.x + run.dx;
    let y0 = first_line_y(run, &font, anchor, lines.len());
    let lh = line_height(run, &font);
    let halo = run.halo.as_ref().map(|h| h.width / 2.0).unwrap_or(0.0);
    for (i, line) in lines.iter().enumerate() {
        let m = measurer.measure(line, &font);
        bounds.add(text_box(&m, x, y0 + i as f64 * lh, run).inflate(halo));
    }
}

// -- glyph rows --------------------------------------------------------------------------------

struct PlacedGlyph {
    cluster: usize,
    center: Point,
    radius: f64,
}

struct PlacedSeparator {
    cluster: usize,
    /// Left edge of the separator's own box.
    x: f64,
    cy: f64,
}

struct PlacedNumeral {
    center: Point,
    total: u64,
}

#[derive(Default)]
struct GlyphPlan {
    glyphs: Vec<PlacedGlyph>,
    separators: Vec<PlacedSeparator>,
    numeral: Option<PlacedNumeral>,
}

/// Place every glyph of a row. Sizes come from the row, except under [`GlyphLayout::Fit`].
fn plan_glyph_row(row: &GlyphRow, node_center: Point) -> GlyphPlan {
    let mut plan = GlyphPlan::default();
    let clusters = &row.clusters;
    let total: u64 = clusters.iter().map(|c| c.count).sum();
    if clusters.is_empty() || total == 0 {
        return plan;
    }
    let anchor = Point::new(node_center.x + row.dx, node_center.y + row.dy);

    match &row.layout {
        GlyphLayout::Fit(fit) => {
            let unit = (fit.min_diameter + fit.gap).max(0.01);
            let max_glyphs = (fit.max_width / unit).floor().max(1.0) as u64;
            if total > max_glyphs {
                if let Some(_numeral) = &fit.numeral {
                    plan.numeral = Some(PlacedNumeral {
                        center: anchor,
                        total,
                    });
                    return plan;
                }
            }
            let d =
                (fit.max_width / total as f64 - fit.gap).clamp(fit.min_diameter, fit.max_diameter);
            let row_w = total as f64 * d + (total - 1) as f64 * fit.gap;
            let mut x = anchor.x - row_w / 2.0 + d / 2.0;
            for (ci, cluster) in clusters.iter().enumerate() {
                for _ in 0..cluster.count {
                    plan.glyphs.push(PlacedGlyph {
                        cluster: ci,
                        center: Point::new(x, anchor.y),
                        radius: d / 2.0,
                    });
                    x += d + fit.gap;
                }
            }
        }
        GlyphLayout::Exact(exact) => {
            let widths: Vec<f64> = clusters.iter().map(|c| c.width()).collect();
            let gap_before = |i: usize| -> f64 {
                match &clusters[i].separator {
                    Some(s) => s.gap_before + s.width + s.gap_after,
                    None => exact.cluster_gap,
                }
            };
            // A separator binds its cluster to the previous one, so no row break falls between.
            let mut runs: Vec<Vec<usize>> = Vec::new();
            for (i, cluster) in clusters.iter().enumerate() {
                if cluster.separator.is_some() {
                    if let Some(last) = runs.last_mut() {
                        last.push(i);
                        continue;
                    }
                }
                runs.push(vec![i]);
            }
            let run_width = |run: &[usize]| -> f64 {
                run.iter().enumerate().fold(0.0, |w, (j, &i)| {
                    w + widths[i] + if j > 0 { gap_before(i) } else { 0.0 }
                })
            };
            // A cluster wider than the wrap width still gets a row of its own.
            let usable = exact
                .wrap_width
                .max(widths.iter().copied().fold(0.0, f64::max));
            let mut rows: Vec<Vec<usize>> = Vec::new();
            let mut cur: Vec<usize> = Vec::new();
            let mut cur_w = 0.0_f64;
            for run in &runs {
                let rw = run_width(run);
                let add = rw
                    + if cur.is_empty() {
                        0.0
                    } else {
                        exact.cluster_gap
                    };
                if !cur.is_empty() && cur_w + add > usable {
                    rows.push(std::mem::take(&mut cur));
                    cur_w = rw;
                    cur.extend_from_slice(run);
                } else {
                    cur_w += add;
                    cur.extend_from_slice(run);
                }
            }
            if !cur.is_empty() {
                rows.push(cur);
            }

            let max_r = clusters.iter().map(|c| c.radius).fold(0.0_f64, f64::max);
            let ideal = 2.0 * max_r + exact.row_gap;
            // `anchor.y` is where the first row belongs, since the caller measured clearance to it.
            let row_h = match (rows.len() > 1, exact.max_bottom) {
                (true, Some(bottom)) => {
                    let available =
                        ((bottom - (anchor.y - max_r)) / (rows.len() - 1) as f64).max(2.0 * max_r);
                    ideal.min(available)
                }
                _ => ideal,
            };

            for (ri, row_idx) in rows.iter().enumerate() {
                let ry = anchor.y + ri as f64 * row_h;
                let row_w = run_width(row_idx);
                let mut gx = anchor.x - row_w / 2.0;
                for (j, &ci) in row_idx.iter().enumerate() {
                    if j > 0 {
                        let gap = gap_before(ci);
                        if let Some(sep) = &clusters[ci].separator {
                            plan.separators.push(PlacedSeparator {
                                cluster: ci,
                                x: gx + sep.gap_before,
                                cy: ry,
                            });
                        }
                        gx += gap;
                    }
                    let cluster = &clusters[ci];
                    let step = cluster.step();
                    // Right to left, so the leftmost glyph ends up on top where they overlap.
                    for i in (0..cluster.count).rev() {
                        plan.glyphs.push(PlacedGlyph {
                            cluster: ci,
                            center: Point::new(gx + cluster.radius + i as f64 * step, ry),
                            radius: cluster.radius,
                        });
                    }
                    gx += widths[ci];
                }
            }
        }
    }
    plan
}

fn render_glyph_row(
    row: &GlyphRow,
    node_center: Point,
    theme: &Theme,
    defs: &mut Defs,
    out: &mut String,
) {
    let plan = plan_glyph_row(row, node_center);
    let open_group = !row.attrs.is_empty();
    if open_group {
        out.push_str(&format!("<g{}>\n", attrs_str(&row.attrs)));
    }
    if let Some(numeral) = &plan.numeral {
        if let GlyphLayout::Fit(fit) = &row.layout {
            if let Some(style) = &fit.numeral {
                let color = style
                    .color
                    .clone()
                    .unwrap_or_else(|| theme.palette.node_text.clone());
                out.push_str(&format!(
                    r#"<text x="{x}" y="{y}" text-anchor="middle" dominant-baseline="central" font-size="{fs}" font-weight="{fw}" fill="{col}">{n}</text>
"#,
                    x = fmt(numeral.center.x),
                    y = fmt(numeral.center.y),
                    fs = fmt(style.size),
                    fw = fmt(style.weight),
                    col = xml_escape(&color),
                    n = numeral.total,
                ));
            }
        }
    }
    for sep in &plan.separators {
        let Some(cluster) = row.clusters.get(sep.cluster) else {
            continue;
        };
        let Some(s) = &cluster.separator else {
            continue;
        };
        let color = s
            .color
            .clone()
            .unwrap_or_else(|| theme.palette.arc_color.clone());
        defs.use_symbol(&s.symbol);
        out.push_str(&use_element(
            &s.symbol,
            Rect::new(sep.x, sep.cy - s.height / 2.0, s.width, s.height),
            Some(&color),
            &Attrs::default(),
        ));
    }
    for g in &plan.glyphs {
        let Some(cluster) = row.clusters.get(g.cluster) else {
            continue;
        };
        let color = cluster
            .color
            .clone()
            .unwrap_or_else(|| theme.palette.node_text.clone());
        let outline = cluster
            .outline
            .as_ref()
            .map(|s| stroke_attrs(s, defs))
            .unwrap_or_default();
        match &cluster.glyph {
            Glyph::Dot => out.push_str(&format!(
                r#"<circle cx="{cx}" cy="{cy}" r="{r}" fill="{col}"{outline}{attrs}/>
"#,
                cx = fmt(g.center.x),
                cy = fmt(g.center.y),
                r = fmt(g.radius),
                col = xml_escape(&color),
                attrs = attrs_str(&cluster.attrs),
            )),
            Glyph::Square {
                scale,
                corner_radius,
                opacity,
            } => {
                let side = 2.0 * g.radius * scale;
                out.push_str(&format!(
                    r#"<rect x="{x}" y="{y}" width="{w}" height="{w}" rx="{rx}" fill="{col}""#,
                    x = fmt(g.center.x - side / 2.0),
                    y = fmt(g.center.y - side / 2.0),
                    w = fmt(side),
                    rx = fmt(*corner_radius),
                    col = xml_escape(&color),
                ));
                if (*opacity - 1.0).abs() > f64::EPSILON {
                    out.push_str(&format!(r#" opacity="{}""#, fmt(*opacity)));
                }
                out.push_str(&outline);
                out.push_str(&attrs_str(&cluster.attrs));
                out.push_str("/>\n");
            }
            Glyph::Symbol { id } => {
                defs.use_symbol(id);
                out.push_str(&use_element(
                    id,
                    Rect::new(
                        g.center.x - g.radius,
                        g.center.y - g.radius,
                        2.0 * g.radius,
                        2.0 * g.radius,
                    ),
                    Some(&color),
                    &cluster.attrs,
                ));
            }
        }
    }
    if open_group {
        out.push_str("</g>\n");
    }
}

fn glyph_row_bounds(row: &GlyphRow, node_center: Point, bounds: &mut Bounds) {
    let plan = plan_glyph_row(row, node_center);
    for g in &plan.glyphs {
        let extra = row
            .clusters
            .get(g.cluster)
            .and_then(|c| c.outline.as_ref())
            .map(|s| s.width / 2.0)
            .unwrap_or(0.0);
        bounds.add_circle(g.center, g.radius + extra);
    }
    for sep in &plan.separators {
        if let Some(s) = row
            .clusters
            .get(sep.cluster)
            .and_then(|c| c.separator.as_ref())
        {
            bounds.add(Rect::new(sep.x, sep.cy - s.height / 2.0, s.width, s.height));
        }
    }
    if let Some(n) = &plan.numeral {
        if let GlyphLayout::Fit(fit) = &row.layout {
            if let Some(style) = &fit.numeral {
                let w = style.size * 0.6 * n.total.to_string().chars().count() as f64;
                bounds.add(Rect::new(
                    n.center.x - w / 2.0,
                    n.center.y - style.size / 2.0,
                    w,
                    style.size,
                ));
            }
        }
    }
}

// -- <use> -------------------------------------------------------------------------------------

fn use_element(symbol: &str, box_: Rect, color: Option<&str>, attrs: &Attrs) -> String {
    let mut out = format!(
        r##"<use href="#{id}" x="{x}" y="{y}" width="{w}" height="{h}""##,
        id = xml_escape(symbol),
        x = fmt(box_.x),
        y = fmt(box_.y),
        w = fmt(box_.w),
        h = fmt(box_.h),
    );
    if let Some(c) = color {
        out.push_str(&format!(r#" color="{}""#, xml_escape(c)));
    }
    out.push_str(&attrs_str(attrs));
    out.push_str("/>\n");
    out
}

fn raw_element(raw: &RawSvg, out: &mut String) {
    if raw.attrs.is_empty() {
        out.push_str(&raw.markup);
    } else {
        out.push_str(&format!("<g{}>", attrs_str(&raw.attrs)));
        out.push_str(&raw.markup);
        out.push_str("</g>");
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
}

// -- nodes -------------------------------------------------------------------------------------

fn node_outline(node: &SceneNode, fill: &str, stroke_attrs: &str, attrs: &str) -> String {
    let c = node.rect.center();
    match node.shape {
        Shape::Circle => format!(
            r#"<circle cx="{cx}" cy="{cy}" r="{r}" fill="{fill}"{stroke_attrs}{attrs}/>
"#,
            cx = fmt(c.x),
            cy = fmt(c.y),
            r = fmt(node.rect.w / 2.0),
        ),
        Shape::Ellipse => format!(
            r#"<ellipse cx="{cx}" cy="{cy}" rx="{rx}" ry="{ry}" fill="{fill}"{stroke_attrs}{attrs}/>
"#,
            cx = fmt(c.x),
            cy = fmt(c.y),
            rx = fmt(node.rect.w / 2.0),
            ry = fmt(node.rect.h / 2.0),
        ),
        // Clamp like CSS border-radius. SVG clamps rx and ry independently, so an oversized
        // radius (999 for a pill) would turn the box into an ellipse.
        Shape::Rect { corner_radius } => format!(
            r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" fill="{fill}"{stroke_attrs}{attrs}/>
"#,
            x = fmt(node.rect.x),
            y = fmt(node.rect.y),
            w = fmt(node.rect.w),
            h = fmt(node.rect.h),
            r = fmt(corner_radius.min(node.rect.w / 2.0).min(node.rect.h / 2.0)),
        ),
    }
}

fn render_ring(node: &SceneNode, ring: &Ring, out: &mut String) {
    let c = node.rect.center();
    let inset = ring.inset + ring.width / 2.0;
    let common = format!(
        r#" fill="none" stroke="{col}" stroke-width="{sw}"/>
"#,
        col = xml_escape(&ring.color),
        sw = fmt(ring.width),
    );
    match node.shape {
        Shape::Circle => {
            // The same circle the outline draws: diameter `w`, whatever the height.
            let r = node.rect.w / 2.0 - inset;
            if r <= 0.0 {
                return;
            }
            out.push_str(&format!(
                r#"<circle cx="{cx}" cy="{cy}" r="{r}""#,
                cx = fmt(c.x),
                cy = fmt(c.y),
                r = fmt(r)
            ));
        }
        Shape::Ellipse => {
            let (rx, ry) = (node.rect.w / 2.0 - inset, node.rect.h / 2.0 - inset);
            if rx <= 0.0 || ry <= 0.0 {
                return;
            }
            out.push_str(&format!(
                r#"<ellipse cx="{cx}" cy="{cy}" rx="{rx}" ry="{ry}""#,
                cx = fmt(c.x),
                cy = fmt(c.y),
                rx = fmt(rx),
                ry = fmt(ry)
            ));
        }
        Shape::Rect { corner_radius } => {
            let (w, h) = (node.rect.w - 2.0 * inset, node.rect.h - 2.0 * inset);
            if w <= 0.0 || h <= 0.0 {
                return;
            }
            out.push_str(&format!(
                r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}""#,
                x = fmt(c.x - w / 2.0),
                y = fmt(c.y - h / 2.0),
                w = fmt(w),
                h = fmt(h),
                r = fmt((corner_radius - inset).max(0.0)),
            ));
        }
    }
    out.push_str(&common);
}

fn render_node(
    node: &SceneNode,
    scene: &Scene,
    theme: &Theme,
    measurer: &dyn TextMeasurer,
    defs: &mut Defs,
    out: &mut String,
) {
    let wrap = !node.attrs.is_empty();
    if wrap {
        out.push_str(&format!("<g{}>\n", attrs_str(&node.attrs)));
    }
    let fill = node
        .fill
        .clone()
        .unwrap_or_else(|| theme.palette.node_bg.clone());
    let stroke = node.stroke.clone().unwrap_or_else(|| Stroke {
        paint: Paint::Color(theme.palette.node_border.clone()),
        ..Stroke::default()
    });
    out.push_str(&node_outline(
        node,
        &xml_escape(&fill),
        &stroke_attrs(&stroke, defs),
        "",
    ));

    let center = node.rect.center();
    for content in &node.content {
        match content {
            NodeContent::Text(run) => render_text(run, center, scene, theme, measurer, out),
            NodeContent::Glyphs(row) => render_glyph_row(row, center, theme, defs, out),
            NodeContent::Rings { rings } => {
                for ring in rings {
                    render_ring(node, ring, out);
                }
            }
            NodeContent::Use {
                symbol,
                at,
                size,
                color,
                attrs,
            } => {
                defs.use_symbol(symbol);
                out.push_str(&use_element(
                    symbol,
                    Rect::from_center(Point::new(center.x + at.x, center.y + at.y), *size),
                    color.as_deref(),
                    attrs,
                ));
            }
            NodeContent::Raw(raw) => raw_element(raw, out),
        }
    }
    if wrap {
        out.push_str("</g>\n");
    }
}

fn node_bounds(
    node: &SceneNode,
    scene: &Scene,
    measurer: &dyn TextMeasurer,
    theme_stroke: f64,
    bounds: &mut Bounds,
) {
    let half_stroke = node
        .stroke
        .as_ref()
        .map(|s| s.width / 2.0)
        .unwrap_or(theme_stroke / 2.0);
    bounds.add(node.outline_rect().inflate(half_stroke));
    let center = node.rect.center();
    for content in &node.content {
        match content {
            NodeContent::Text(run) => text_bounds(run, center, scene, measurer, bounds),
            NodeContent::Glyphs(row) => glyph_row_bounds(row, center, bounds),
            NodeContent::Rings { .. } => {}
            NodeContent::Use { at, size, .. } => bounds.add(Rect::from_center(
                Point::new(center.x + at.x, center.y + at.y),
                *size,
            )),
            NodeContent::Raw(raw) => bounds.add(raw.bounds),
        }
    }
}

// -- edges -------------------------------------------------------------------------------------

fn pts_of(points: &[Point]) -> Vec<Pt> {
    points.iter().map(|p| (p.x, p.y)).collect()
}

/// Where one anchor lands, given the edge's untrimmed points.
fn anchor_point(at: &EdgeAnchor, raw: &[Pt]) -> Point {
    match at {
        EdgeAnchor::At(p) => *p,
        EdgeAnchor::Fraction(f) => polyline_point_at(raw, *f).into(),
    }
}

/// The path actually drawn: simplified if asked, then pulled back for whichever markers are on.
fn drawn_points(edge: &SceneEdge, start_gap: f64, end_gap: f64) -> Vec<Pt> {
    let mut pts = pts_of(&edge.points);
    // Simplify before trimming. Trimming can leave a sub-pixel stub whose direction orients the
    // arrowhead, and a later simplify pass would flatten it and swing the head off the node.
    if edge.simplify > 0.0 {
        pts = clean_path(&pts, edge.simplify);
    }
    if end_gap > 0.0 {
        pts = shorten_end(&pts, end_gap);
    }
    if start_gap > 0.0 {
        let mut rev: Vec<Pt> = pts.iter().rev().copied().collect();
        rev = shorten_end(&rev, start_gap);
        pts = rev.into_iter().rev().collect();
    }
    pts
}

fn render_decoration(
    dec: &Decoration,
    raw: &[Pt],
    theme: &Theme,
    defs: &mut Defs,
    out: &mut String,
) {
    match dec {
        Decoration::Band {
            points,
            stroke,
            corner_radius,
            attrs,
        } => {
            if points.len() < 2 {
                return;
            }
            out.push_str(&format!(
                r#"<path d="{d}" fill="none"{stroke}{attrs}/>
"#,
                d = xml_escape(&path_d(&pts_of(points), *corner_radius)),
                stroke = stroke_attrs(stroke, defs),
                attrs = attrs_str(attrs),
            ));
        }
        Decoration::Dot {
            at,
            radius,
            fill,
            stroke,
            attrs,
        } => {
            let p = anchor_point(at, raw);
            let color = fill
                .clone()
                .unwrap_or_else(|| theme.palette.arc_color.clone());
            out.push_str(&format!(
                r#"<circle cx="{cx}" cy="{cy}" r="{r}" fill="{col}"{stroke}{attrs}/>
"#,
                cx = fmt(p.x),
                cy = fmt(p.y),
                r = fmt(*radius),
                col = xml_escape(&color),
                stroke = stroke
                    .as_ref()
                    .map(|s| stroke_attrs(s, defs))
                    .unwrap_or_default(),
                attrs = attrs_str(attrs),
            ));
        }
        Decoration::Tick {
            at,
            length,
            stroke,
            attrs,
        } => {
            let p = anchor_point(at, raw);
            // Perpendicular to the local direction of travel.
            let (dx, dy) = local_direction(at, raw);
            let (nx, ny) = (-dy, dx);
            out.push_str(&format!(
                r#"<line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}"{stroke}{attrs}/>
"#,
                x1 = fmt(p.x - nx * length / 2.0),
                y1 = fmt(p.y - ny * length / 2.0),
                x2 = fmt(p.x + nx * length / 2.0),
                y2 = fmt(p.y + ny * length / 2.0),
                stroke = stroke_attrs(stroke, defs),
                attrs = attrs_str(attrs),
            ));
        }
        Decoration::Glyph {
            at,
            symbol,
            size,
            color,
            attrs,
        } => {
            let p = anchor_point(at, raw);
            defs.use_symbol(symbol);
            out.push_str(&use_element(
                symbol,
                Rect::from_center(p, *size),
                color.as_deref(),
                attrs,
            ));
        }
    }
}

/// Unit direction of the polyline at an anchor (defaults to +x for a degenerate path).
fn local_direction(at: &EdgeAnchor, raw: &[Pt]) -> (f64, f64) {
    if raw.len() < 2 {
        return (1.0, 0.0);
    }
    let f = match at {
        EdgeAnchor::Fraction(f) => f.clamp(0.0, 1.0),
        EdgeAnchor::At(_) => 0.5,
    };
    let a = polyline_point_at(raw, (f - 0.01).max(0.0));
    let b = polyline_point_at(raw, (f + 0.01).min(1.0));
    let len = f64::hypot(b.0 - a.0, b.1 - a.1);
    if len < EPS {
        (1.0, 0.0)
    } else {
        ((b.0 - a.0) / len, (b.1 - a.1) / len)
    }
}

#[allow(clippy::too_many_arguments)]
fn render_edge(
    edge: &SceneEdge,
    index: usize,
    verticals: &[VerticalRun],
    scene: &Scene,
    theme: &Theme,
    measurer: &dyn TextMeasurer,
    defs: &mut Defs,
    out: &mut String,
) {
    if edge.points.len() < 2 {
        return;
    }
    let raw = pts_of(&edge.points);
    let fallback = match &edge.stroke.paint {
        Paint::Color(c) => c.clone(),
        Paint::Gradient(_) => theme.palette.arc_color.clone(),
    };
    let start = defs.marker(&edge.marker_start, scene, &fallback, edge.stroke.width);
    let end = defs.marker(&edge.marker_end, scene, &fallback, edge.stroke.width);
    let pts = drawn_points(
        edge,
        start.as_ref().map(|(_, g)| *g).unwrap_or(0.0),
        end.as_ref().map(|(_, g)| *g).unwrap_or(0.0),
    );

    let wrap = !edge.attrs.is_empty();
    if wrap {
        out.push_str(&format!("<g{}>\n", attrs_str(&edge.attrs)));
    }

    // Bands ride under the carrier's own path, in list order.
    for dec in &edge.decorations {
        if matches!(dec, Decoration::Band { .. }) {
            render_decoration(dec, &raw, theme, defs, out);
        }
    }

    out.push_str(&format!(
        r#"<path d="{d}" fill="none"{stroke}"#,
        d = xml_escape(&if edge.hop_radius > 0.0 {
            hopping_path(
                &pts,
                edge.corner_radius,
                &hops_of(&pts, index, verticals, edge.hop_radius, edge.corner_radius),
                edge.hop_radius,
            )
        } else {
            path_d(&pts, edge.corner_radius)
        }),
        stroke = stroke_attrs(&edge.stroke, defs),
    ));
    if let Some((id, _)) = &start {
        out.push_str(&format!(r#" marker-start="url(#{})""#, xml_escape(id)));
    }
    if let Some((id, _)) = &end {
        out.push_str(&format!(r#" marker-end="url(#{})""#, xml_escape(id)));
    }
    out.push_str("/>\n");

    for dec in &edge.decorations {
        if !matches!(dec, Decoration::Band { .. }) {
            render_decoration(dec, &raw, theme, defs, out);
        }
    }

    for label in &edge.labels {
        let p = anchor_point(&label.at, &raw);
        let open = !label.attrs.is_empty();
        if open {
            out.push_str(&format!("<g{}>\n", attrs_str(&label.attrs)));
        }
        if let Some(chip) = &label.chip {
            let font = label.text.font(&scene.font);
            let lines = run_lines(&label.text, &font, measurer);
            let mut b = Bounds::default();
            let y0 = first_line_y(&label.text, &font, p, lines.len());
            let lh = line_height(&label.text, &font);
            for (i, line) in lines.iter().enumerate() {
                let m = measurer.measure(line, &font);
                b.add(text_box(
                    &m,
                    p.x + label.text.dx,
                    y0 + i as f64 * lh,
                    &label.text,
                ));
            }
            if let Some(r) = b.rect {
                let r = r.inflate(chip.padding);
                out.push_str(&format!(
                    r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{rx}" fill="{fill}"{stroke}/>
"#,
                    x = fmt(r.x),
                    y = fmt(r.y),
                    w = fmt(r.w),
                    h = fmt(r.h),
                    rx = fmt(chip.corner_radius),
                    fill = xml_escape(&chip.fill),
                    stroke = chip
                        .stroke
                        .as_ref()
                        .map(|s| stroke_attrs(s, defs))
                        .unwrap_or_default(),
                ));
            }
        }
        render_text(&label.text, p, scene, theme, measurer, out);
        if open {
            out.push_str("</g>\n");
        }
    }

    if wrap {
        out.push_str("</g>\n");
    }
}

fn edge_bounds(edge: &SceneEdge, scene: &Scene, measurer: &dyn TextMeasurer, bounds: &mut Bounds) {
    if edge.points.is_empty() {
        return;
    }
    let half = edge.stroke.width / 2.0;
    for p in &edge.points {
        bounds.add(Rect::new(p.x - half, p.y - half, 2.0 * half, 2.0 * half));
    }
    let raw = pts_of(&edge.points);
    if let (Some(first), Some(last)) = (edge.points.first(), edge.points.last()) {
        let s = marker_extent(&edge.marker_start, scene, edge.stroke.width);
        bounds.add_circle(*first, s);
        let e = marker_extent(&edge.marker_end, scene, edge.stroke.width);
        bounds.add_circle(*last, e);
    }
    for dec in &edge.decorations {
        match dec {
            Decoration::Band { points, stroke, .. } => {
                let h = stroke.width / 2.0;
                for p in points {
                    bounds.add(Rect::new(p.x - h, p.y - h, 2.0 * h, 2.0 * h));
                }
            }
            Decoration::Dot {
                at, radius, stroke, ..
            } => {
                let extra = stroke.as_ref().map(|s| s.width / 2.0).unwrap_or(0.0);
                bounds.add_circle(anchor_point(at, &raw), radius + extra);
            }
            Decoration::Tick { at, length, .. } => {
                bounds.add_circle(anchor_point(at, &raw), length / 2.0);
            }
            Decoration::Glyph { at, size, .. } => {
                bounds.add(Rect::from_center(anchor_point(at, &raw), *size));
            }
        }
    }
    for label in &edge.labels {
        let p = anchor_point(&label.at, &raw);
        let mut b = Bounds::default();
        text_bounds(&label.text, p, scene, measurer, &mut b);
        if let Some(r) = b.rect {
            let pad = label.chip.as_ref().map(|c| c.padding).unwrap_or(0.0);
            bounds.add(r.inflate(pad));
        }
    }
}

// -- panels ------------------------------------------------------------------------------------

/// One laid-out panel line: where its baseline sits and how wide the whole line is.
struct PanelLine {
    baseline: f64,
    width: f64,
}

fn panel_lines(panel: &Panel, scene: &Scene, measurer: &dyn TextMeasurer) -> (Vec<PanelLine>, f64) {
    let style = &panel.style;
    let mut lines = Vec::new();
    let mut cy = panel.origin.y;
    for section in &panel.sections {
        if let Some(title) = &section.title {
            let font = FontSpec {
                size: style.title_size,
                weight: style.title_weight,
                ..scene.font.clone()
            };
            lines.push(PanelLine {
                baseline: cy,
                width: measurer.measure(title, &font).width,
            });
            cy += style.line_height;
        }
        for item in &section.items {
            let font = FontSpec {
                size: style.item_size,
                weight: 400.0,
                ..scene.font.clone()
            };
            let advance = item.swatch_advance.unwrap_or(style.swatch_column);
            lines.push(PanelLine {
                baseline: cy,
                width: advance + measurer.measure(&item.text, &font).width,
            });
            cy += style.line_height;
        }
        cy += style.section_gap;
    }
    (lines, cy)
}

fn render_panel(
    panel: &Panel,
    scene: &Scene,
    theme: &Theme,
    measurer: &dyn TextMeasurer,
    defs: &mut Defs,
    out: &mut String,
) {
    let style = &panel.style;
    let text_color = style
        .text_color
        .clone()
        .unwrap_or_else(|| theme.palette.node_text.clone());
    let open_panel = !panel.attrs.is_empty();
    if open_panel {
        out.push_str(&format!("<g{}>\n", attrs_str(&panel.attrs)));
    }
    let mut cy = panel.origin.y;
    for section in &panel.sections {
        let open = !section.attrs.is_empty();
        if open {
            out.push_str(&format!("<g{}>\n", attrs_str(&section.attrs)));
        }
        if let Some(title) = &section.title {
            let run = TextRun {
                text: title.clone(),
                size: Some(style.title_size),
                weight: Some(style.title_weight),
                color: Some(text_color.clone()),
                anchor: TextAnchor::Start,
                baseline: Baseline::Alphabetic,
                ..TextRun::default()
            };
            render_text(
                &run,
                Point::new(panel.origin.x, cy),
                scene,
                theme,
                measurer,
                out,
            );
            cy += style.line_height;
        }
        for item in &section.items {
            let advance = item.swatch_advance.unwrap_or(style.swatch_column);
            let box_ = Rect::new(
                panel.origin.x,
                cy + style.swatch_dy - style.swatch_height / 2.0,
                style.swatch_height,
                style.swatch_height,
            );
            let open_item = !item.attrs.is_empty();
            if open_item {
                out.push_str(&format!("<g{}>\n", attrs_str(&item.attrs)));
            }
            match &item.swatch {
                Swatch::None => {}
                Swatch::Color { color } => out.push_str(&format!(
                    r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{rx}" fill="{col}"/>
"#,
                    x = fmt(box_.x),
                    y = fmt(box_.y),
                    w = fmt(box_.w),
                    h = fmt(box_.h),
                    rx = fmt(style.swatch_corner_radius),
                    col = xml_escape(color),
                )),
                Swatch::Symbol { id, size, color } => {
                    defs.use_symbol(id);
                    let color = color
                        .clone()
                        .unwrap_or_else(|| theme.palette.arc_color.clone());
                    out.push_str(&use_element(
                        id,
                        Rect::new(
                            panel.origin.x,
                            cy + style.swatch_dy - size.h / 2.0,
                            size.w,
                            size.h,
                        ),
                        Some(&color),
                        &Attrs::default(),
                    ));
                }
                Swatch::Raw(raw) => raw_element(raw, out),
            }
            let run = TextRun {
                text: item.text.clone(),
                dx: advance,
                size: Some(style.item_size),
                weight: Some(400.0),
                color: Some(text_color.clone()),
                anchor: TextAnchor::Start,
                baseline: Baseline::Alphabetic,
                ..TextRun::default()
            };
            render_text(
                &run,
                Point::new(panel.origin.x, cy),
                scene,
                theme,
                measurer,
                out,
            );
            if open_item {
                out.push_str("</g>\n");
            }
            cy += style.line_height;
        }
        cy += style.section_gap;
        if open {
            out.push_str("</g>\n");
        }
    }
    if open_panel {
        out.push_str("</g>\n");
    }
}

fn panel_bounds(panel: &Panel, scene: &Scene, measurer: &dyn TextMeasurer, bounds: &mut Bounds) {
    let (lines, end_y) = panel_lines(panel, scene, measurer);
    let width = lines.iter().map(|l| l.width).fold(0.0_f64, f64::max);
    let top = lines
        .first()
        .map(|l| l.baseline - panel.style.title_size)
        .unwrap_or(panel.origin.y);
    if width <= 0.0 && lines.is_empty() {
        return;
    }
    bounds.add(Rect::new(
        panel.origin.x,
        top,
        width.max(panel.style.swatch_column),
        (end_y - top).max(0.0),
    ));
}

// -- the scene ---------------------------------------------------------------------------------

impl Scene {
    /// The scene's bounding box, before [`Scene::padding`].
    ///
    /// Covers node outlines and edge polylines (each inflated by half its stroke), marker extents,
    /// every measured text run and edge label, declared [`RawSvg::bounds`] and overlay panels, so
    /// an export never clips what it drew.
    pub fn bounds(&self, measurer: &dyn TextMeasurer) -> Rect {
        let mut bounds = Bounds::default();
        for node in &self.nodes {
            node_bounds(node, self, measurer, 1.0, &mut bounds);
        }
        for edge in &self.edges {
            edge_bounds(edge, self, measurer, &mut bounds);
        }
        for overlay in &self.overlays {
            match &overlay.content {
                OverlayContent::Raw(raw) => bounds.add(raw.bounds),
                OverlayContent::Panel(panel) => panel_bounds(panel, self, measurer, &mut bounds),
            }
        }
        bounds.rect.unwrap_or(Rect::new(0.0, 0.0, 100.0, 100.0))
    }
}

/// Draw a [`Scene`] to a standalone SVG document.
///
/// The output is deterministic. The same scene, theme and measurer produce the same bytes on
/// every platform.
pub fn render(scene: &Scene, theme: &Theme, measurer: &dyn TextMeasurer) -> String {
    let view = scene.bounds(measurer).inflate(scene.padding);
    let mut defs = Defs::default();

    let mut edges_svg = String::new();
    let verticals = vertical_runs(&scene.edges);
    for (i, edge) in scene.edges.iter().enumerate() {
        render_edge(
            edge,
            i,
            &verticals,
            scene,
            theme,
            measurer,
            &mut defs,
            &mut edges_svg,
        );
    }
    let mut nodes_svg = String::new();
    for node in &scene.nodes {
        render_node(node, scene, theme, measurer, &mut defs, &mut nodes_svg);
    }
    let mut under_svg = String::new();
    let mut over_svg = String::new();
    for overlay in &scene.overlays {
        let target = match overlay.z {
            ZOrder::Under => &mut under_svg,
            ZOrder::Over => &mut over_svg,
        };
        let open = !overlay.attrs.is_empty();
        if open {
            target.push_str(&format!("<g{}>\n", attrs_str(&overlay.attrs)));
        }
        match &overlay.content {
            OverlayContent::Raw(raw) => raw_element(raw, target),
            OverlayContent::Panel(panel) => {
                render_panel(panel, scene, theme, measurer, &mut defs, target)
            }
        }
        if open {
            target.push_str("</g>\n");
        }
    }

    // Defs are assembled last because drawing discovers which markers, gradients and symbols are
    // referenced.
    let mut defs_svg = String::new();
    for raw in &scene.defs {
        defs_svg.push_str(&raw.markup);
        if !defs_svg.ends_with('\n') {
            defs_svg.push('\n');
        }
    }
    let mut emitted: BTreeSet<&str> = BTreeSet::new();
    for symbol in &scene.symbols {
        if emitted.insert(symbol.id.as_str()) {
            defs_svg.push_str(&symbol_element(symbol));
        }
    }
    for symbol in builtin_symbols() {
        if defs.used_symbols.contains(&symbol.id) && !emitted.contains(symbol.id.as_str()) {
            defs_svg.push_str(&symbol_element(&symbol));
        }
    }
    for marker in &scene.markers {
        defs_svg.push_str(&marker_element(marker));
    }
    for (_, body) in &defs.markers {
        defs_svg.push_str(body);
    }
    for (_, body) in &defs.gradients {
        defs_svg.push_str(body);
    }

    let (first_id, first, second_id, second) = match scene.order {
        DrawOrder::EdgesUnderNodes => ("edges", &edges_svg, "nodes", &nodes_svg),
        DrawOrder::EdgesOverNodes => ("nodes", &nodes_svg, "edges", &edges_svg),
    };

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{vx} {vy} {w} {h}" width="{w}" height="{h}" font-family="{font}""#,
        vx = fmt(view.x),
        vy = fmt(view.y),
        w = fmt(view.w),
        h = fmt(view.h),
        font = xml_escape(&scene.font.family),
    ));
    out.push_str(&attrs_str(&scene.attrs));
    out.push_str(">\n");
    out.push_str("<defs>\n");
    out.push_str(&defs_svg);
    out.push_str("</defs>\n");
    if let Some(bg) = &scene.background {
        out.push_str(&format!(
            r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" fill="{bg}"/>
"#,
            x = fmt(view.x),
            y = fmt(view.y),
            w = fmt(view.w),
            h = fmt(view.h),
            bg = xml_escape(bg),
        ));
    }
    if !under_svg.is_empty() {
        out.push_str("<g id=\"overlays-under\">\n");
        out.push_str(&under_svg);
        out.push_str("</g>\n");
    }
    out.push_str(&format!("<g id=\"{first_id}\">\n"));
    out.push_str(first);
    out.push_str("</g>\n");
    out.push_str(&format!("<g id=\"{second_id}\">\n"));
    out.push_str(second);
    out.push_str("</g>\n");
    if !over_svg.is_empty() {
        out.push_str("<g id=\"overlays\">\n");
        out.push_str(&over_svg);
        out.push_str("</g>\n");
    }
    out.push_str("</svg>");
    out
}

fn symbol_element(symbol: &SymbolDef) -> String {
    format!(
        r#"<symbol id="{id}" viewBox="{x} {y} {w} {h}" overflow="visible"{attrs}>
{markup}</symbol>
"#,
        id = xml_escape(&symbol.id),
        x = fmt(symbol.view_box.x),
        y = fmt(symbol.view_box.y),
        w = fmt(symbol.view_box.w),
        h = fmt(symbol.view_box.h),
        attrs = attrs_str(&symbol.attrs),
        markup = symbol.markup,
    )
}

fn marker_element(marker: &MarkerDef) -> String {
    format!(
        r#"<marker id="{id}" markerWidth="{mw}" markerHeight="{mh}" viewBox="{vx} {vy} {vw} {vh}" orient="{orient}" refX="{rx}" refY="{ry}" markerUnits="userSpaceOnUse"{attrs}>{markup}</marker>
"#,
        id = xml_escape(&marker.id),
        mw = fmt(marker.size.w),
        mh = fmt(marker.size.h),
        vx = fmt(marker.view_box.x),
        vy = fmt(marker.view_box.y),
        vw = fmt(marker.view_box.w),
        vh = fmt(marker.view_box.h),
        orient = if marker.orient_auto { "auto" } else { "0" },
        rx = fmt(marker.ref_point.x),
        ry = fmt(marker.ref_point.y),
        attrs = attrs_str(&marker.attrs),
        markup = marker.markup,
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::svg::text::HeuristicMeasurer;

    fn measurer() -> HeuristicMeasurer {
        HeuristicMeasurer::new()
    }

    fn draw(scene: &Scene) -> String {
        render(scene, &Theme::default(), &measurer())
    }

    fn node_at(x: f64, y: f64, w: f64, h: f64) -> SceneNode {
        SceneNode {
            rect: Rect::new(x, y, w, h),
            ..SceneNode::default()
        }
    }

    // -- escaping and attribute safety ---------------------------------------------------------

    #[test]
    fn escaping_covers_every_metacharacter() {
        assert_eq!(
            xml_escape("a & b < c > d \"e\" f 'g'"),
            "a &amp; b &lt; c &gt; d &quot;e&quot; f &#39;g&#39;"
        );
        // Ampersand first, or the escapes escape each other.
        assert_eq!(xml_escape("&lt;"), "&amp;lt;");
    }

    #[test]
    fn text_and_attribute_values_are_escaped() {
        let scene = Scene {
            nodes: vec![SceneNode {
                content: vec![NodeContent::Text(TextRun::new(
                    "</text><script>alert('x' & \"y\")</script>",
                ))],
                attrs: Attrs::default()
                    .id("a\"b")
                    .data("kind", "<danger>")
                    .style("fill:red;content:'q'"),
                ..node_at(0.0, 0.0, 100.0, 40.0)
            }],
            ..Scene::default()
        };
        let svg = draw(&scene);
        assert!(!svg.contains("<script>"), "{svg}");
        assert!(svg.contains("&lt;/text&gt;&lt;script&gt;"), "{svg}");
        assert!(svg.contains(r#"id="a&quot;b""#), "{svg}");
        assert!(svg.contains(r#"data-kind="&lt;danger&gt;""#), "{svg}");
        assert!(svg.contains("content:&#39;q&#39;"), "{svg}");
    }

    /// A run takes the scene font's size and weight unless it names its own, and the scene's
    /// italic flag reaches the drawing.
    #[test]
    fn the_scene_font_is_every_runs_default() {
        let scene = Scene {
            font: FontSpec {
                size: 20.0,
                weight: 700.0,
                italic: true,
                ..FontSpec::default()
            },
            nodes: vec![SceneNode {
                content: vec![
                    NodeContent::Text(TextRun::new("plain")),
                    NodeContent::Text(TextRun::new("own").size(9.0).weight(300.0)),
                ],
                ..node_at(0.0, 0.0, 100.0, 60.0)
            }],
            ..Scene::default()
        };
        let svg = draw(&scene);
        assert!(
            svg.contains(r#"font-size="20.00" font-weight="700.00""#),
            "{svg}"
        );
        assert!(
            svg.contains(r#"font-size="9.00" font-weight="300.00""#),
            "{svg}"
        );
        assert_eq!(svg.matches(r#"font-style="italic""#).count(), 2, "{svg}");
    }

    /// A ring inside a circle follows the circle, which is `w` across whatever the height.
    #[test]
    fn a_ring_follows_a_circle_of_any_box() {
        let scene = Scene {
            nodes: vec![SceneNode {
                shape: Shape::Circle,
                content: vec![NodeContent::Rings {
                    rings: vec![Ring {
                        color: "#000".into(),
                        width: 2.0,
                        inset: 2.0,
                    }],
                }],
                ..node_at(0.0, 0.0, 40.0, 20.0)
            }],
            ..Scene::default()
        };
        let svg = draw(&scene);
        // Outline r = 20. Ring r = 20 - (2 + 2 / 2) = 17.
        assert!(
            svg.contains(r#"r="20.00""#) && svg.contains(r#"r="17.00""#),
            "{svg}"
        );
    }

    /// A panel's own attributes wrap the panel.
    #[test]
    fn panel_attributes_are_emitted() {
        let scene = Scene {
            overlays: vec![Overlay::panel(Panel {
                attrs: Attrs::default().id("legend"),
                ..Panel::default()
            })],
            ..Scene::default()
        };
        assert!(draw(&scene).contains(r#"<g id="legend">"#));
    }

    #[test]
    fn attribute_names_are_validated_not_escaped() {
        assert!(is_valid_attr_name("data-x"));
        assert!(is_valid_attr_name("_x:y.z-1"));
        assert!(!is_valid_attr_name(""));
        assert!(!is_valid_attr_name("1x"));
        assert!(!is_valid_attr_name("a b"));
        assert!(!is_valid_attr_name("a\"onload=alert(1) b"));
        assert!(!is_valid_attr_name("-x"));

        let scene = Scene {
            nodes: vec![SceneNode {
                // A name that would close the attribute and start a new one is dropped whole.
                // Escaping it would leave an attribute nobody can address.
                attrs: Attrs::default()
                    .attr("x\" onload=\"alert(1)", "boom")
                    .attr("1bad", "boom")
                    .data("ok name", "boom")
                    .attr("data-fine", "kept"),
                ..node_at(0.0, 0.0, 10.0, 10.0)
            }],
            ..Scene::default()
        };
        let svg = draw(&scene);
        assert!(!svg.contains("onload"), "{svg}");
        assert!(!svg.contains("boom"), "{svg}");
        assert!(svg.contains(r#"data-fine="kept""#), "{svg}");
    }

    /// A horizontal run crossing another edge's vertical run hops over it. A run that only meets
    /// the other at its end (a T) does not, and with hops off nothing hops.
    #[test]
    fn horizontal_runs_hop_over_crossing_vertical_runs() {
        let edge = |pts: &[(f64, f64)], hop: f64| SceneEdge {
            points: pts.iter().map(|&(x, y)| Point::new(x, y)).collect(),
            hop_radius: hop,
            ..SceneEdge::default()
        };
        let scene = |hop: f64| Scene {
            edges: vec![
                edge(&[(0.0, 50.0), (100.0, 50.0)], hop),
                edge(&[(50.0, 0.0), (50.0, 100.0)], hop),
                // Ends on the horizontal run, so it forms a T and draws flat.
                edge(&[(80.0, 0.0), (80.0, 50.0)], hop),
            ],
            ..Scene::default()
        };
        let svg = draw(&scene(4.0));
        assert_eq!(svg.matches(" A 4.00,4.00 ").count(), 1, "{svg}");
        assert!(
            svg.contains("L 46.00,50.00 A 4.00,4.00 0 0 1 54.00,50.00"),
            "{svg}"
        );
        assert!(!draw(&scene(0.0)).contains(" A "));
    }

    // -- escape hatches ------------------------------------------------------------------------

    #[test]
    fn raw_fragments_are_emitted_verbatim_exactly_once() {
        let markup = "<circle cx=\"1\" cy=\"2\" r=\"3\" fill=\"#abc\"/><!-- vouched for -->";
        let scene = Scene {
            defs: vec![RawSvg::new(
                "<filter id=\"blur\"><feGaussianBlur stdDeviation=\"2\"/></filter>",
                Rect::default(),
            )],
            nodes: vec![SceneNode {
                content: vec![NodeContent::Raw(RawSvg::new(
                    markup,
                    Rect::new(0.0, 0.0, 4.0, 4.0),
                ))],
                ..node_at(0.0, 0.0, 10.0, 10.0)
            }],
            overlays: vec![Overlay::raw(RawSvg::new(
                "<text x=\"0\" y=\"0\">note &amp; co</text>",
                Rect::new(0.0, 0.0, 30.0, 10.0),
            ))],
            ..Scene::default()
        };
        let svg = draw(&scene);
        assert_eq!(svg.matches(markup).count(), 1, "{svg}");
        assert!(
            svg.contains("<feGaussianBlur stdDeviation=\"2\"/>"),
            "{svg}"
        );
        assert!(svg.contains("note &amp; co"), "{svg}");
    }

    #[test]
    fn symbols_are_defined_once_and_referenced_by_use() {
        let symbol = SymbolDef::new(
            "star",
            Rect::new(0.0, 0.0, 10.0, 10.0),
            "<polygon points=\"5,0 6,4 10,4 7,6 8,10 5,7 2,10 3,6 0,4 4,4\" fill=\"currentColor\"/>",
        );
        let use_content = |x: f64| SceneNode {
            content: vec![NodeContent::Use {
                symbol: "star".into(),
                at: Point::new(0.0, 0.0),
                size: Size::new(8.0, 8.0),
                color: Some("#f00".into()),
                attrs: Attrs::default(),
            }],
            ..node_at(x, 0.0, 20.0, 20.0)
        };
        let scene = Scene {
            symbols: vec![symbol],
            nodes: vec![use_content(0.0), use_content(40.0)],
            ..Scene::default()
        };
        let svg = draw(&scene);
        assert_eq!(svg.matches("<symbol id=\"star\"").count(), 1, "{svg}");
        assert_eq!(svg.matches("<use href=\"#star\"").count(), 2, "{svg}");
        assert!(svg.contains("color=\"#f00\""), "{svg}");
    }

    #[test]
    fn builtin_symbols_are_only_emitted_when_referenced() {
        let plain = Scene {
            nodes: vec![node_at(0.0, 0.0, 10.0, 10.0)],
            ..Scene::default()
        };
        assert!(!draw(&plain).contains("<symbol"));

        let using = Scene {
            nodes: vec![SceneNode {
                content: vec![NodeContent::Use {
                    symbol: SYMBOL_PLAY_TRIANGLE.into(),
                    at: Point::default(),
                    size: Size::new(6.0, 6.0),
                    color: None,
                    attrs: Attrs::default(),
                }],
                ..node_at(0.0, 0.0, 10.0, 10.0)
            }],
            ..Scene::default()
        };
        let svg = draw(&using);
        assert_eq!(
            svg.matches(&format!("<symbol id=\"{SYMBOL_PLAY_TRIANGLE}\""))
                .count(),
            1,
            "{svg}"
        );
    }

    #[test]
    fn a_custom_marker_is_defined_and_referenced() {
        let scene = Scene {
            markers: vec![MarkerDef {
                id: "diamond".into(),
                markup: "<path d=\"M 0,6 L 6,0 L 12,6 L 6,12 Z\" fill=\"#123456\"/>".into(),
                tip_gap: 6.0,
                ..MarkerDef::default()
            }],
            edges: vec![SceneEdge {
                points: vec![Point::new(0.0, 0.0), Point::new(100.0, 0.0)],
                stroke: Stroke::new("#000", 2.0),
                marker_end: MarkerRef::Custom {
                    id: "diamond".into(),
                },
                ..SceneEdge::default()
            }],
            ..Scene::default()
        };
        let svg = draw(&scene);
        assert!(svg.contains("<marker id=\"diamond\""), "{svg}");
        assert!(svg.contains("marker-end=\"url(#diamond)\""), "{svg}");
        // The declared tip gap pulls the path back off the endpoint.
        assert!(svg.contains("L 94.00,0.00"), "{svg}");
    }

    #[test]
    fn an_unknown_custom_marker_is_skipped_rather_than_dangling() {
        let scene = Scene {
            edges: vec![SceneEdge {
                points: vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0)],
                marker_end: MarkerRef::Custom { id: "nope".into() },
                ..SceneEdge::default()
            }],
            ..Scene::default()
        };
        let svg = draw(&scene);
        assert!(!svg.contains("marker-end"), "{svg}");
    }

    #[test]
    fn builtin_markers_are_shared_between_edges_that_ask_for_the_same_one() {
        let edge = |y: f64| SceneEdge {
            points: vec![Point::new(0.0, y), Point::new(50.0, y)],
            stroke: Stroke::new("#000", 2.0),
            marker_end: MarkerRef::builtin(BuiltinMarker::Arrow),
            ..SceneEdge::default()
        };
        let scene = Scene {
            edges: vec![edge(0.0), edge(20.0)],
            ..Scene::default()
        };
        let svg = draw(&scene);
        assert_eq!(svg.matches("<marker ").count(), 1, "{svg}");
        assert_eq!(
            svg.matches("marker-end=\"url(#gl-mk-0)\"").count(),
            2,
            "{svg}"
        );
    }

    // -- bounds --------------------------------------------------------------------------------

    #[test]
    fn bounds_include_a_wide_label_and_a_legend() {
        let m = measurer();
        let node = SceneNode {
            content: vec![NodeContent::Text(TextRun::new(
                "a label far wider than the node it is centred on",
            ))],
            ..node_at(0.0, 0.0, 20.0, 20.0)
        };
        let bare = Scene {
            nodes: vec![node.clone()],
            ..Scene::default()
        };
        let b = bare.bounds(&m);
        assert!(b.w > 100.0, "the label must widen the box: {b:?}");
        assert!(b.x < 0.0 && b.max_x() > 20.0, "centred both ways: {b:?}");

        let with_legend = Scene {
            nodes: vec![node],
            overlays: vec![Overlay::panel(Panel {
                origin: Point::new(400.0, 0.0),
                sections: vec![PanelSection {
                    title: Some("Object types".into()),
                    items: vec![LegendItem {
                        swatch: Swatch::Color {
                            color: "#3b82f6".into(),
                        },
                        text: "orders".into(),
                        ..LegendItem::default()
                    }],
                    ..PanelSection::default()
                }],
                ..Panel::default()
            })],
            ..Scene::default()
        };
        let l = with_legend.bounds(&m);
        assert!(
            l.max_x() > 420.0,
            "the legend panel must be inside the box: {l:?}"
        );
        assert!(l.max_y() > b.max_y(), "and so must its height: {l:?}");
    }

    #[test]
    fn bounds_inflate_edges_by_half_their_stroke_and_include_markers() {
        let m = measurer();
        let scene = Scene {
            edges: vec![SceneEdge {
                points: vec![Point::new(0.0, 0.0), Point::new(100.0, 0.0)],
                stroke: Stroke::new("#000", 10.0),
                marker_end: MarkerRef::builtin(BuiltinMarker::Arrow),
                ..SceneEdge::default()
            }],
            ..Scene::default()
        };
        let b = scene.bounds(&m);
        assert!(b.y <= -5.0, "half the stroke: {b:?}");
        assert!(b.max_x() >= 125.0, "the arrowhead's own box: {b:?}");
    }

    #[test]
    fn an_empty_scene_falls_back_to_a_fixed_box() {
        let b = Scene::default().bounds(&measurer());
        assert_eq!(b, Rect::new(0.0, 0.0, 100.0, 100.0));
    }

    // -- paths ---------------------------------------------------------------------------------

    #[test]
    fn a_two_point_path_is_a_straight_line() {
        let d = rounded_path(&[(0.0, 0.0), (10.0, 0.0)], 8.0);
        assert!(d.starts_with("M "));
        assert!(d.contains(" L "));
        assert!(!d.contains(" Q "), "nothing to round: {d}");
    }

    #[test]
    fn a_three_point_path_rounds_its_corner() {
        let d = rounded_path(&[(0.0, 0.0), (0.0, 50.0), (50.0, 50.0)], 8.0);
        assert!(d.contains(" Q "), "{d}");
        assert_eq!(
            path_d(&[(0.0, 0.0), (0.0, 50.0), (50.0, 50.0)], 0.0)
                .matches(" Q ")
                .count(),
            0
        );
    }

    #[test]
    fn a_collinear_point_is_no_corner() {
        let d = rounded_path(&[(0.0, 0.0), (25.0, 0.0), (50.0, 0.0), (50.0, 50.0)], 8.0);
        assert_eq!(
            d,
            "M 0.00,0.00 L 42.00,0.00 Q 50.00,0.00 50.00,8.00 L 50.00,50.00"
        );
        let hopping = hopping_path(
            &[(0.0, 0.0), (50.0, 0.0), (100.0, 0.0), (100.0, 50.0)],
            8.0,
            &[(0, 25.0)],
            4.0,
        );
        assert_eq!(hopping.matches(" Q ").count(), 1, "{hopping}");
        assert_eq!(hopping.matches(" A ").count(), 1, "{hopping}");
    }

    /// Two corners on a leg of twice the radius meet, with no zero-length line between them.
    #[test]
    fn meeting_corners_share_their_point() {
        let d = rounded_path(&[(0.0, 0.0), (50.0, 0.0), (50.0, 16.0), (100.0, 16.0)], 8.0);
        assert_eq!(
            d,
            "M 0.00,0.00 L 42.00,0.00 Q 50.00,0.00 50.00,8.00 Q 50.00,16.00 58.00,16.00 L 100.00,16.00"
        );
    }

    // -- glyph rows ----------------------------------------------------------------------------

    #[test]
    fn a_fit_row_collapses_to_a_numeral_when_the_glyphs_cannot_fit() {
        let row = GlyphRow {
            clusters: vec![GlyphCluster {
                count: 50,
                ..GlyphCluster::default()
            }],
            layout: GlyphLayout::Fit(FitLayout {
                max_width: 40.0,
                numeral: Some(NumeralStyle {
                    size: 20.0,
                    weight: 600.0,
                    color: None,
                }),
                ..FitLayout::default()
            }),
            ..GlyphRow::default()
        };
        let plan = plan_glyph_row(&row, Point::new(0.0, 0.0));
        assert!(plan.glyphs.is_empty());
        assert_eq!(plan.numeral.map(|n| n.total), Some(50));
    }

    #[test]
    fn a_separator_binds_its_cluster_to_the_one_before_it() {
        let dot = |sep: bool| GlyphCluster {
            count: 1,
            radius: 4.0,
            separator: sep.then(|| Separator {
                symbol: SYMBOL_MAPSTO.into(),
                width: 9.0,
                height: 8.0,
                gap_before: 2.0,
                gap_after: 1.0,
                color: None,
            }),
            ..GlyphCluster::default()
        };
        let row = GlyphRow {
            clusters: vec![dot(false), dot(false), dot(true)],
            layout: GlyphLayout::Exact(ExactLayout {
                wrap_width: 30.0,
                ..ExactLayout::default()
            }),
            ..GlyphRow::default()
        };
        let plan = plan_glyph_row(&row, Point::new(0.0, 0.0));
        let y_of = |cluster: usize| {
            plan.glyphs
                .iter()
                .find(|g| g.cluster == cluster)
                .map(|g| g.center.y)
        };
        assert_eq!(y_of(1), y_of(2), "the bound pair shares a row");
        assert_ne!(y_of(0), y_of(1), "and the row before it wrapped");
        assert_eq!(plan.separators.len(), 1);
    }

    // -- whole documents -----------------------------------------------------------------------

    #[test]
    fn rendering_is_deterministic() {
        let scene = kitchen_sink();
        let a = draw(&scene);
        let b = draw(&scene.clone());
        assert_eq!(a, b, "the same scene must render to the same bytes");
    }

    /// A scene using every content type, every decoration and both escape hatches.
    fn kitchen_sink() -> Scene {
        Scene {
            background: Some("#ffffff".into()),
            padding: 16.0,
            defs: vec![RawSvg::new(
                "<filter id=\"soft\"><feGaussianBlur stdDeviation=\"1\"/></filter>",
                Rect::default(),
            )],
            symbols: vec![SymbolDef::new(
                "tick",
                Rect::new(0.0, 0.0, 10.0, 10.0),
                "<path d=\"M 1,5 L 4,8 L 9,2\" fill=\"none\" stroke=\"currentColor\"/>",
            )],
            markers: vec![MarkerDef {
                id: "diamond".into(),
                markup: "<path d=\"M 0,6 L 6,0 L 12,6 L 6,12 Z\" fill=\"#123456\"/>".into(),
                tip_gap: 6.0,
                ..MarkerDef::default()
            }],
            nodes: vec![
                SceneNode {
                    rect: Rect::new(0.0, 0.0, 60.0, 60.0),
                    shape: Shape::Circle,
                    fill: Some("#eef".into()),
                    stroke: Some(Stroke::new("#334", 2.0).dashed("4 2")),
                    content: vec![
                        NodeContent::Rings { rings: vec![Ring {
                            color: "#f59e0b".into(),
                            width: 2.0,
                            inset: 3.0,
                        }] },
                        NodeContent::Glyphs(GlyphRow {
                            clusters: vec![GlyphCluster {
                                count: 3,
                                ..GlyphCluster::default()
                            }],
                            layout: GlyphLayout::Fit(FitLayout {
                                max_width: 50.0,
                                numeral: Some(NumeralStyle {
                                    size: 24.0,
                                    weight: 600.0,
                                    color: None,
                                }),
                                ..FitLayout::default()
                            }),
                            ..GlyphRow::default()
                        }),
                    ],
                    attrs: Attrs::default().id("p1").class("place"),
                },
                SceneNode {
                    rect: Rect::new(120.0, 0.0, 120.0, 60.0),
                    shape: Shape::Rect { corner_radius: 6.0 },
                    content: vec![
                        NodeContent::Text(TextRun {
                            text: "Confirm Customer Order Details & Pay".into(),
                            dy: -8.0,
                            wrap: Some(TextWrap {
                                max_width: 108.0,
                                max_lines: 2,
                                line_height: 14.0,
                            }),
                            ..TextRun::default()
                        }),
                        NodeContent::Use {
                            symbol: "tick".into(),
                            at: Point::new(40.0, 16.0),
                            size: Size::new(10.0, 10.0),
                            color: Some("#16a34a".into()),
                            attrs: Attrs::default(),
                        },
                        NodeContent::Glyphs(GlyphRow {
                            clusters: vec![
                                GlyphCluster {
                                    glyph: Glyph::Square {
                                        scale: 0.9,
                                        corner_radius: 2.0,
                                        opacity: 0.4,
                                    },
                                    count: 1,
                                    ..GlyphCluster::default()
                                },
                                GlyphCluster {
                                    glyph: Glyph::Symbol {
                                        id: SYMBOL_SWATCH_DOT.into(),
                                    },
                                    count: 2,
                                    outline: Some(Stroke::new("#334", 1.0).dashed("2 1.4")),
                                    separator: Some(Separator {
                                        symbol: SYMBOL_MAPSTO.into(),
                                        width: 9.0,
                                        height: 8.0,
                                        gap_before: 2.0,
                                        gap_after: 1.0,
                                        color: None,
                                    }),
                                    ..GlyphCluster::default()
                                },
                            ],
                            dy: 16.0,
                            layout: GlyphLayout::Exact(ExactLayout {
                                wrap_width: 112.0,
                                max_bottom: Some(60.0),
                                ..ExactLayout::default()
                            }),
                            ..GlyphRow::default()
                        }),
                        NodeContent::Raw(RawSvg::new(
                            "<circle cx=\"230\" cy=\"10\" r=\"3\" fill=\"#dc2626\"/>",
                            Rect::new(227.0, 7.0, 6.0, 6.0),
                        )),
                    ],
                    ..SceneNode::default()
                },
                SceneNode {
                    rect: Rect::new(0.0, 90.0, 80.0, 34.0),
                    shape: Shape::Ellipse,
                    fill: Some("none".into()),
                    content: vec![
                        NodeContent::Rings { rings: vec![Ring {
                            color: "#94a3b8".into(),
                            width: 1.0,
                            inset: 4.0,
                        }] },
                        NodeContent::Text(TextRun {
                            text: "start".into(),
                            baseline: Baseline::Alphabetic,
                            anchor: TextAnchor::Start,
                            family: Some("Georgia, serif".into()),
                            ..TextRun::default()
                        }),
                    ],
                    ..SceneNode::default()
                },
            ],
            edges: vec![SceneEdge {
                points: vec![
                    Point::new(60.0, 30.0),
                    Point::new(90.0, 30.0),
                    Point::new(90.0, 30.0),
                    Point::new(120.0, 30.0),
                ],
                stroke: Stroke {
                    paint: Paint::Gradient(Gradient {
                        from: Point::new(60.0, 30.0),
                        to: Point::new(120.0, 30.0),
                        stops: vec![
                            GradientStop {
                                offset: 0.0,
                                color: "#3b82f6".into(),
                            },
                            GradientStop {
                                offset: 1.0,
                                color: "#ef4444".into(),
                            },
                        ],
                    }),
                    width: 2.0,
                    dash: Some("6 3".into()),
                    linecap: LineCap::Round,
                    opacity: Some(0.9),
                },
                corner_radius: 4.0,
                hop_radius: 0.0,
                simplify: 3.0,
                marker_start: MarkerRef::builtin(BuiltinMarker::Ball),
                marker_end: MarkerRef::Custom { id: "diamond".into() },
                labels: vec![
                    EdgeLabel {
                        text: TextRun {
                            text: "3 <of> 5 & rising".into(),
                            size: Some(10.0),
                            weight: Some(600.0),
                            halo: Some(Halo {
                                color: "#ffffff".into(),
                                width: 2.5,
                            }),
                            ..TextRun::default()
                        },
                        at: EdgeAnchor::Fraction(0.5),
                        ..EdgeLabel::default()
                    },
                    EdgeLabel {
                        text: TextRun::new("pinned"),
                        at: EdgeAnchor::At(Point::new(90.0, 60.0)),
                        chip: Some(Chip::default()),
                        ..EdgeLabel::default()
                    },
                ],
                decorations: vec![
                    Decoration::Band {
                        points: vec![Point::new(60.0, 34.0), Point::new(120.0, 34.0)],
                        stroke: Stroke {
                            linecap: LineCap::Round,
                            opacity: Some(0.5),
                            ..Stroke::new("#93c5fd", 5.0)
                        },
                        corner_radius: 0.0,
                        attrs: Attrs::default(),
                    },
                    Decoration::Dot {
                        at: EdgeAnchor::Fraction(0.25),
                        radius: 4.33,
                        fill: Some("#ffffff".into()),
                        stroke: Some(Stroke::new("#334", 1.0)),
                        attrs: Attrs::default(),
                    },
                    Decoration::Tick {
                        at: EdgeAnchor::Fraction(0.75),
                        length: 8.0,
                        stroke: Stroke::new("#334", 1.5),
                        attrs: Attrs::default(),
                    },
                    Decoration::Glyph {
                        at: EdgeAnchor::Fraction(0.6),
                        symbol: SYMBOL_MAPSTO.into(),
                        size: Size::new(9.0, 8.0),
                        color: Some("#334".into()),
                        attrs: Attrs::default(),
                    },
                ],
                attrs: Attrs::default().class("arc").data("from", "p1"),
            }],
            overlays: vec![
                Overlay {
                    z: ZOrder::Under,
                    content: OverlayContent::Raw(RawSvg::new(
                        "<rect x=\"-10\" y=\"-10\" width=\"20\" height=\"20\" fill=\"#f8fafc\"/>",
                        Rect::new(-10.0, -10.0, 20.0, 20.0),
                    )),
                    attrs: Attrs::default().class("watermark"),
                },
                Overlay::panel(Panel {
                    origin: Point::new(280.0, 10.0),
                    sections: vec![
                        PanelSection {
                            title: Some("Object types".into()),
                            items: vec![LegendItem {
                                swatch: Swatch::Color { color: "#3b82f6".into() },
                                text: "orders".into(),
                                ..LegendItem::default()
                            }],
                            ..PanelSection::default()
                        },
                        PanelSection {
                            title: Some("Notes".into()),
                            items: vec![
                                LegendItem {
                                    swatch: Swatch::Symbol {
                                        id: SYMBOL_SWATCH_LOOP.into(),
                                        size: Size::new(10.0, 10.0),
                                        color: Some("#334".into()),
                                    },
                                    text: "may repeat".into(),
                                    ..LegendItem::default()
                                },
                                LegendItem {
                                    swatch: Swatch::Symbol {
                                        id: SYMBOL_SWATCH_MAPSTO.into(),
                                        size: Size::new(24.0, 8.0),
                                        color: Some("#334".into()),
                                    },
                                    text: "maps to".into(),
                                    swatch_advance: Some(28.0),
                                    ..LegendItem::default()
                                },
                                LegendItem {
                                    swatch: Swatch::Raw(RawSvg::new(
                                        "<rect x=\"280\" y=\"70\" width=\"10\" height=\"10\" fill=\"#a3a3a3\"/>",
                                        Rect::new(280.0, 70.0, 10.0, 10.0),
                                    )),
                                    text: "raw swatch".into(),
                                    ..LegendItem::default()
                                },
                            ],
                            ..PanelSection::default()
                        },
                    ],
                    ..Panel::default()
                }),
            ],
            order: DrawOrder::EdgesUnderNodes,
            attrs: Attrs::default().class("graphlay-scene"),
            font: FontSpec::default(),
        }
    }

    /// A scene using every content type renders to well-formed XML.
    ///
    /// The SVG is also written to the temp dir so a real XML parser can check it.
    #[test]
    fn an_example_scene_renders_to_well_formed_xml() {
        let svg = draw(&kitchen_sink());
        let path = std::env::temp_dir().join("graphlay-example-scene.svg");
        let _ = std::fs::write(&path, &svg);

        assert!(svg.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg "));
        assert!(svg.ends_with("</svg>"));
        // A cheap well-formedness check that every element opened is closed.
        let opens = svg.matches('<').count();
        let closes = svg.matches('>').count();
        assert_eq!(opens, closes, "unbalanced angle brackets\n{svg}");
        for tag in ["g", "text", "symbol", "marker", "defs", "linearGradient"] {
            let open =
                svg.matches(&format!("<{tag} ")).count() + svg.matches(&format!("<{tag}>")).count();
            let close = svg.matches(&format!("</{tag}>")).count();
            assert_eq!(open, close, "unbalanced <{tag}>\n{svg}");
        }
        // The scene exercises every element kind.
        for needle in [
            "<circle",
            "<ellipse",
            "<rect",
            "<path",
            "<text",
            "<use",
            "<line",
            "<symbol",
            "<marker",
            "<linearGradient",
            "url(#gl-grad-0)",
            "marker-start=",
            "marker-end=",
            "class=\"arc\"",
            "data-from=\"p1\"",
            "&amp; rising",
            "overlays-under",
        ] {
            assert!(svg.contains(needle), "missing {needle}\n{svg}");
        }
    }
}

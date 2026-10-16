# SVG rendering

`graphlay::svg` turns a `Scene`, a plain description of one drawing, into an SVG document. The
renderer draws everything at the coordinates the caller computed. It works out two things itself,
both through a `TextMeasurer`: where a wrapped label breaks, and how a `Panel`'s entries stack.

```text
caller geometry -> Scene -> render(&Scene, &Theme, &dyn TextMeasurer) -> String
```

| module | what lives there |
|---|---|
| `svg::model` | `Scene`, `SceneNode`, `SceneEdge`, `Attrs`, `RawSvg`, `SymbolDef`, `MarkerDef`, `Overlay` |
| `svg::render` | `render`, `Scene::bounds`, the builtin marker and symbol libraries |
| `svg::text` | `TextMeasurer`, `HeuristicMeasurer`, wrapping and ellipsis |
| `svg::theme` | `Theme` and `Palette`, the six fallback colours |
| `svg::sketch` | `sketch_scene` and `sketch`, a plain drawing of a graph and its layout |

## The scene

A `Scene` is a background and padding, a `defs`/`symbols`/`markers` prelude, `nodes`, `edges`,
`overlays`, a `DrawOrder` (edges under or over nodes) and a default `FontSpec`.

Nodes are a `Rect` plus a `Shape` (`Rect { corner_radius }`, `Ellipse` or `Circle`), a fill, a
stroke, and `content` painted inside them in list order:

* `NodeContent::Text(TextRun)` is one line, or several when `wrap` is set. Offsets are relative to
  the node's centre, and `halo` puts a background-coloured stroke under the glyphs. Non-empty
  `spans` replace `text` with one line of parts in their own colour or weight.
* `NodeContent::Glyphs(GlyphRow)` is a row of counted glyphs, e.g., tokens or multiplicities. Each
  `GlyphCluster` is `count` copies of a `Glyph` (`Dot`, `Square` or a `Symbol`). The row's
  `GlyphLayout` is `Fit`, which scales the glyphs to a width and shows a numeral when even the
  smallest do not fit (or there are more than `max_glyphs`), or `Exact`, which draws at the
  declared radius and wraps into rows. A `counted` numeral shows each cluster as its glyph and
  count, one line per cluster. A cluster's `separator` is drawn in the gap before it and keeps a
  row break from falling there.
* `NodeContent::Rings(Vec<Ring>)` are concentric insets that follow the node's shape.
* `NodeContent::Use { symbol, .. }` draws a `SymbolDef` by id, tinted through `currentColor`.
* `NodeContent::Polygon { points, fill, stroke, .. }` is a plain `<polygon>` with its points
  relative to the node's centre. For a mark drawn once, it is smaller than a `Use` and renders the
  same in every viewer.
* `NodeContent::Raw(RawSvg)` is raw markup, see below.

Edges are a polyline plus a `Stroke` (a colour or a gradient, dash, linecap), a `corner_radius`, a
`MarkerRef` at each end, `labels` and `decorations`. A `hop_radius` above zero draws a small arc
where a horizontal run crosses another edge, so a crossing does not look like a junction. Labels anchor at a fraction along the path, a
distance from a fraction (`Along`) or a point, and can carry a background `Chip` or a `Halo`.
Decorations are `Dot`, `Tick` and `Glyph` at an anchor, and `Band`, a lane painted under the
edge. The path end is pulled back so a marker's tip lands on the endpoint. Fractions are measured
along the line as drawn, rounded corners included, from the untrimmed endpoints.

Overlays are raw fragments or `Panel`s, drawn under or over the graph. A legend is a `Panel` of
`PanelSection`s of `LegendItem { swatch, text }`, where a swatch is a colour, a symbol or raw
markup.

## Bounds

`Scene::bounds(&measurer)` is the union of node outlines and edge polylines grown by half their
stroke, marker extents, every measured text run and label, declared `RawSvg::bounds` and laid-out
panels. `render` adds `Scene::padding` to get the viewBox, so an export never clips what it drew.

## Escape hatches

For output the model does not cover:

* `Attrs` on every drawable sets `id`, `class`, `data-*`, `style` and other attributes. Values are
  escaped. The renderer drops attribute names that do not match `[A-Za-z_:][A-Za-z0-9_.:-]*`, so
  neither can inject markup.
* `RawSvg { markup, bounds }` is markup the caller vouches for, emitted verbatim once, as node
  content, an overlay, a legend swatch or `Scene::defs`. The renderer does not parse the markup,
  so `bounds` tells it how much room to leave.
* `SymbolDef` with `NodeContent::Use` defines a fragment once for use anywhere. Paint it with
  `currentColor` and every use can tint it. The builtin symbols (`SYMBOL_MAPSTO`,
  `SYMBOL_SWATCH_DOT` and others) are emitted only when referenced.
* `MarkerDef` is a `<marker>` of your own, referenced with `MarkerRef::Custom(id)`, next to the
  eight builtin markers.
* `TextMeasurer` takes a canvas or a parsed font. The default `HeuristicMeasurer` uses
  per-character width classes.

## Sketch

`svg::sketch_scene(&graph, &layout, &SketchOptions)` builds a plain `Scene` from a layout: boxes
and ellipses with each node's label or key, edges coloured by group, junction dots, dashed
compound frames and swimlanes. Restyle that scene and pass it to `render`, or call `svg::sketch`
to render it with the default theme in one step. `SketchOptions` sets the labels, lane names,
background and padding.

```rust
use graphlay::{layout, svg, GraphBuilder, LayoutOptions, Size};

let mut b = GraphBuilder::new();
let a = b.add_box(Size::new(120.0, 50.0));
let c = b.add_box(Size::new(120.0, 50.0));
b.connect(a, c);
let g = b.build();
let l = layout(&g, &LayoutOptions::layered_default()).expect("a valid graph");

let mut scene = svg::sketch_scene(&g, &l, &svg::SketchOptions::default());
scene.background = Some("#f8fafc".into());
let node = &mut scene.nodes[0];
node.fill = Some("#dbeafe".into());
node.content.push(svg::NodeContent::Glyphs(svg::GlyphRow::dots(1, 5.0, "#1e3a8a")));
if let Some(label) = node.label_mut() {
    label.weight = Some(600.0);
}
let edge = &mut scene.edges[0];
edge.marker_end = svg::MarkerRef::builtin(svg::BuiltinMarker::Arrow).colored("#2563eb");
edge.labels.push(svg::EdgeLabel::new("42", l.edges[0].label_anchor).chip());
let svg = svg::render(&scene, &svg::Theme::default(), &svg::HeuristicMeasurer::new());
```

Scene node `i` is graph node `i` unless the graph has compounds, which come first. The builders
`label_mut`, `GlyphRow::dots`, `NodeContent::symbol`, `EdgeLabel::new(..).chip()`,
`MarkerRef::builtin(..).colored(..)`, `Decoration::band`, `LegendItem::new` and `Panel::legend`
cover the usual restyling. [The gallery](gallery.md#drawing-styles) has complete examples.

The JSON form takes the same steps: `json::sketch_scene` returns the scene as JSON,
`json::render_scene` draws a scene, and `json::sketch` does both.

## Example

```rust
use graphlay::svg::*;

let scene = Scene {
    background: Some("#ffffff".into()),
    padding: 16.0,
    symbols: vec![SymbolDef::new(
        "tick",
        Rect::new(0.0, 0.0, 10.0, 10.0),
        r#"<path d="M 1,5 L 4,8 L 9,2" fill="none" stroke="currentColor"/>"#,
    )],
    nodes: vec![SceneNode {
        rect: Rect::new(0.0, 0.0, 120.0, 48.0),
        shape: Shape::Rect { corner_radius: 6.0 },
        content: vec![
            NodeContent::Text(TextRun::new("Confirm order").at(0.0, -6.0)),
            NodeContent::Use {
                symbol: "tick".into(), at: Point::new(0.0, 12.0),
                size: Size::new(10.0, 10.0), color: Some("#16a34a".into()),
                attrs: Attrs::default(),
            },
        ],
        attrs: Attrs::default().id("n1").data("kind", "activity"),
        ..SceneNode::default()
    }],
    edges: vec![SceneEdge {
        points: vec![Point::new(120.0, 24.0), Point::new(220.0, 24.0)],
        stroke: Stroke::new("#374151", 2.0),
        marker_end: MarkerRef::builtin(BuiltinMarker::Arrow),
        ..SceneEdge::default()
    }],
    ..Scene::default()
};
let svg = render(&scene, &Theme::default(), &HeuristicMeasurer::new());
```

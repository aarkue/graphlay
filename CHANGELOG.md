# Changelog

## 0.4.0 (2026-11-20)

- On equal weights, the main path runs through the nodes with the most extent along the flow. On
  an unweighted Petri net, it thus runs through the labelled transitions, and silent transitions
  that skip them go aside. The quality metrics use the same main path.
- `CycleOptions::whole_returns` (default `true`) reverses a backward edge together with its return
  path of up to three edges. A loop's redo path then lies between the loop's entry and exit with
  its arrows pointing back, as in Inductive visual Miner.
- `Spacing::narrow_layer_gap` (default 38) is the gap next to a layer whose nodes are all at most
  half the layer gap deep, e.g., silent transitions. The hub-and-spoke presets keep the full gap.
- In a fork/join block off the main path, a branch's places line up with its widest node, e.g.,
  its labelled transition.
- A node whose neighbours are all on the main path (e.g., a loop's redo transition) moves with
  the path when it is straightened.
- A direct return between a loop's place and its redo node attaches to the place through the
  border facing the redo node.
- Edges in a layer gap keep `Spacing::stub` from the nodes on either side. Where ports add more
  edges to a gap than it holds, the gap is widened and the edges are routed again. Jogs of a few
  pixels next to ports are removed.
- `svg::SceneEdge::smooth` draws an edge as one B-spline through its corners (d3's `curveBasis`),
  straight at both ends for the markers. Where the curve would cut a node, the edge falls back to
  `corner_radius`.
- `SceneEdge`, `Spacing` and `CycleOptions` have new public fields, so struct literals need
  `..Default::default()` and the next release is 0.4.0.

## 0.3.1 (2026-10-26)

- `reroute` keeps every node where the caller put it. Nodes dropped closer together than the node
  gap were pushed apart, so a host that kept its own positions drew routes made for other ones.
- `reroute` joins two nodes stacked across the flow by one straight edge between the borders that
  face each other, where the router left by the flow-facing border and looped round to the other.
- `reroute` moves an edge's end to another border when the edge would run through its own source
  or target, for either end, and tries all three other borders. It checks a place against its
  circle, not its bounding box, and leaves the ends of a `PortSides::FacingOnly` node alone.
- `layout` with pinned nodes closer together than the node gap kept the pins but could push a free
  node between them out of the layer order. Such a node is now spread evenly between its pins.
  `Node::pinned` without a position now has no effect, as documented.

## 0.3.0 (2026-10-16)

- `svg::NodeContent::Polygon`: a filled or stroked polygon with its points relative to the node
  centre.
- `FitLayout::max_glyphs` collapses a row to its numeral past a count, even when the glyphs would
  fit. `NumeralStyle::counted` draws that numeral as one line per cluster, its glyph and count,
  shrunk to the row's width. Both are new public fields, so the next release is 0.3.0.
- `TextRun::spans`: parts of one line in their own colour or weight, drawn as `<tspan>`s with
  `xml:space="preserve"`, since Qt SVG and librsvg drop the spaces between them otherwise.
- `Decoration::Tick::slant` turns a tick away from the perpendicular, e.g., for a `//` that
  crosses out an edge.
- `EdgeAnchor::Fraction` now measures along the line as drawn, so a label or decoration near a
  rounded corner sits on the curve instead of on the corner of the polyline.
  `EdgeAnchor::Along { fraction, offset }` places one a fixed distance from a fraction, e.g.,
  27 units before the end.
- Centred text is now placed by moving its `y` by the measurer's ascent and descent. The renderer
  no longer writes `dominant-baseline="central"`, which Qt SVG (Gwenview, Okular) ignores, so
  labels there sat half a line too high.
- `font-weight` is written as a whole number. Qt SVG and librsvg ignored values like `700.00`, so
  bold labels came out regular.

## 0.2.1 (2026-10-15)

- Releases now publish through trusted publishing. No code changes.

## 0.2.0 (2026-10-15)

First release.

- A layered layout: cycle breaking, network-simplex layering, multi-seed crossing minimisation
  with sifting, network-simplex or Brandes-Kopf coordinates, and orthogonal or diagonal routing.
- A tidy-tree layout, and `reroute` for keeping nodes where a user moved them.
- Port sides and port hints, merged edges, parallel-edge bundles, self-loops, edge labels, groups
  and strict swimlanes, compounds, rank constraints, row wrapping, and relayout from seeds.
- The same drawing for the same input natively, in wasm and in Python, checked by a conformance
  corpus.
- Crossing minimisation on several threads with the default `parallel` feature, with the same
  result as on one.
- An SVG renderer for the `svg::Scene` model, and `svg::sketch` for a plain drawing of a layout.
- Quality metrics and a score (`metrics::measure`), and the `eval` harness over a fixture corpus.
- A JSON request format, an npm package built from wasm, a Python package and a command-line tool.

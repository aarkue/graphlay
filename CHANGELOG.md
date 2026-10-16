# Changelog

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

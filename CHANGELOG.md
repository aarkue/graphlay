# Changelog

## 0.2.0 (unreleased)

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

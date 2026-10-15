# graphlay

Deterministic graph layout in pure Rust. The main engine is layered (Sugiyama-style): cycle
breaking, weight-aware layering, crossing minimisation, network-simplex or Brandes-Kopf
coordinates, and orthogonal or diagonal edge routing. There is also a tidy-tree layout and an SVG
renderer. The npm package (wasm) and the Python package wrap the same engine and give the same
drawings.

| | | |
|---|---|---|
| [![An object-centric directly-follows graph](docs/gallery/process_models-3.svg)](docs/gallery.md#process-models) | [![An object-centric Petri net](docs/gallery/process_models-2.svg)](docs/gallery.md#process-models) | [![A directly-follows graph of road traffic fines](docs/gallery/process_models-4.svg)](docs/gallery.md#process-models) |
| [![A service map](docs/gallery/styles-5.svg)](docs/gallery.md#drawing-styles) | [![Frequencies as heat](docs/gallery/styles-3.svg)](docs/gallery.md#drawing-styles) | [![A finite automaton](docs/gallery/styles-4.svg)](docs/gallery.md#drawing-styles) |
| [![Merged edges](docs/gallery/merge_edges-2.svg)](docs/gallery.md#merged-edges) | [![Swimlanes](docs/gallery/swimlanes-2.svg)](docs/gallery.md#swimlanes) | [![Nested subprocesses](docs/gallery/compounds-1.svg)](docs/gallery.md#nested-subprocesses) |
| [![A Petri net: order to cash](docs/gallery/process_models-1.svg)](docs/gallery.md#process-models) | [![Wrapping into rows](docs/gallery/wrapping.svg)](docs/gallery.md#wrapping-into-rows) | [![Port hints](docs/gallery/port_hints-2.svg)](docs/gallery.md#port-hints) |

The [gallery](docs/gallery.md) shows each drawing with the code that made it.

## Packages

| package | install | docs |
|---|---|---|
| Rust crate | `cargo add graphlay` | `cargo doc --open` |
| npm (wasm) | `npm install graphlay` | [`bindings/wasm/js/README.md`](bindings/wasm/js/README.md) |
| Python | `pip install graphlay` | [`bindings/python/README.md`](bindings/python/README.md) |
| command line | `cargo install graphlay-cli` | [`cli/README.md`](cli/README.md) |

## Usage

```rust
use graphlay::{layout, GraphBuilder, LayoutOptions, Node, Size};

let mut b = GraphBuilder::new();
let a = b.add_node(Node { size: Size::new(120.0, 52.0), ..Node::default() });
let c = b.add_node(Node { size: Size::new(120.0, 52.0), ..Node::default() });
b.connect(a, c);

let drawing = layout(&b.build(), &LayoutOptions::layered_default()).expect("a valid graph");
for placement in &drawing.nodes {
    println!("({:.0}, {:.0}) in layer {:?}", placement.center.x, placement.center.y, placement.rank);
}
for route in &drawing.edges {
    println!("{:?}: {} points", route.kind, route.points.len());
}
```

`graphlay::svg::sketch` draws a layout as SVG. `graphlay::json` takes the same request as the
npm and Python packages: `{ "graph": ..., "preset": ..., "options": ... }`.

## Documentation

- [`docs/guide.md`](docs/guide.md): which options to set for which goal.
- [`docs/gallery.md`](docs/gallery.md): a picture of each feature, with its code.
- [`docs/svg.md`](docs/svg.md): the SVG renderer and its scene model.
- [`docs/architecture.md`](docs/architecture.md): modules, layout phases and tests.
- [`docs/quality.md`](docs/quality.md): the quality metrics and the `eval` harness.
- [`docs/benchmarks.md`](docs/benchmarks.md): graphlay, dagre and ELK on the same corpus.
- [`docs/literature.md`](docs/literature.md): the papers behind the phases and metrics.

## Feature flags

| feature | default | contents |
|---|---|---|
| `serde` | yes | the `json` module |
| `svg` | yes | the SVG renderer, `graphlay::svg` |
| `parallel` | yes | crossing minimisation on several threads, with the same result as on one |
| `schemars` | no | `JsonSchema` derives, for binding generators |
| `fixtures` | no | the fixture loader, the `eval` example and the gallery test |

## License

MIT or Apache-2.0, at your option.

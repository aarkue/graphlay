# Architecture

How the crate is laid out and what runs in which order. The metrics are in
[`quality.md`](quality.md) and the SVG model in [`svg.md`](svg.md).

## Entry points

| Call | Does |
|---|---|
| `graphlay::layout(&Graph, &LayoutOptions)` | A layered (Sugiyama-style) layout from scratch, or from `Node::position` seeds. |
| `graphlay::reroute(&Graph, &LayoutOptions)` | Keeps every node at its `position` and recomputes layers and routes, e.g., after a drag. |
| `graphlay::tree::layout(&Graph, &TreeOptions)` | A tidy tree, parents centred over their children, for rooted trees and forests. |
| `json::LayoutRequest::split`, `json::TreeRequest::split` | The JSON form: a graph, a `Preset` and an options patch, with errors that name the bad field. |
| `svg::render(&Scene, &Theme, &dyn TextMeasurer)` | Draws a `Scene` as SVG text. `svg::sketch` builds a plain one from a layout. |

The layout calls validate first (`Graph::validate`, `LayoutOptions::validate`, `Limits::check`)
and return a `LayoutError` on bad input. The crate denies `unwrap` and `expect` outside tests and
forbids unsafe code.

## Crate map

| Module | Role |
|---|---|
| `graph` | Input model: `Graph`, `Node`, `Edge`, `GraphBuilder`, validation. |
| `options` | `LayoutOptions`, the presets, and `LayoutOptions::tuning` for iteration counts and thresholds. |
| `layout` | Output model: `Layout`, `Placement`, `EdgeRoute`, `PortInfo`, `RouteKind`. |
| `error` | `LayoutError`. |
| `json` | The JSON request format (feature `serde`). |
| `compound` | Nested layout, bottom-up. Each compound's children are laid out flat, the compound is sized to hold them, and boundary nodes with port hints join the levels. |
| `canonical` | Renumbers a keyed graph (`Node::key`) so the drawing does not depend on input order. |
| `geometry` | Shapes and border hits, polylines, statistics, and `fp`, the math functions that give the same bits on every target. |
| `layered` | The layered engine (private): ingest, front ends, the phases, output. |
| `tree` | The tidy-tree engine. |
| `svg` | Scene model, renderer, text measurement and `sketch` (feature `svg`). |
| `metrics` | Quality measurement of a laid-out graph, and stability between two layouts. |
| `splits` | Fork/join block analysis for the `AlignSplits` phase and the metrics. |
| `sort` | One shared sort instantiation, which keeps the wasm small. |
| `fixture`, `synthetic`, `pipeline` | The fixture format, generated stress graphs and the per-phase profile (feature `fixtures`). |

## From `Graph` to `Layout`

```text
layout(g, o)
  validate g and o
  canonical::canonical(g)            only when every node has a unique key
  layered::prepare::ingest           Graph -> LayeredInput, in flow space
  layered::layout_layered_inner
      bundle::bundle_parallel_arcs   parallel arcs laid out as one edge, then split into lanes
      isolated-node split            edgeless nodes go into a grid below the drawing
      LayeredState::new
      Pipeline::layered().run        the phases below
  layered::finish::emit              flow space -> output space
  canonical::restore                 back to the caller's numbering
```

`reroute` shares ingest, the state and the output, and runs `Pipeline::reroute()`:
`RankFromPositions`, `ReserveSelfLoops`, `PlaceOverPositions`, `Route`, `RouteSelfLoops` and
`MergeEdges`.

Everything inside `layered` works in flow space. "Along" is the layer axis and "cross" the axis
within a layer, whatever the drawing's direction. `finish` transposes for top-to-bottom drawings.

## Phases

`Pipeline::layered()` lists the phases `layout()` runs, as `layered::pipeline::Phase` values.
"Seeds" means some node has a `position`. "Structure kept" means `SeedMode::Structure` applies.

| Phase | Module | What it does | Skipped when |
|---|---|---|---|
| BreakCycles | `cycle`, `stable` | Reverses arcs so the graph is acyclic (`CycleStrategy`), or reads layers off the seeds. | |
| AssignRanks | `rank` | Network-simplex layering with fixed layers (`RankConstraint`). | structure kept |
| InsertDummies | `dummy` | Cuts long edges into chains of waypoints, one per layer. | |
| MinimizeCrossings | `order` | Multi-seed barycentre and median sweeps, sifting and kicks, exhaustive search on small layers. | structure kept |
| EnforceGroups | `group` | Bands groups (`Node::group`) into lanes within a crossing budget, or always for `GroupLanes::Strict`. | seeds, fewer than 2 groups |
| BuildChains | `pipeline` | Collects each edge's waypoint chain. | |
| OrderChains | `chainorder` | Swaps waypoints past neighbours where that opens a lane and costs no crossing. | |
| BalanceSides | `balance` | Mirrors side branches to balance the main path. | seeds |
| ReserveLabels | `label` | Widens layer gaps for edge labels. | |
| ReserveSelfLoops | `selfloop` | Picks each self-loop's border and reserves its room. | |
| PlaceAlong | `coord` | Spaces the layers. | |
| PlaceCross | `coord`, `simplex` | Cross-axis coordinates by network simplex or Brandes-Kopf, or at the seeds. | |
| ChannelPullIn | `pullin` | Pulls a long edge's channel in from its worst layer. | seeds |
| AlignSplits | `splits` | Draws fork/join blocks as blocks. | structure kept |
| CentreTerminals | `coord` | Centres first- and last-layer terminals over their flow. | option off |
| Compact | `compact` | Order-preserving compaction (hub-and-spoke preset). | option off |
| AlignLanes | `group` | Pulls each group's band onto one slot. | seeds, fewer than 2 groups |
| AlignMainPath | `mainpath` | Draws the heaviest flow straight. | seeds, symmetric branches, strict lanes |
| StraightenChains | `coord` | Pulls each long edge onto one lane. | |
| FlattenChains | `flatten` | Moves nodes aside to give a long edge a lane. | structure kept |
| ReserveLanes | `coord`, `stable` | Widens gaps for the vertical runs they hold. | |
| Route | `route/` | Ports, polylines, bundle lanes and repair passes. | |
| RouteSelfLoops | `selfloop` | Draws each self-loop as a bump. | |
| MergeEdges | `merge` | Draws forks and joins on a shared trunk where that costs no crossing or bend. | option off |
| Wrap | `wrap` | Wraps a long drawing into rows for a viewport aspect. | option off |
| PlaceLabels | `label` | Puts each label in its reserved gap. | |
| MeasureLanes | `group` | Measures the strict swimlanes' rectangles. | not strict lanes |

`route/mod.rs` runs the router's stages over a shared `RouteCtx`. `sides` picks each end's border,
`spread` places ports along each border, `gutter` places waypoints and gutter lanes, `orthogonal`
and `diagonal` draw the first polylines, and `lanes` draws bundle lanes. The repair passes follow:
`repair`, `centre` and `separate`. `hints` resolves the caller's port hints once, and every later
stage keeps a hinted end where the hint put it.

## Shared state

`layered::state::LayeredState` is the one mutable object the phases share. It holds the internal
graph (`nodes` with real nodes first, then waypoints, and `arcs`), the layering (`layers`,
`order_of`), per-edge data (`chains`, `weight`, `routes`, bundle lanes, label sizes and anchors),
layer gaps, group bands, self-loop slots, the flow `axis` and the run `mode`.

## Determinism

Every phase breaks ties by index and sorts floats with `total_cmp`. Trigonometry and `hypot` go
through `geometry::fp`, so native, wasm and Python builds produce the same bits. With `Node::key`
on every node the drawing does not depend on input order. The `parallel` feature runs crossing
minimisation on several threads and gets the same result as on one.

## Tests

| What | Where |
|---|---|
| Unit tests per phase and kernel | `#[cfg(test)]` modules in `src/` |
| Exact output of every fixture under each option variant | `tests/golden.rs`, `tests/golden.json` |
| The drawings in `docs/gallery.md` | `tests/gallery.rs`, examples in `tests/gallery/` |
| The corpus the npm and Python packages must reproduce | `tests/conformance.rs`, `tests/conformance/` |
| Layout contract on random graphs (no overlaps, ends on outlines, determinism, order invariance) | `tests/random_graphs.rs` |
| Contract over the fixture corpus | `tests/fixtures_layout.rs` |
| Relayout, pinning, reroute, lanes | `tests/relayout.rs` |
| Layer constraints | `tests/constraints.rs` |
| Port hints | `tests/port_hints.rs` |

`GRAPHLAY_BLESS=1` on a release build rewrites the snapshots.

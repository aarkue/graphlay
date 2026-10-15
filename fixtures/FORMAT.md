# Fixture format

Each fixture is one JSON file at `fixtures/<kind>/<name>.json`. It holds only the input. The
`eval` harness lays it out and measures the result.

```jsonc
{
  "name": "sepsis-dfg-all",            // unique, file stem
  "kind": "dfg",                       // petri | ocpn | dfg | ocdfg | ocdeclare | tree | typegraph | synthetic
  "source": "test_data/xes/Sepsis Cases - Event Log.xes.gz, DFG discovery, all edges", // free text provenance
  "direction": "TB",                   // "TB" (top-bottom) or "LR" (left-right)
  "options": {
    "flow_edges": true,                // process-map style (DFG-like) spacing/routing
    "flow_diagonal": false,            // diagonal instead of orthogonal routing (only with flow_edges)
    "tree": false,                     // tidy-tree mode (input must be a rooted tree/forest)
    "compact": false                   // compact the cross axis (hub-and-spoke graphs)
  },
  "nodes": [
    {
      "id": "A",                       // stable string id (unique)
      "label": "Register patient",     // display text (may be "" for silent transitions / dummies)
      "width": 120.0, "height": 52.0,  // px box extents
      "shape": "box",                  // "box" | "ellipse"
      "pin": null,                     // null | "first" | "last"  (source / sink rank)
      "category": null                 // null | integer lane group (e.g. object type index)
    }
  ],
  "edges": [
    {
      "from": 0, "to": 1,              // node indices into `nodes`, from == to is a self-loop
      "weight": 1.0,                   // layout importance (e.g. 1 + ln(count)); > 0
      "thickness": 2.0,                // drawn stroke width in px
      "label": null,                   // null | display text
      "label_size": null,              // null | [w, h] px box to reserve for the label
      "category": null,                // null | integer group (e.g. object type index of an OC-DFG edge)
      "reversed": false                // rank as to -> from, the route still runs from -> to
    }
  ]
}
```

Node indices must be in range and `weight` must be finite and positive. Parallel edges and
self-loops are allowed. With `reversed`, cycle breaking, layering and ordering see `to -> from`,
so the target lands in the lower layer. The polyline still runs `from -> to` with the arrowhead at
`to`.

Node sizes follow the usual drawing of each model. Places are 52x52 ellipses, transitions 120x52
boxes and silent transitions 14x52 boxes. DFG activities are `clamp(8 * chars + 24, 120, 260)` by
52, process-tree operators 64x64 ellipses and leaves 160x56. OC-DECLARE activities are 140x44 with
edge labels of about `[7 * chars + 8, 14]`.

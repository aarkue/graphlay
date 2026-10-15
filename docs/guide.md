# Choosing options

This guide maps what you want a drawing to do to the options that do it. The API docs
(`cargo doc --open`) have the full meaning of each option, and the [gallery](gallery.md) has
pictures of most of them.

Everything below is a field of `LayoutOptions`. In JSON, send a preset name and a patch with only
the fields you change. The layout merges the patch into the preset and rejects an unknown or
invalid field with an error that names it:

```json
{ "graph": { "nodes": [], "edges": [] },
  "preset": "process_map",
  "options": { "routing": { "merge_edges": "both" }, "effort": "thorough" } }
```

## Presets

| preset | for | what it sets |
|---|---|---|
| `LayoutOptions::layered_default()` (`"layered"`) | Petri nets, BPMN-like models, most directed graphs | left to right, orthogonal edges, edges preferably enter from the sides, one straight branch per fork/join block |
| `LayoutOptions::process_map()` (`"process_map"`) | directly-follows graphs and other process maps | the same, top to bottom, with tighter gaps and start and end nodes centred over their main flow |
| `LayoutOptions::hub_and_spoke()` (`"hub_and_spoke"`) | dense relationship graphs without a direction of flow | tall layer gaps, a tight order axis, compaction, edges on any side |
| `LayoutOptions::hub_and_spoke_flow()` (`"hub_and_spoke_flow"`) | dense graphs that still have a flow, e.g., an object-type graph of an event log | `hub_and_spoke` spacing with the process-map centring |

`LayoutOptions::default()` is `layered_default()`.

## Direction and cycles

`direction` is `LeftRight` or `TopBottom`. Margins, port hints and lanes mean the same thing in
both.

One edge of every cycle has to point backwards. `cycles.strategy = MinimumReversal` (default)
reverses the least total edge weight, which gives the fewest crossings. `FollowFlow` keeps
everything outside a cycle pointing forward, which reads better when the graph has a clear start.
If you know which edges are loops (a rework edge, a retry), set `Edge::reverse_for_ranking = true`
on them and the layering treats them as backward edges without guessing. This matters most when
every node sits on a cycle, because then no strategy can tell where the flow starts.

`Node::rank = RankConstraint::First` or `Last` pins a node to the first or last layer, and
`Fixed(k)` puts nodes that share a `k` in one layer.

`Edge::weight` makes an edge shorter and straighter than its neighbours.
`coords.branch_style = StraightMain`, the default in the process presets, draws one branch of every
fork/join block straight through.

## Ports

`routing.port_sides` sets how readily an edge attaches to a border across the flow, e.g., the top
or bottom of a node in a left-to-right drawing. From strictest to loosest the values are
`FacingOnly`, `FacingPreferred`, `LongEdgesAny` and `Any`. `Node::port_sides` overrides it for one
node.

`routing.centre_ports` packs the ends on a border around its middle: `Round` (default, ellipses
only), `All` or `Off`.

`Edge::source_port` and `Edge::target_port` force a border (`PortHint::Side`) or a point on it
(`PortHint::Offset`), and a hint always wins. Placement puts the far end of an offset-hinted edge
level with the hinted point, so the edge can run straight. The fork/join and main-path
straightening passes align node centres and can move it off again.

`routing.style = Diagonal` draws straight segments between ports instead of orthogonal ones.

## Busy drawings

`routing.merge_edges` (`Forks`, `Joins` or `Both`) draws the edges that fan out of or into one node
on a shared trunk, with a junction dot where they split. It never adds a crossing or a bend.

`routing.bundle_parallel`, on by default, draws parallel edges between the same two nodes as one
bundle of lanes.

`effort = Thorough` spends more time on crossing minimization. `Fast` spends less, for interactive
use on large graphs.

In SVG, `SceneEdge::hop_radius` draws a small arc where two edges cross, so a crossing does not
look like a junction.

## Groups

`Node::group` puts nodes into lanes, e.g., one per object type or department.

`order.group_lanes = Soft` (default) bands the groups where that is cheap.
`order.group_lane_budget_percent` caps the extra crossings and `order.group_lane_area_percent` the
extra width. Both default to `0`, which keeps only the bandings that cost no crossings. Raise them
for cleaner strips.

`order.group_lanes = Strict` draws swimlanes, a strip per group whatever it costs, and
`Layout::lanes` returns the strips as rectangles to draw.

`Node::lane_key` orders the nodes inside a lane.

## Nesting

Set `Node::parent` to draw a node inside another. Compounds nest to any depth.
`spacing.compound_padding` is the room around the children and `spacing.compound_header` the extra
room at the top for a title. `reroute` and the tree layout take flat graphs only.

## Screen fit

`wrap.viewport_aspect` (e.g., `16.0 / 9.0`) breaks a long drawing into rows for a viewport of that
shape. It is off by default, and strict lanes turn it off.

`spacing.layer_gap` is the gap between layers and `spacing.node_gap` the gap between neighbours in
a layer. Labels, self-loops and edge bundles reserve their own room on top.

`Node::margin` keeps room free around a node for content you draw yourself, e.g., a badge or a
counter. `cross_*` keeps neighbours in the layer away and `along_*` the layers before and after.
Nodes stay out of the margin, edges may still pass through it.

## Stability

To filter without the drawing jumping, lay out the full graph once. Then lay out each filtered
view with `Node::position` set to the full drawing's positions and `seed_mode = Structure`. Nodes
keep their layer and their order.

After the user drags a node, call `reroute` with every node's position. No node moves and the
edges are routed around the new positions.

Give nodes a `Node::key` (an activity name, an id) and the drawing depends only on the graph, not
on the order you added nodes and edges in.

## Labels

`Edge::label_size` reserves room for an edge label, and `EdgeRoute::label_anchor` returns the centre
of that room. `labels.placement` picks the middle of the gap (`GapCentre`) or the best of several
candidate slots (`Candidates`).

## Input limits

`limits.max_nodes`, `limits.max_edges` and `limits.max_dummies` reject an input that is too big
with `LayoutError::TooLarge`, before the expensive work starts. Dummies are the routing waypoints
of long edges, and they are what grows fastest.

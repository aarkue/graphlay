# Layout quality

`graphlay::metrics::measure(&graph, &options, &layout)` measures one drawing. It is pure geometry
and deterministic, so the same code gives the same numbers and a run can serve as a baseline.

Defect counts (crossings, edges through nodes, overlaps) are what most layout papers optimise.
Perceived-quality metrics (label size after scaling to a screen, density, aspect, fork/join
alignment) catch drawings with no defects that are still hard to read, like a tidy tree 5000 px
wide. Still look at the drawings, see [Reviewing by eye](#reviewing-by-eye).

## Metrics

### Problem size

| metric | meaning |
|---|---|
| `node_count`, `edge_count` | nodes and edges in the input, self-loops included |
| `unrouted_edges` | edges with fewer than two route points |
| `self_loops` | edges from a node to itself |
| `parallel_edge_groups` | `(from, to)` pairs carrying more than one edge |

### Defects

| metric | meaning |
|---|---|
| `crossings` | segment pairs of different edges that properly cross. Crossings within 1 px of an endpoint the two edges share are not counted |
| `overlaps` | segment pairs collinear within 0.5 px that overlap for more than 2 px, i.e., two edges drawn as one line. Tree elbows share a bus by design and count here |
| `node_edge_hits` | `(edge, node)` pairs where the edge passes through a node it does not attach to |
| `node_overlaps` | node boxes overlapping by more than 0.5 px on both axes |
| `endpoint_off_outline` | route ends more than 0.5 px off their node's border |
| `max_endpoint_outline_gap` | the largest such distance (px) |
| `inward_ends` | route ends whose first segment heads back across the border it attaches to |
| `backward_edges` | edges whose target is not strictly after its source along the layout direction |

### Route shape

| metric | meaning |
|---|---|
| `bends` | direction changes over 5 degrees, summed over all edges |
| `edges_with_bends` | edges with at least one bend |
| `total_edge_length`, `mean_edge_length`, `max_edge_length` | polyline lengths (px) |
| `detour_ratio` | polyline length over the straight distance between the ends. `1.0` is straight |
| `short_steps` | jogs under 12 px between two runs, the target is 0 |
| `staircase_steps` | consecutive jogs in the same cross direction. A level change should be one jog |
| `route_reversals` | how often a route turns back on itself along either axis |
| `perpendicular_ends` | ends on a border across the layout axis, e.g., the top of a node in a left-to-right drawing |
| `off_centre_ends` | ends more than a third of the way from the middle of their border to its corner |
| `converged_ends` | ends sharing their attachment point with another end on the same node |

### Clearances

Bigger is better, and each is `None` when undefined.

| metric | meaning |
|---|---|
| `min_node_gap` | smallest gap between two node boxes, negative when they overlap |
| `min_edge_node_clearance` | smallest distance from an edge to a node it does not attach to |
| `min_port_gap` | smallest distance between two edge ends on the same node |

### Extent and perceived quality

| metric | meaning |
|---|---|
| `width`, `height`, `area` | bounding box of all node boxes and route points |
| `aspect_ratio` | `width / height` |
| `fit_font_px` | the size of a 12.5 px label once the drawing is scaled to fit 1600x900, never above 12.5. Below about 7 labels are unreadable |
| `density` | node box area over bounding-box area |
| `aspect_penalty` | `max(aspect, 1 / aspect)`, `1.0` for a square |
| `split_pairs` | fork/join blocks in the graph |
| `split_aligned` | how many of them the drawing shows as a block, fork and join level and the branches on either side |
| `split_alignment` | `split_aligned / split_pairs`, `1.0` without blocks |

### Labels

All are `0` or `None` when no edge has a label.

| metric | meaning |
|---|---|
| `label_count` | edges that asked for a label and got an anchor |
| `label_label_overlaps` | pairs of label boxes overlapping by more than 0.5 px |
| `label_node_overlaps` | `(label, node)` pairs overlapping, the label's own end nodes included |
| `label_edge_hits` | `(label, edge)` pairs where another edge enters the label box |
| `label_overlaps` | the sum of the three, the one in the score |
| `min_label_clearance` | smallest distance from a label to another label, a node or another edge |

### Crossings by weight and angle

| metric | meaning |
|---|---|
| `weighted_crossings` | sum of `w(e) * w(f)` over crossing segment pairs (Mennens et al. 2019) |
| `thickness_weighted_crossings` | the same over `Edge::thickness` |
| `crossing_angle_min`, `crossing_angle_mean` | acute angle between crossing segments, in degrees |
| `crossings_below_30deg` | crossings shallower than 30 degrees |

### Flow

| metric | meaning |
|---|---|
| `main_path_len` | nodes on the heaviest source-to-sink path, `0` when there is none |
| `main_path_aligned_fraction` | share of them within 2 px of the path's median cross coordinate |
| `main_path_deviation` | mean cross-axis deviation from that median over the path's length |
| `flow_consistency` | share of drawn length heading in the dominant pair of compass directions (Burattin et al. 2018) |
| `flow_consistency_weighted` | the same with each segment weighted by `Edge::weight` |
| `path_turn_mean` | weighted mean of the total turn angle along each edge, in degrees |
| `path_turn_max` | the largest single turn in the drawing |

### Crowding and lanes

| metric | meaning |
|---|---|
| `parallel_run_length` | length (px) over which two edges run within 10 degrees of parallel and closer than 1.5 stroke widths |
| `block_intrusion` | segments of other edges passing through a fork/join block's bounding box |
| `lane_purity` | mean share of each group's cross-axis span that no other group occupies, `1.0` with fewer than two groups |
| `lane_intruders` | nodes of another group inside a group's interquartile band |
| `whitespace_balance` | coefficient of variation of node area over a grid of about one cell per node, `0.0` is even (Mooney et al. 2024) |

### Stability

`metrics::compare(&a, &b, &common)` compares two drawings of graphs that share nodes, e.g., a
full graph and a filtered view of it.

| metric | meaning |
|---|---|
| `hausdorff` | largest distance (px) a common node moved |
| `relative_euclidean` | mean relative change in the distance between sampled node pairs |
| `orthogonal_ordering` | share of sampled pairs whose left/right or above/below relation flipped |
| `edge_shape` | mean shape change of the edges both drawings have |
| `common_nodes`, `sampled_pairs`, `matched_edges` | what the above were computed over |

## Score

`LayoutMetrics::score(&weights)` sums the metrics into one number, lower is better. Use it to
spot a regression, then look at the metric that moved. Counts enter unnormalised, so only scores
of the same graph are comparable.

```text
score =  crossing         * crossings
      +  overlap          * overlaps
      +  node_edge_hit    * node_edge_hits
      +  node_overlap     * node_overlaps
      +  bend             * bends
      +  backward_edge    * backward_edges
      +  area             * area / (10_000 * n)
      +  edge_length      * total_edge_length / (100 * n)
      +  fit_font         * (8 - fit_font_px).max(0)^2          [0 when fit_font_px == 0]
      +  aspect           * (aspect_penalty - 3).max(0)
      +  split_misaligned * (split_pairs - split_aligned)
      +  label_overlap    * label_overlaps
      +  main_path        * main_path_len * (1 - main_path_aligned_fraction)
      +  flow             * routed_edges * (1 - flow_consistency)
```

`n` is `node_count.max(1)` and `routed_edges` is `edge_count - unrouted_edges`. The font term is
quadratic because 7 px is hard to read and 4 px is impossible. The other metrics are reported but
not scored.

| weight | `balanced()` (default) | `readability()` |
|---|---|---|
| `crossing` | 10 | 10 |
| `overlap` | 4 | 4 |
| `node_edge_hit` | 25 | 25 |
| `node_overlap` | 50 | 50 |
| `bend` | 1 | 1 |
| `backward_edge` | 0.5 | 0.5 |
| `area` | 12 | 1 |
| `edge_length` | 12 | 1 |
| `fit_font` | 10 | 20 |
| `aspect` | 8 | 12 |
| `split_misaligned` | 10 | 12 |
| `label_overlap` | 25 | 25 |
| `main_path` | 10 | 12 |
| `flow` | 2 | 3 |

`readability()` uses compactness only as a tie-breaker and weighs the perceived-quality terms
higher. In both, a drawing that is bad at one term pays about what a drawing with many crossings
pays. The flow term counts every routed edge, so it gets a fifth of the weight of the terms that
count nodes.

## The eval harness

The `eval` example lays out every fixture under `fixtures/` (format:
[`fixtures/FORMAT.md`](https://github.com/aarkue/graphlay/blob/main/fixtures/FORMAT.md)), measures
it and prints a table. `--help` lists every flag.

```sh
cargo run -p graphlay --release --features fixtures --example eval -- [args]

#   --filter <substr>       only fixtures whose name or kind contains <substr>
#   --svg-out <dir>         render one SVG per fixture into <dir>
#   --baseline <file.json>  compare against saved metrics, print deltas and mark regressions
#   --save-baseline <file>  write this run's metrics as a new baseline
#   --repeat <n>            lay each fixture out n times and take the median duration
#   --effort <level>        fast, balanced (default) or thorough
#   --wide                  print a second table with the metrics the main table leaves out
#   --phase-profile         the staircase steps of the long edges after each pipeline phase
#   --stability             lay each fixture out again without its lightest 10% of edges and
#                           report how far the drawing moved
```

Further flags override one option for every fixture: `--port-sides`, `--merge-edges`,
`--group-lanes`, `--placement`, `--labels`, `--label-reserve`, `--lane-budget` and `--lane-area`.

In the table, `fit` is `fit_font_px`, `split` is `split_aligned/split_pairs`, `wcross` is
`weighted_crossings`, `lbl` is `label_overlaps`, `main` is `main_path_aligned_fraction` and `flow`
is `flow_consistency`.

A typical check of a change:

```sh
cargo run -p graphlay --release --features fixtures --example eval -- \
    --svg-out /tmp/before --save-baseline /tmp/before.json
# change the engine
cargo run -p graphlay --release --features fixtures --example eval -- \
    --svg-out /tmp/after --baseline /tmp/before.json
```

The exit code is 1 when a fixture fails to lay out. Timings vary between runs, so compare
`--repeat 3` or more before reading anything into a small difference.

## Reviewing by eye

Some unreadable drawings score well. Render the corpus with `--svg-out` before and after a change
and compare the pairs.

The SVGs carry a `viewBox`, so a headless browser screenshot at a smaller window scales the whole
drawing the way a reader's screen does:

```sh
chromium --headless=new --no-sandbox --hide-scrollbars \
    --window-size=1600,900 --screenshot=out.png file:///abs/path/to/drawing.svg
```

Chromium adds an 8 px margin, so make the window about 20 px larger than the drawing. To check a
suspected overlap, copy the SVG, set its `viewBox` to the region at 1:1 and screenshot that.

Things to check, with the metric that should agree:

- Can you read a label? (`fit_font_px`)
- Are there big empty regions crossed by a few long edges? (`density`, `whitespace_balance`)
- Do long, nearly straight edges graze nodes? (`node_edge_hits`)
- Do fork/join blocks read as blocks? (`split_alignment`)
- Can you tell which edge a label belongs to? (`label_overlaps`, `min_label_clearance`)
- Does the heaviest path run straight? (`main_path_aligned_fraction`)

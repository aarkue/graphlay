//! Every option of the layout engine, as a field with a default.
//!
//! The presets ([`LayoutOptions::layered_default`], [`LayoutOptions::process_map`],
//! [`LayoutOptions::hub_and_spoke`], [`LayoutOptions::hub_and_spoke_flow`]) are starting points.
//! Each choice they make is an independent field, so a caller can take a preset and change one
//! thing. Iteration counts and budgets live in [`Tuning`], which most callers never touch.

use serde::{Deserialize, Serialize};

use crate::error::LayoutError;
use crate::graph::{Graph, PortSides};

/// Primary flow direction of the layered layout. Default [`Direction::LeftRight`].
///
/// The output is in screen coordinates either way (`x` right, `y` down). The direction decides
/// which axis the layers advance along. Options named in flow terms ([`crate::Margins`],
/// [`LoopSide`], [`PortSides`]) turn with it. Port hints ([`crate::PortHint`]) and the output
/// do not.
///
/// ```
/// use graphlay::{layout, Direction, GraphBuilder, LayoutOptions, Size};
///
/// let mut b = GraphBuilder::new();
/// let a = b.add_box(Size::new(100.0, 40.0));
/// let c = b.add_box(Size::new(100.0, 40.0));
/// b.connect(a, c);
/// let g = b.build();
/// let mut o = LayoutOptions::layered_default();
///
/// o.direction = Direction::LeftRight; // the default
/// let across = layout(&g, &o)?;
/// let gap = across.nodes[1].center.x - across.nodes[0].center.x - 100.0;
/// assert!((gap - o.spacing.layer_gap).abs() < 0.5);
///
/// o.direction = Direction::TopBottom;
/// let down = layout(&g, &o)?;
/// // The same layer gap, now between a 40px-tall box and the one below it.
/// let gap = down.nodes[1].center.y - down.nodes[0].center.y - 40.0;
/// assert!((gap - o.spacing.layer_gap).abs() < 0.5);
/// # Ok::<(), graphlay::LayoutError>(())
/// ```
///
/// There is no `BottomTop` or `RightLeft`. Mirroring the layer axis would reverse the handedness
/// the order axis, port sides and routing repairs assume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    /// Layers advance left to right.
    #[default]
    LeftRight,
    /// Layers advance top to bottom.
    TopBottom,
}

/// Gaps and clearances, all in px.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct Spacing {
    /// Between layer columns, on the layer axis. Default 54.
    ///
    /// A left-to-right drawing's width grows with `n_layers * (node_w + layer_gap)`, so lower
    /// this to narrow it.
    pub layer_gap: f64,
    /// Between two real nodes in one layer, on the order axis. Default 46.
    pub node_gap: f64,
    /// Between a routing track and its order-axis neighbour. Default 30, tighter than `node_gap`
    /// because a track is a line.
    pub dummy_gap: f64,
    /// Minimum port-to-port separation along one border. Default 18.
    pub port_gap: f64,
    /// Clear space between the strokes of two adjacent ports, on top of half of each stroke's
    /// width. Default 14. Raise it for thick strokes.
    pub port_stroke_clear: f64,
    /// Straight run out of a port before the first bend. Default 15.
    pub stub: f64,
    /// Clear space between the strokes of two lanes in one parallel-edge bundle. Default 8.
    pub bundle_lane_gap: f64,
    /// Depth step between two self-loops stacked on one node border. Default 10, also the
    /// default of [`SelfLoopOptions::gap`].
    pub self_loop_gap: f64,
    /// Extra layer gap beyond an edge label's own layer-axis extent, so the edge has room for
    /// the label and its arrow marker. Default 24.
    pub edge_label_margin: f64,
    /// Clear space between two edge labels stacked in one layer gap. Default 6.
    pub edge_label_stack_gap: f64,
    /// Order-axis gap [`crate::reroute()`] uses instead of [`Spacing::node_gap`]. Default 46.
    ///
    /// A reroute keeps the caller's coordinates and runs no compaction, so a compaction preset's
    /// tight `node_gap` would leave it cramped.
    pub reroute_node_gap: f64,
    /// Padding around each cell of the grid that holds edgeless nodes, and the gap between that
    /// grid and the connected drawing above it. Default 46.
    pub isolated_gap: f64,
    /// Column cap for the isolated-node grid. Further nodes wrap into new rows. Default 24.
    pub isolated_max_columns: usize,
    /// Room between a compound's border and its children's drawing, on every side
    /// ([`crate::Node::parent`]). Default 20.
    pub compound_padding: f64,
    /// Extra room along a compound's top border, on top of [`Spacing::compound_padding`], for a
    /// title. Default 0.
    pub compound_header: f64,
}

impl Spacing {
    /// The default gaps, close to ELK layered but a little looser so edges keep about 25px from
    /// nodes.
    pub fn layered() -> Self {
        Spacing {
            layer_gap: 54.0,
            node_gap: 46.0,
            dummy_gap: 30.0,
            port_gap: 18.0,
            port_stroke_clear: 14.0,
            stub: 15.0,
            bundle_lane_gap: 8.0,
            self_loop_gap: 10.0,
            edge_label_margin: 24.0,
            edge_label_stack_gap: 6.0,
            reroute_node_gap: 46.0,
            isolated_gap: 46.0,
            isolated_max_columns: 24,
            compound_padding: 20.0,
            compound_header: 0.0,
        }
    }
}

impl Default for Spacing {
    fn default() -> Self {
        Spacing::layered()
    }
}

/// How edges are drawn between their ports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum RoutingStyle {
    /// Orthogonal segments through gutter lanes, with ports on the border centre or spread along
    /// it. The default.
    #[default]
    Orthogonal,
    /// Diagonal segments. Each port sits where the ray from the node centre toward the other end
    /// crosses the border, and a straight port-to-port line is used wherever it clears every
    /// node box. Long edges keep side ports only where they add no crossing.
    Diagonal,
}

/// Which nodes have the edge ends on a border packed around its middle. Default
/// [`PortCentring::Round`].
///
/// Spreading ends at the port gap pushes the outer ones onto a circle's shoulders, where the stroke
/// meets the curve at a glancing angle. Centring packs them half a port gap apart around the
/// middle, and on a round node may move an end that turns anyway to the free top or bottom border.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum PortCentring {
    /// Every end keeps the port the router spread it to.
    Off,
    /// Ellipse nodes centre their borders' ends.
    #[default]
    Round,
    /// Every node centres its borders' ends. Only ellipses move ends round a corner.
    All,
}

/// How edges are routed: the style, bundling, and which borders and lanes the ends use.
///
/// The repair passes after the first routing pass are in [`RoutingTuning`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct Routing {
    /// How edges are drawn between their ports. Default [`RoutingStyle::Orthogonal`].
    pub style: RoutingStyle,
    /// Route parallel edges with the same endpoints as one edge, then split it into offset lanes.
    /// Default `true`. When off, each is routed separately and they can scatter across corridors.
    pub bundle_parallel: bool,
    /// Room reserved per riser in a layer gap. A gap where `k` edges change height is widened to
    /// at least `(k + 1) * lane_gap`, so risers never merge into one thick line. Default 8.
    pub lane_gap: f64,
    /// How readily an edge end may attach to a border across the layout axis instead of the one
    /// facing the edge's direction. See [`PortSides`]. [`crate::Node::port_sides`] overrides it
    /// per node. Default [`PortSides::Any`], but [`LayoutOptions::layered_default`] and
    /// [`LayoutOptions::process_map`] use [`PortSides::FacingPreferred`].
    pub port_sides: PortSides,
    /// Which nodes pack a border's ends around its middle. See [`PortCentring`]. Default
    /// [`PortCentring::Round`].
    pub centre_ports: PortCentring,
    /// Draw a node's forks (edges leaving it) or joins (edges reaching it) on one shared trunk
    /// that branches at a common riser. Default [`MergeEdges::None`]. Orthogonal routing only.
    /// It moves no node, and edges with a port hint are never merged.
    ///
    /// ```
    /// use graphlay::{layout, GraphBuilder, LayoutOptions, MergeEdges, Size};
    ///
    /// let mut b = GraphBuilder::new();
    /// let split = b.add_box(Size::new(80.0, 40.0));
    /// for _ in 0..3 {
    ///     let branch = b.add_box(Size::new(80.0, 40.0));
    ///     b.connect(split, branch);
    /// }
    /// let mut o = LayoutOptions::layered_default();
    /// o.routing.merge_edges = MergeEdges::Forks;
    /// let l = layout(&b.build(), &o)?;
    ///
    /// // All three edges leave `split` from one point, and branch where the junctions say.
    /// assert!(l.edges.iter().all(|e| e.points[0] == l.edges[0].points[0]));
    /// assert!(!l.junctions.is_empty());
    /// # Ok::<(), graphlay::LayoutError>(())
    /// ```
    pub merge_edges: MergeEdges,
}

/// Which fan-outs and fan-ins [`Routing::merge_edges`] draws on a shared trunk.
///
/// A merge is made only where it adds no crossing or bend and cuts through no node.
/// [`crate::Layout::junctions`] marks the branch points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum MergeEdges {
    /// Every edge keeps its own port and line.
    #[default]
    None,
    /// Edges leaving one node share a trunk.
    Forks,
    /// Edges reaching one node share a trunk.
    Joins,
    /// Both forks and joins.
    Both,
}

impl Default for Routing {
    fn default() -> Self {
        Routing {
            style: RoutingStyle::Orthogonal,
            bundle_parallel: true,
            lane_gap: 8.0,
            port_sides: PortSides::Any,
            centre_ports: PortCentring::Round,
            merge_edges: MergeEdges::None,
        }
    }
}

/// The router's repair constants ([`Tuning::routing`]).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct RoutingTuning {
    /// Minimum clearance between a diagonal segment and a node box. A closer segment takes a
    /// corner detour, since a graze reads as badly as a hit. Default 8.
    pub diagonal_clear: f64,
    /// How far outside a node box a diagonal's corner detour is placed. Default 12.
    pub diagonal_detour_margin: f64,
    /// How far past the inner channel's turn an outer channel is carried when two nested
    /// long-edge channels would otherwise cross at their shared target. Default 12.
    pub bundle_nest_gap: f64,
    /// Largest port jog that is straightened onto the edge's main track. Default 6.
    pub endpoint_snap: f64,
    /// Shortest segment that counts as a main track. Default 25.
    pub endpoint_snap_min_track: f64,
    /// Tallest step (run, perpendicular jog, run) merged away after routing by moving one run onto
    /// the other's line. Default `1.0e6` (no cap), `0.0` disables the pass.
    ///
    /// A move is taken only if it hits no node box, keeps ports [`Spacing::port_gap`] apart, and
    /// adds no crossing or overlap.
    pub step_flatten: f64,
    /// Minimum distance between two ports on adjacent borders of one node, the corner case
    /// `port_gap` does not cover. Default 10.
    pub port_relax_clear: f64,
    /// Step of the greedy nudge that enforces `port_relax_clear`. Default 3.
    pub port_relax_step: f64,
    /// Iterations of that nudge per node. Default 48.
    pub port_relax_rounds: u32,
    /// Rounds of the repair that separates two edges' runs drawn on one line along the layer
    /// axis, where that adds no box hit or crossing. Default 8. `0` disables it.
    pub overlap_repair_rounds: u32,
    /// How far that repair moves a run or a port, in one or two steps. Default 8. `0.0`
    /// disables the repair.
    pub overlap_repair_step: f64,
}

impl Default for RoutingTuning {
    fn default() -> Self {
        RoutingTuning {
            diagonal_clear: 8.0,
            diagonal_detour_margin: 12.0,
            bundle_nest_gap: 12.0,
            endpoint_snap: 6.0,
            endpoint_snap_min_track: 25.0,
            step_flatten: 1.0e6,
            port_relax_clear: 10.0,
            port_relax_step: 3.0,
            port_relax_rounds: 48,
            overlap_repair_rounds: 8,
            overlap_repair_step: 8.0,
        }
    }
}

/// Which border of its node a self-loop bumps out of. Default [`LoopSide::Auto`].
///
/// The names are in flow terms, like [`crate::Margins`], so they hold for either [`Direction`].
/// `cross` is the order axis (along which one layer's nodes spread) and `along` the layer axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum LoopSide {
    /// The positive order side (screen bottom in `LeftRight`, screen right in `TopBottom`). This
    /// is the side [`crate::Margins::cross_pos`] reserves room on.
    CrossPositive,
    /// The negative order side.
    CrossNegative,
    /// The forward layer side, toward the next layer (screen right in `LeftRight`, screen bottom
    /// in `TopBottom`).
    AlongPositive,
    /// Let the engine pick per node, once the layers are ordered.
    ///
    /// A node with an order-axis neighbour on one side only uses the free side. Otherwise it uses
    /// [`LoopSide::CrossPositive`], unless the node's own [`crate::Margins`] reserve more room on
    /// the negative side.
    #[default]
    Auto,
}

/// How self-loops (`source == target` edges) are drawn.
///
/// Self-loops take no part in cycle breaking, layering or ordering. Their room is reserved before
/// coordinate assignment, so placement widens exactly the slot the loops need.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct SelfLoopOptions {
    /// Which border the loops bump out of. Default [`LoopSide::Auto`].
    pub side: LoopSide,
    /// How far the innermost loop reaches beyond the node's border. Default 15, the
    /// [`Spacing::stub`] default, so a loop leaves its port by the same run as an ordinary edge.
    pub size: f64,
    /// Depth step between two loops stacked on one node, so `k` loops nest. Default 10, the
    /// [`Spacing::self_loop_gap`] default.
    pub gap: f64,
    /// Clear space between the outer run of a loop and its own label box. Default 4.
    pub label_gap: f64,
}

impl Default for SelfLoopOptions {
    fn default() -> Self {
        let spacing = Spacing::layered();
        SelfLoopOptions {
            side: LoopSide::Auto,
            size: spacing.stub,
            gap: spacing.self_loop_gap,
            label_gap: 4.0,
        }
    }
}

/// Which one-sided heuristic a crossing-minimisation sweep sorts a layer by. Default
/// [`OrderHeuristic::Alternate`].
///
/// Both place a node by where its neighbours in the previous layer sit. They differ on outliers:
/// Neighbours at `{0, 1, 20}` give position 7 under the barycentre and 1 under the median.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum OrderHeuristic {
    /// Weighted mean of the neighbours' positions (Sugiyama et al.). Heavy edges pull harder, so
    /// the main flow ends up straighter.
    Barycenter,
    /// Weighted median of the neighbours' positions (Eades and Wormald 1994), which has the
    /// better worst-case guarantee on one bilayer.
    Median,
    /// Barycentre on even sweeps, median on odd ones. The best-scoring sweep is kept.
    #[default]
    Alternate,
}

/// When a layer gap is widened to hold the labels crossing it. Default
/// [`LabelReservation::WhenNeeded`].
///
/// Reservation is the only guarantee of room, and it costs width because every label widens its
/// gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum LabelReservation {
    /// Every labelled edge widens its gap to hold its label.
    Always,
    /// Only a label too large for the order-axis space between two nodes of a layer
    /// ([`Spacing::node_gap`]) widens its gap. Smaller ones are left to
    /// [`LabelPlacement::Candidates`], which can put them in that space. The test uses the label's
    /// extent across the flow.
    #[default]
    WhenNeeded,
    /// Nothing is reserved. Placement fits each label into whatever free space exists, and
    /// takes the least-bad position when there is none.
    Never,
}

/// How a label's final position is chosen. Default [`LabelPlacement::Candidates`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum LabelPlacement {
    /// The middle of its reserved gap, with labels sharing a gap stacked across the flow.
    GapCentre,
    /// The best of several positions along the edge's own polyline (near the source, middle or
    /// target, on either side of the line), scored by what each one overlaps.
    #[default]
    Candidates,
}

/// How edge labels are given room, and where in the drawing they end up.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct LabelOptions {
    /// When a layer gap is widened to hold a label.
    pub reservation: LabelReservation,
    /// How the final position is chosen.
    pub placement: LabelPlacement,
    /// Clear space a label tries to keep from node boxes, other labels and other edges. Soft: A
    /// position with less clearance is penalised for the shortfall. Default 4.
    pub clearance: f64,
}

impl Default for LabelOptions {
    fn default() -> Self {
        LabelOptions {
            reservation: LabelReservation::default(),
            placement: LabelPlacement::default(),
            clearance: 4.0,
        }
    }
}

/// The candidate positions and rounds of label placement ([`Tuning::labels`]).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct LabelTuning {
    /// Fractions along its own polyline at which a label is tried, besides the midpoint of the
    /// longest segment. Ties go to the earlier fraction. Default `[0.5, 0.32, 0.68, 0.18, 0.82]`.
    pub fractions: [f64; 5],
    /// How far off its line a label may be pushed, as multiples of the distance that just clears
    /// the line (half the label's extent across the line, plus the stroke, plus
    /// [`LabelOptions::clearance`]). Both sides are tried, nearest first. Default `[1, 2, 4, 8]`.
    pub side_steps: [f64; 4],
    /// Rounds of placement. Round one ignores other labels, and each further round re-places
    /// every label against the others. Default 2. `0` places nothing.
    pub rounds: u32,
}

impl Default for LabelTuning {
    fn default() -> Self {
        LabelTuning {
            // Never at an end, where the port stack is.
            fractions: [0.5, 0.32, 0.68, 0.18, 0.82],
            side_steps: [1.0, 2.0, 4.0, 8.0],
            rounds: 2,
        }
    }
}

/// Crossing-minimisation heuristic and group lane budgets.
///
/// How hard the ordering phases work is in [`OrderTuning`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct OrderOptions {
    /// Which one-sided heuristic the sweeps sort by. See [`OrderHeuristic`].
    pub heuristic: OrderHeuristic,
    /// Crossings the lane pass may add to put each group ([`crate::Node::group`]) in a band of
    /// its own, as a percentage of the crossings ordering left, plus
    /// [`OrderTuning::group_lane_budget_floor`]. Default 0.
    ///
    /// A banding that costs nothing is always taken. For clear lanes, try 100 here, a floor of 8
    /// and [`OrderOptions::group_lane_area_percent`] of 50.
    pub group_lane_budget_percent: u32,
    /// How much wider the order axis may get, in percent, when the group bands are stacked into
    /// disjoint slots. Default 0, which leaves the bands where the coordinate phases put them.
    ///
    /// The stack is compressed to fit this budget, letting wide bands overlap at their edges.
    pub group_lane_area_percent: u32,
    /// Whether group lanes are a preference traded against the two budgets above
    /// ([`GroupLanes::Soft`], the default) or required swimlanes ([`GroupLanes::Strict`]).
    pub group_lanes: GroupLanes,
}

impl Default for OrderOptions {
    fn default() -> Self {
        OrderOptions {
            heuristic: OrderHeuristic::Alternate,
            group_lane_budget_percent: 0,
            group_lane_area_percent: 0,
            group_lanes: GroupLanes::Soft,
        }
    }
}

/// How the groups ([`crate::Node::group`]) of a layered drawing are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum GroupLanes {
    /// Bands of the order axis where affordable. A banding over either
    /// [`OrderOptions::group_lane_budget_percent`] or [`OrderOptions::group_lane_area_percent`]
    /// is not drawn.
    #[default]
    Soft,
    /// Swimlanes. Each group gets a disjoint strip of the order axis whatever it costs, and
    /// ungrouped nodes share one more. [`crate::Layout::lanes`] reports the strips. Disables
    /// wrapping ([`WrapOptions`]). Seeded nodes stay put and the strips are measured around them.
    ///
    /// ```
    /// use graphlay::{layout, GraphBuilder, GroupId, GroupLanes, LayoutOptions, Node, Size};
    ///
    /// let node = |group: u32, lane_key: Option<i64>| Node {
    ///     size: Size::new(80.0, 40.0), group: Some(GroupId(group)), lane_key, ..Node::default()
    /// };
    /// let mut b = GraphBuilder::new();
    /// let order = b.add_node(node(0, None));
    /// let late = b.add_node(node(0, Some(2)));
    /// let early = b.add_node(node(0, Some(1)));
    /// let (item, pack) = (b.add_node(node(1, None)), b.add_node(node(1, None)));
    /// for (s, t) in [(order, late), (order, early), (order, item), (item, pack), (late, pack)] {
    ///     b.connect(s, t);
    /// }
    /// let g = b.build();
    /// let mut o = LayoutOptions::layered_default();
    /// o.order.group_lanes = GroupLanes::Strict;
    /// let l = layout(&g, &o)?;
    ///
    /// // One lane per group, stacked down the page without overlapping...
    /// assert_eq!(l.lanes.len(), 2);
    /// assert!(l.lanes[0].rect.max.y <= l.lanes[1].rect.min.y);
    /// // ...every node inside its own group's lane...
    /// for (node, p) in g.nodes.iter().zip(&l.nodes) {
    ///     let lane = l.lanes.iter().find(|lane| lane.group == node.group).expect("a lane");
    ///     assert!(p.center.y - 20.0 >= lane.rect.min.y && p.center.y + 20.0 <= lane.rect.max.y);
    /// }
    /// // ...and, within a lane and a layer, in `lane_key` order.
    /// assert!(l.nodes[early.index()].center.y < l.nodes[late.index()].center.y);
    /// # Ok::<(), graphlay::LayoutError>(())
    /// ```
    Strict,
}

/// How hard the crossing-minimisation and lane phases work ([`Tuning::order`]).
///
/// The counts do not scale with graph size. A non-`Balanced` [`Effort`] overwrites the restart,
/// sweep, permutation, sifting and kick counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct OrderTuning {
    /// Ordering restarts. Default 9. The first three are structural (a DFS seed, its mirror, and
    /// a DFS from the last layer), the rest deterministic shuffles of the first.
    pub seeds: u32,
    /// Upper bound on forward and backward sweeps per restart. Default 8. A restart stops early
    /// after [`OrderTuning::sweep_stall`] sweeps without improvement.
    pub sweeps: u32,
    /// Consecutive non-improving sweeps after which a restart gives up. Default 2. `0` is read as
    /// `1`.
    pub sweep_stall: u32,
    /// Layers with 2 to this many nodes are ordered by trying every permutation. Default 5.
    pub exhaustive_layer_max: usize,
    /// Layers wider than this get the sifting pass. Default 2. The kicks only visit layers that
    /// sift, so raising it also removes layers from the local search.
    pub sifting_min_layer: usize,
    /// Upper bound on sifting rounds over the layer stack. Default 8. A round sifts every layer
    /// still moving and wakes the neighbours of those that moved. It stops early when a round
    /// moves nothing.
    pub sifting_rounds: u32,
    /// Work ceiling for sifting one layer, in arc pairs (about `e^2` for `e` arcs on its two
    /// sides). A heavier layer is left to the sweeps. Default 160000.
    pub sifting_max_work: u64,
    /// Arc count up to which sifting also runs inside each restart's sweeps, instead of only on
    /// the winning order. It multiplies sifting cost by the restart count. Default 20000.
    pub sifting_sweep_max_arcs: usize,
    /// Upper bound on kicks the iterated local search tries on the winning order. A kick reverses
    /// a short run of one layer, re-sifts, and is kept only if strictly better. Usually
    /// [`OrderTuning::kick_patience`] stops the pass first. Default 30000. `0` disables it.
    pub kick_rounds: u32,
    /// Consecutive rejected kicks per layer after which that layer leaves the kick pool. An
    /// accepted kick resets its layer and both neighbours. Default 64. `0` is read as `1`.
    pub kick_patience: u32,
    /// Longest run of a layer one kick may reverse. Each kick picks a length from 2 up to this.
    /// Default 8.
    pub kick_max_span: usize,
    /// Relaxation iterations that spread lane values from grouped to ungrouped nodes.
    /// Default 64.
    pub group_relax_iters: u32,
    /// Re-sort rounds of the group lane pass. Default 8.
    pub group_sort_rounds: u32,
    /// Crossings the lane pass may add on top of [`OrderOptions::group_lane_budget_percent`].
    /// Default 0. A nearly planar drawing has no crossings to take a percentage of, so this
    /// floor is what lets it get lanes.
    pub group_lane_budget_floor: usize,
    /// Barycentre sweeps over the banded order, each re-sorting every layer within its bands.
    /// Default 4.
    pub group_lane_sweeps: u32,
    /// Rounds of band-preserving adjacent swaps that win back crossings the banding cost.
    /// Default 3.
    pub group_lane_repair_rounds: u32,
    /// Rounds of adjacent band swaps looking for a band order with fewer crossings. Default 2.
    pub group_lane_order_rounds: u32,
    /// Rounds of the chain-lane pass, which moves a long edge's waypoints past their neighbours
    /// where that costs no crossing and lets the edge run on one lane. Default 3. `0` disables the
    /// pass.
    pub chain_lane_rounds: u32,
    /// On a graph with more routing waypoints than this, the chain-lane pass skips chains longer
    /// than [`OrderTuning::chain_lane_max_span`]. Default 2000.
    pub chain_lane_size_threshold: usize,
    /// Longest chain the chain-lane pass still works on above the size threshold. Default 12.
    pub chain_lane_max_span: usize,
    /// How many positions the chain-lane pass may move one waypoint in one go. A walk is kept
    /// whole if it opens the lane, else undone. Default 5.
    pub chain_lane_max_steps: usize,
    /// Rounds of the side-balancing pass, which mirrors branches off the main path to whichever
    /// side evens out the layers, where that costs no crossing. Default 4. `0` disables it.
    pub balance_rounds: u32,
}

impl Default for OrderTuning {
    fn default() -> Self {
        OrderTuning {
            seeds: 9,
            sweeps: 8,
            sweep_stall: 2,
            exhaustive_layer_max: 5,
            sifting_min_layer: 2,
            sifting_rounds: 8,
            sifting_max_work: 160_000,
            sifting_sweep_max_arcs: 20_000,
            kick_rounds: 30_000,
            kick_patience: 64,
            kick_max_span: 8,
            group_relax_iters: 64,
            group_sort_rounds: 8,
            group_lane_budget_floor: 0,
            group_lane_sweeps: 4,
            group_lane_repair_rounds: 3,
            group_lane_order_rounds: 2,
            chain_lane_rounds: 3,
            chain_lane_size_threshold: 2_000,
            chain_lane_max_span: 12,
            chain_lane_max_steps: 5,
            balance_rounds: 4,
        }
    }
}

/// How the two branches of a fork/join block are drawn against each other.
///
/// [`crate::metrics::LayoutMetrics::split_alignment`] measures how many blocks come out aligned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum BranchStyle {
    /// Keeps whatever placement produced. Suits graphs not read as blocks, such as dense
    /// hub-and-spoke graphs.
    #[default]
    Free,
    /// One branch of each block runs straight from fork to join and the others are pushed aside
    /// as skips. The straight branch is the heaviest, with ties going to the larger first node
    /// along the layout axis.
    StraightMain,
    /// The two branches sit at equal distance either side of the straight fork-join line. Only
    /// possible when both branches start in the same layer. Adds crossings, so no preset uses it.
    Symmetric,
}

/// Which optimiser places the order axis.
///
/// Both keep the crossing-minimised order and every gap in [`crate::Spacing`]. They differ in
/// what they optimise within those constraints. [`CoordOptions`] defaults to
/// [`Placement::NetworkSimplex`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
// Named apart from `layout::Placement` (where a node landed) in generated schemas and bindings.
#[cfg_attr(feature = "schemars", schemars(rename = "CrossPlacement"))]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    /// Brandes-Kopf, as in dagre and ELK, followed by the straightness snap
    /// ([`CoordTuning::snap_rounds`]). Fast and local.
    #[default]
    BrandesKoepf,
    /// The dot placement (Gansner et al. 1993, section 4.2): A network simplex minimises the
    /// total weighted cross-axis edge offset. Shorter edges and smaller area than Brandes-Kopf,
    /// with more bends. Large graphs fall back to Brandes-Kopf
    /// ([`CoordTuning::simplex_max_nodes`]).
    NetworkSimplex,
}

/// Cross-axis placement options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct CoordOptions {
    /// Which optimiser places the order axis. Default [`Placement::NetworkSimplex`].
    pub placement: Placement,
    /// Keep each branch off the main path in a band of its own, so one branch never slides into
    /// a sibling's row. Default `true`. Graphs without a main path (several flows, caller seeds,
    /// group lanes) are unaffected.
    pub branch_bands: bool,
    /// Centre a first- or last-layer terminal on the weighted median of its neighbours, so it
    /// sits over its main flow. Default `false`, `true` in [`LayoutOptions::process_map`].
    pub terminal_centering: bool,
    /// How a fork/join block's branches are drawn. See [`BranchStyle`]. Default
    /// [`BranchStyle::Free`], but [`LayoutOptions::layered_default`] and
    /// [`LayoutOptions::process_map`] use [`BranchStyle::StraightMain`].
    pub branch_style: BranchStyle,
}

impl Default for CoordOptions {
    fn default() -> Self {
        CoordOptions {
            // Readers preferred it to Brandes-Kopf in a blind pairwise test.
            placement: Placement::NetworkSimplex,
            branch_bands: true,
            terminal_centering: false,
            branch_style: BranchStyle::Free,
        }
    }
}

/// Iteration counts and budgets of the coordinate phases ([`Tuning::coords`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct CoordTuning {
    /// Auxiliary-graph size (layered nodes plus arcs) above which [`Placement::NetworkSimplex`]
    /// falls back to Brandes-Kopf. Default 10000.
    pub simplex_max_nodes: usize,
    /// Simplex iterations before [`Placement::NetworkSimplex`] stops with its best placement so
    /// far. Every iteration is feasible, so a capped solve is still a valid drawing. Default
    /// 50000.
    pub simplex_max_pivots: usize,
    /// Rounds of the walk over the simplex's optimal face that re-places each long edge, heaviest
    /// first, on the fewest lanes possible at the same cost. Default 2. `0` switches it off.
    /// Brandes-Kopf ignores it.
    pub simplex_face_rounds: u32,
    /// Rounds of the straightness snap after Brandes-Kopf. Default 8. The network simplex
    /// ignores it.
    pub snap_rounds: u32,
    /// How much wider, in percent, the drawing may get to draw its heaviest source-to-sink path
    /// as one straight line, but at least the widest node on the path. Default 15. `0` turns the
    /// pass off.
    pub main_path_room: u32,
    /// How much wider than the balanced placement one of Brandes-Kopf's four biased passes may
    /// be and still be chosen, in percent. Default 20.
    pub balance_width_slack: u32,
    /// Pixels of layer width one removed staircase step is worth when chain flattening pushes a
    /// node aside to free a lane. Default 300. `0` turns the phase off.
    pub flatten_room: u32,
    /// Rounds of channel pull-in, which moves a long edge's waypoints past a real node standing
    /// between them and their endpoints' line when crossings allow. Default 0 (off), 3 in
    /// [`LayoutOptions::process_map`].
    pub pullin_rounds: u32,
    /// Rounds of the [`BranchStyle`] pass. Each round walks every block innermost-first and
    /// stops early once a round aligns nothing new. Default 3.
    pub branch_rounds: u32,
}

impl Default for CoordTuning {
    fn default() -> Self {
        CoordTuning {
            // Solve time grows faster than the auxiliary graph, and placement runs several times
            // per layout.
            simplex_max_nodes: 10_000,
            simplex_max_pivots: 50_000,
            // A second round lets a light chain take the lane a heavy one vacated.
            simplex_face_rounds: 2,
            snap_rounds: 8,
            main_path_room: 15,
            balance_width_slack: 20,
            flatten_room: 300,
            pullin_rounds: 0,
            branch_rounds: 3,
        }
    }
}

/// Whether the cross axis is compacted after placement. Default [`Compaction::None`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Compaction {
    /// Leave the coordinates the placement passes produced.
    #[default]
    None,
    /// Order-preserving priority compaction: Pack each layer, align it to its weighted-median
    /// edges, then slide nodes toward their neighbours. Suits dense hub-and-spoke graphs.
    Priority,
}

/// Cross-axis compaction. Its iteration counts are in [`CompactionTuning`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct CompactionOptions {
    /// Whether the cross axis is compacted after placement.
    pub mode: Compaction,
}

impl Default for CompactionOptions {
    fn default() -> Self {
        CompactionOptions {
            mode: Compaction::None,
        }
    }
}

/// Iteration counts of [`Compaction::Priority`] ([`Tuning::compaction`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct CompactionTuning {
    /// Rounds of rigid per-layer offsetting. Default 24.
    pub offset_rounds: u32,
    /// Rounds of per-node refinement inside the packed envelope. Default 8.
    pub refine_rounds: u32,
}

impl Default for CompactionTuning {
    fn default() -> Self {
        CompactionTuning {
            offset_rounds: 24,
            refine_rounds: 8,
        }
    }
}

/// How node positions given by the caller ([`crate::Node::position`]) steer a layout. Default
/// [`SeedMode::Placement`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SeedMode {
    /// Seeds set the nodes' cross-axis positions, but layering and ordering run from scratch.
    /// A changed graph can land in other layers and orders, so the drawing may jump.
    #[default]
    Placement,
    /// When every node has a seed, read layers and order off the seeds, so filtered views of
    /// one graph keep each node's layer and order. Falls back to `Placement` when an edge would
    /// join two nodes of one layer.
    ///
    /// ```
    /// use graphlay::{layout, GraphBuilder, Layout, LayoutOptions, SeedMode, Size};
    ///
    /// let mut b = GraphBuilder::new();
    /// let [a, m, z] = [(); 3].map(|_| b.add_box(Size::new(80.0, 40.0)));
    /// b.connect(a, m);
    /// b.connect(m, z);
    /// b.connect(a, z);
    /// let full = b.build();
    /// let o = LayoutOptions::layered_default();
    /// let before = layout(&full, &o)?;
    ///
    /// // A filtered view without `a -> m`, seeded with the full drawing's positions.
    /// let mut view = full.clone();
    /// view.edges.remove(0);
    /// for (node, placed) in view.nodes.iter_mut().zip(&before.nodes) {
    ///     node.position = Some(placed.center);
    /// }
    /// let ranks = |l: &Layout| l.nodes.iter().map(|p| p.rank).collect::<Vec<_>>();
    /// let kept = layout(&view, &LayoutOptions { seed_mode: SeedMode::Structure, ..o })?;
    /// assert_eq!(ranks(&kept), ranks(&before)); // every node keeps its layer
    /// // Layered afresh, `m` would move into the first layer beside `a`.
    /// assert_eq!(layout(&view, &o)?.nodes[m.index()].rank, Some(0));
    /// # Ok::<(), graphlay::LayoutError>(())
    /// ```
    Structure,
}

/// Which edges of a cyclic graph are drawn against the flow. Default
/// [`CycleStrategy::MinimumReversal`].
///
/// A layered drawing draws one edge per cycle backwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum CycleStrategy {
    /// Reverse as little edge weight as possible (a heuristic minimum feedback arc set). Fewest
    /// crossings, but a rare step can be drawn before the steps that lead to it.
    #[default]
    MinimumReversal,
    /// Order the nodes inside a cycle by distance from where the flow enters it, so every
    /// reversed edge reads as a return to an earlier point. Can cost crossings on dense graphs.
    FollowFlow,
}

/// Cycle-breaking options. See [`CycleStrategy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct CycleOptions {
    /// Which edges of a cycle are drawn backwards.
    pub strategy: CycleStrategy,
}

/// Wrapping a long drawing into rows to fit a viewport shape. When a wrap pays is in
/// [`WrapTuning`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct WrapOptions {
    /// Width over height of the viewport the drawing is shown in. Default `0`, never wrap.
    pub viewport_aspect: f64,
}

impl Default for WrapOptions {
    fn default() -> Self {
        WrapOptions {
            viewport_aspect: 0.0,
        }
    }
}

/// When and how far a drawing is wrapped once [`WrapOptions::viewport_aspect`] asks for it
/// ([`Tuning::wrap`]).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct WrapTuning {
    /// Most rows a drawing is wrapped into. Default 4.
    pub max_rows: u32,
    /// Scale factor by which the drawing must grow in the viewport for a wrap to be taken.
    /// Default 1.2. Every edge across a cut gets a U-turn, which a small gain does not pay for.
    pub min_gain: f64,
    /// Most edges one cut may carry to the next row. Default 4.
    pub max_cut_edges: usize,
}

impl Default for WrapTuning {
    fn default() -> Self {
        WrapTuning {
            max_rows: 4,
            min_gain: 1.2,
            max_cut_edges: 4,
        }
    }
}

/// Everything that steers one layout run.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct LayoutOptions {
    /// Which way the layers advance.
    pub direction: Direction,
    /// Gaps and clearances.
    pub spacing: Spacing,
    /// Routing style, bundling and port choices.
    pub routing: Routing,
    /// Crossing-minimisation heuristic and lane budgets.
    pub order: OrderOptions,
    /// Cross-axis placement behaviour.
    pub coords: CoordOptions,
    /// Whether and how the cross axis is compacted afterwards.
    pub compaction: CompactionOptions,
    /// How self-loops are placed and drawn.
    pub self_loops: SelfLoopOptions,
    /// How edge labels are given room, and where they end up.
    pub labels: LabelOptions,
    /// How hard the engine works, as one dial. See [`Effort`].
    pub effort: Effort,
    /// Seed for the crossing minimisation's restarts and kicks. The same seed gives the same
    /// drawing. 32 bits so it survives JSON in JavaScript. Default [`DEFAULT_SEED`].
    pub seed: u32,
    /// Input size limits. Exceeding one is an error instead of unbounded time.
    pub limits: Limits,
    /// Wrapping a long drawing into rows. Off unless a viewport shape is given.
    #[serde(default)]
    pub wrap: WrapOptions,
    /// Which edges of a cyclic graph are drawn against the flow.
    #[serde(default)]
    pub cycles: CycleOptions,
    /// How node positions given as seeds steer the layout.
    #[serde(default)]
    pub seed_mode: SeedMode,
    /// The engine's internal iteration counts, budgets and thresholds. See [`Tuning`].
    #[serde(default)]
    pub tuning: Tuning,
}

/// Iteration counts, budgets and thresholds, tuned on the fixture corpus.
///
/// A non-`Balanced` [`LayoutOptions::effort`] overrides the ordering counts in [`OrderTuning`]
/// and [`CoordTuning::pullin_rounds`].
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct Tuning {
    /// The router's repair passes.
    pub routing: RoutingTuning,
    /// Crossing-minimisation and lane effort.
    pub order: OrderTuning,
    /// The coordinate phases' rounds and budgets.
    pub coords: CoordTuning,
    /// Priority compaction's rounds.
    pub compaction: CompactionTuning,
    /// Label placement's candidates and rounds.
    pub labels: LabelTuning,
    /// When and how far a drawing is wrapped.
    pub wrap: WrapTuning,
}

/// The default of [`LayoutOptions::seed`].
pub const DEFAULT_SEED: u32 = 0;

/// The 64-bit state a random stream starts from, for a seed and a salt that tells the streams
/// of one layout apart.
pub(crate) fn rng_state(seed: u32, salt: u64) -> u64 {
    u64::from(seed) ^ 0x9e37_79b9_7f4a_7c15 ^ salt
}

/// How hard the engine works, as one dial over the iteration counts. Default
/// [`Effort::Balanced`].
///
/// `Balanced` leaves [`OrderTuning`] and [`CoordTuning`] as given, so hand-tuned counts survive.
/// Any other value overwrites the counts in its [`Effort::budget`]. `Thorough` usually saves a
/// few crossings on dense graphs for two to three times the time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    /// Fewer restarts and sweeps, for interactive relayout of large graphs.
    Fast,
    /// The [`Tuning`] counts as given.
    #[default]
    Balanced,
    /// More restarts and sweeps, for a final rendering.
    Thorough,
    /// Exactly these counts.
    Custom(EffortBudget),
}

/// The iteration counts one [`Effort`] resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct EffortBudget {
    /// Ordering restarts ([`OrderTuning::seeds`]).
    pub order_seeds: u32,
    /// Sweeps per restart ([`OrderTuning::sweeps`]).
    pub order_sweeps: u32,
    /// Brute-force permutation cap ([`OrderTuning::exhaustive_layer_max`]).
    pub exhaustive_layer_max: usize,
    /// Sifting rounds per wide layer ([`OrderTuning::sifting_rounds`]).
    pub sifting_rounds: u32,
    /// Channel pull-in rounds, applied only when [`CoordTuning::pullin_rounds`] is non-zero.
    pub pullin_rounds: u32,
    /// Rejected kicks per layer before the local search gives up on it
    /// ([`OrderTuning::kick_patience`]).
    pub kick_patience: u32,
}

impl Default for EffortBudget {
    fn default() -> Self {
        Effort::Balanced.budget(0, 0)
    }
}

impl Effort {
    /// The iteration counts for a graph of `n` nodes and `m` edges.
    ///
    /// `n` and `m` are reserved and ignored. `Balanced` returns the [`OrderTuning`] defaults, and
    /// `Fast` and `Thorough` scale them down and up.
    pub fn budget(self, _n: usize, _m: usize) -> EffortBudget {
        let balanced = EffortBudget {
            order_seeds: 9,
            order_sweeps: 8,
            exhaustive_layer_max: 5,
            sifting_rounds: 8,
            pullin_rounds: 3,
            kick_patience: 64,
        };
        match self {
            Effort::Balanced => balanced,
            Effort::Fast => EffortBudget {
                order_seeds: 3,
                order_sweeps: 4,
                exhaustive_layer_max: 4,
                sifting_rounds: 1,
                pullin_rounds: 1,
                kick_patience: 8,
            },
            Effort::Thorough => EffortBudget {
                order_seeds: 36,
                order_sweeps: 12,
                exhaustive_layer_max: 6,
                sifting_rounds: 32,
                pullin_rounds: 4,
                kick_patience: 256,
            },
            Effort::Custom(b) => b,
        }
    }
}

impl LayoutOptions {
    /// Check that every numeric option is finite and non-negative.
    ///
    /// [`crate::layout()`] and [`crate::reroute()`] call this first.
    ///
    /// # Errors
    ///
    /// [`LayoutError::InvalidOption`] naming the first offending field.
    pub fn validate(&self) -> Result<(), crate::LayoutError> {
        let numbers: [(&'static str, f64); 30] = [
            ("spacing.layer_gap", self.spacing.layer_gap),
            ("spacing.node_gap", self.spacing.node_gap),
            ("spacing.dummy_gap", self.spacing.dummy_gap),
            ("spacing.port_gap", self.spacing.port_gap),
            ("spacing.port_stroke_clear", self.spacing.port_stroke_clear),
            ("spacing.stub", self.spacing.stub),
            ("spacing.bundle_lane_gap", self.spacing.bundle_lane_gap),
            ("spacing.self_loop_gap", self.spacing.self_loop_gap),
            ("spacing.edge_label_margin", self.spacing.edge_label_margin),
            (
                "spacing.edge_label_stack_gap",
                self.spacing.edge_label_stack_gap,
            ),
            ("spacing.reroute_node_gap", self.spacing.reroute_node_gap),
            ("spacing.isolated_gap", self.spacing.isolated_gap),
            ("spacing.compound_padding", self.spacing.compound_padding),
            ("spacing.compound_header", self.spacing.compound_header),
            (
                "tuning.routing.diagonal_clear",
                self.tuning.routing.diagonal_clear,
            ),
            (
                "tuning.routing.diagonal_detour_margin",
                self.tuning.routing.diagonal_detour_margin,
            ),
            (
                "tuning.routing.bundle_nest_gap",
                self.tuning.routing.bundle_nest_gap,
            ),
            (
                "tuning.routing.endpoint_snap",
                self.tuning.routing.endpoint_snap,
            ),
            (
                "tuning.routing.endpoint_snap_min_track",
                self.tuning.routing.endpoint_snap_min_track,
            ),
            ("routing.lane_gap", self.routing.lane_gap),
            (
                "tuning.routing.step_flatten",
                self.tuning.routing.step_flatten,
            ),
            (
                "tuning.routing.port_relax_clear",
                self.tuning.routing.port_relax_clear,
            ),
            (
                "tuning.routing.port_relax_step",
                self.tuning.routing.port_relax_step,
            ),
            (
                "tuning.routing.overlap_repair_step",
                self.tuning.routing.overlap_repair_step,
            ),
            ("self_loops.size", self.self_loops.size),
            ("self_loops.gap", self.self_loops.gap),
            ("self_loops.label_gap", self.self_loops.label_gap),
            ("labels.clearance", self.labels.clearance),
            ("wrap.viewport_aspect", self.wrap.viewport_aspect),
            ("tuning.wrap.min_gain", self.tuning.wrap.min_gain),
        ];
        match numbers.iter().find(|(_, v)| !(v.is_finite() && *v >= 0.0)) {
            Some(&(field, _)) => Err(crate::LayoutError::InvalidOption { field }),
            None => Ok(()),
        }
    }

    /// These options with [`LayoutOptions::effort`] applied. Unchanged for `Balanced`,
    /// otherwise the budget's counts overwrite the ordering and pull-in settings.
    pub fn with_effort_applied(&self) -> LayoutOptions {
        let mut o = *self;
        if self.effort == Effort::Balanced {
            return o;
        }
        let b = self.effort.budget(0, 0);
        o.tuning.order.seeds = b.order_seeds;
        o.tuning.order.sweeps = b.order_sweeps;
        o.tuning.order.exhaustive_layer_max = b.exhaustive_layer_max;
        o.tuning.order.sifting_rounds = b.sifting_rounds;
        o.tuning.order.kick_patience = b.kick_patience;
        if o.tuning.coords.pullin_rounds > 0 {
            o.tuning.coords.pullin_rounds = b.pullin_rounds;
        }
        o
    }
}

/// Input size limits.
///
/// All `None` by default. Set them when the input is untrusted (e.g., JSON from a wasm caller).
/// Exceeding a limit is [`LayoutError::TooLarge`], never a hang or a panic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    /// Largest accepted node count.
    pub max_nodes: Option<usize>,
    /// Largest accepted edge count.
    pub max_edges: Option<usize>,
    /// Largest accepted routing-waypoint count (one per layer a long edge passes), checked after
    /// layering. [`crate::LayoutError::TooLarge`] names it `"routing waypoints"`.
    pub max_dummies: Option<usize>,
}

impl Limits {
    /// Reject `g` if it exceeds `max_nodes` or `max_edges`.
    ///
    /// # Errors
    ///
    /// [`LayoutError::TooLarge`] for the first limit exceeded.
    pub fn check(&self, g: &Graph) -> Result<(), LayoutError> {
        if let Some(limit) = self.max_nodes {
            if g.nodes.len() > limit {
                return Err(LayoutError::TooLarge {
                    what: "nodes",
                    got: g.nodes.len(),
                    limit,
                });
            }
        }
        if let Some(limit) = self.max_edges {
            if g.edges.len() > limit {
                return Err(LayoutError::TooLarge {
                    what: "edges",
                    got: g.edges.len(),
                    limit,
                });
            }
        }
        Ok(())
    }
}

impl Default for LayoutOptions {
    fn default() -> Self {
        LayoutOptions::layered_default()
    }
}

impl LayoutOptions {
    /// Layered layout with orthogonal routing and ELK-like gaps, suited to Petri nets and other
    /// block-structured process models. This is also the [`Default`].
    pub fn layered_default() -> Self {
        LayoutOptions {
            direction: Direction::LeftRight,
            spacing: Spacing::layered(),
            routing: Routing {
                // Process models are read with edges entering from the sides (Petri nets) or the
                // top (process maps). This trades some bends for fewer crossings.
                port_sides: PortSides::FacingPreferred,
                ..Routing::default()
            },
            order: OrderOptions::default(),
            coords: CoordOptions {
                // A Petri net reads as nested XOR/AND blocks.
                branch_style: BranchStyle::StraightMain,
                ..CoordOptions::default()
            },
            compaction: CompactionOptions::default(),
            self_loops: SelfLoopOptions::default(),
            labels: LabelOptions::default(),
            effort: Effort::Balanced,
            seed: DEFAULT_SEED,
            limits: Limits::default(),
            wrap: WrapOptions::default(),
            cycles: CycleOptions::default(),
            seed_mode: SeedMode::default(),
            tuning: Tuning::default(),
        }
    }

    /// Process map (directly-follows graph): top to bottom, wider gaps, terminals centred on
    /// their main flow, and channel pull-in so a long edge's channel is not pushed out by its
    /// widest layer.
    pub fn process_map() -> Self {
        let mut o = LayoutOptions::layered_default();
        o.direction = Direction::TopBottom;
        o.spacing.layer_gap = 64.0;
        o.spacing.node_gap = 54.0;
        o.spacing.reroute_node_gap = 54.0;
        o.coords.terminal_centering = true;
        o.tuning.coords.pullin_rounds = 3;
        o
    }

    /// Dense hub-and-spoke graphs (e.g., an object-type graph): taller layers so long edges clear
    /// the intermediate rows, a tight order axis, and order-preserving compaction.
    pub fn hub_and_spoke() -> Self {
        let mut o = LayoutOptions::layered_default();
        o.spacing.layer_gap = 150.0;
        o.spacing.node_gap = 30.0;
        o.compaction.mode = Compaction::Priority;
        // No fork/join blocks here, and compaction runs later and would undo the alignment.
        o.coords.branch_style = BranchStyle::Free;
        // Every border of a hub is full, so let the router spread the ends freely.
        o.routing.port_sides = PortSides::Any;
        o
    }

    /// [`LayoutOptions::hub_and_spoke`] spacing with the process map's terminal centring, channel
    /// pull-in and reroute gap, for a dense graph that still has a direction of flow.
    pub fn hub_and_spoke_flow() -> Self {
        let flow = LayoutOptions::process_map();
        let mut o = LayoutOptions::hub_and_spoke();
        o.coords.terminal_centering = flow.coords.terminal_centering;
        o.tuning.coords.pullin_rounds = flow.tuning.coords.pullin_rounds;
        o.spacing.reroute_node_gap = flow.spacing.reroute_node_gap;
        o
    }

    /// The options a [`Preset`] stands for.
    pub fn preset(preset: Preset) -> Self {
        match preset {
            Preset::Layered => LayoutOptions::layered_default(),
            Preset::ProcessMap => LayoutOptions::process_map(),
            Preset::HubAndSpoke => LayoutOptions::hub_and_spoke(),
            Preset::HubAndSpokeFlow => LayoutOptions::hub_and_spoke_flow(),
        }
    }
}

/// A named starting point for [`LayoutOptions`], so a caller or a JSON request
/// ([`crate::json::LayoutRequest`]) picks the kind of drawing and changes only what it needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    /// [`LayoutOptions::layered_default`], for Petri nets.
    #[default]
    Layered,
    /// [`LayoutOptions::process_map`], for directly-follows graphs.
    ProcessMap,
    /// [`LayoutOptions::hub_and_spoke`], for dense relationship graphs.
    HubAndSpoke,
    /// [`LayoutOptions::hub_and_spoke_flow`], for dense graphs with a direction of flow.
    HubAndSpokeFlow,
}

impl Preset {
    /// Every preset, in declaration order.
    pub const ALL: [Preset; 4] = [
        Preset::Layered,
        Preset::ProcessMap,
        Preset::HubAndSpoke,
        Preset::HubAndSpokeFlow,
    ];
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Pins the presets' constants, so changing one means editing this test.
    #[test]
    fn presets_pin_their_constants() {
        let d = LayoutOptions::layered_default();
        assert_eq!(d.spacing.layer_gap, 54.0);
        assert_eq!(d.spacing.node_gap, 46.0);
        assert_eq!(d.spacing.dummy_gap, 30.0);
        assert_eq!(d.spacing.stub, 15.0);
        assert_eq!(d.spacing.port_gap, 18.0);
        assert_eq!(d.spacing.port_stroke_clear, 14.0);
        assert_eq!(d.spacing.bundle_lane_gap, 8.0);
        assert_eq!(d.spacing.self_loop_gap, 10.0);
        assert_eq!(d.spacing.isolated_gap, 46.0); // node_gap, for the isolated-node grid
        assert_eq!(d.spacing.edge_label_margin, 24.0);

        let f = LayoutOptions::process_map();
        assert_eq!((f.spacing.layer_gap, f.spacing.node_gap), (64.0, 54.0));
        // The router is not part of the preset.
        assert_eq!(f.routing.style, RoutingStyle::Orthogonal);
        assert_eq!(f.spacing.reroute_node_gap, 54.0);
        assert!(f.coords.terminal_centering);
        assert_eq!(f.tuning.coords.pullin_rounds, 3);

        // The block pass is on for the process-model presets.
        assert_eq!(d.coords.branch_style, BranchStyle::StraightMain);
        assert_eq!(f.coords.branch_style, BranchStyle::StraightMain);
        assert_eq!(
            LayoutOptions::hub_and_spoke().coords.branch_style,
            BranchStyle::Free
        );
        // The bare default leaves it off, so field-by-field options get no surprise pass.
        assert_eq!(CoordOptions::default().branch_style, BranchStyle::Free);

        assert_eq!(d.routing.port_sides, PortSides::FacingPreferred);
        assert_eq!(f.routing.port_sides, PortSides::FacingPreferred);
        assert_eq!(
            LayoutOptions::hub_and_spoke().routing.port_sides,
            PortSides::Any
        );
        assert_eq!(Routing::default().port_sides, PortSides::Any);

        let c = LayoutOptions::hub_and_spoke();
        assert_eq!((c.spacing.layer_gap, c.spacing.node_gap), (150.0, 30.0));
        assert_eq!(c.compaction.mode, Compaction::Priority);
        assert_eq!(c.routing.style, RoutingStyle::Orthogonal);
        assert!(!c.coords.terminal_centering);
        assert_eq!(c.tuning.coords.pullin_rounds, 0);
    }

    /// The ordering effort budgets. Each is a ceiling the pass stops short of.
    #[test]
    fn order_defaults_are_the_measured_effort_budgets() {
        let (u, o) = (OrderOptions::default(), OrderTuning::default());
        assert_eq!(o.seeds, 9); // 3 structural + 6 shuffled restarts
        assert_eq!(o.sweeps, 8); // sweep rounds per seed, at most
        assert_eq!(o.sweep_stall, 2); // ...and it gives up after two that pay nothing
        assert_eq!(u.heuristic, OrderHeuristic::Alternate);
        assert_eq!(o.exhaustive_layer_max, 5); // brute-force permutation cap
        assert_eq!(o.sifting_min_layer, 2); // every layer of three or more sifts
        assert_eq!(o.sifting_rounds, 8);
        assert_eq!(o.sifting_max_work, 160_000); // well above the corpus's heaviest layer
        assert_eq!(o.sifting_sweep_max_arcs, 20_000); // well above the corpus's biggest stack
        assert_eq!(o.kick_rounds, 30_000);
        assert_eq!(o.kick_patience, 64); // per layer, not per stack
        assert_eq!(o.kick_max_span, 8);
        assert_eq!(o.group_relax_iters, 64);
        assert_eq!(o.group_sort_rounds, 8);
        // Readers in a blind pairwise test preferred drawings without lanes.
        assert_eq!(u.group_lane_budget_percent, 0);
        assert_eq!(o.group_lane_budget_floor, 0);
        assert_eq!(o.group_lane_sweeps, 4);
        assert_eq!(o.group_lane_repair_rounds, 3);
        assert_eq!(o.group_lane_order_rounds, 2);
        assert_eq!(u.group_lane_area_percent, 0);
        assert_eq!(CoordTuning::default().snap_rounds, 8);
        assert_eq!(CompactionTuning::default().offset_rounds, 24);
        assert_eq!(CompactionTuning::default().refine_rounds, 8);
    }

    /// The span cap applies only past a size the fixture corpus does not reach.
    #[test]
    fn chain_lane_defaults() {
        let o = OrderTuning::default();
        assert_eq!(o.chain_lane_rounds, 3);
        assert_eq!(o.chain_lane_max_steps, 5);
        assert_eq!(o.chain_lane_size_threshold, 2_000);
        assert_eq!(o.chain_lane_max_span, 12);
    }

    #[test]
    fn routing_defaults_are_pinned() {
        let (u, r) = (Routing::default(), RoutingTuning::default());
        assert_eq!(r.diagonal_clear, 8.0);
        assert_eq!(r.diagonal_detour_margin, 12.0);
        assert_eq!(r.bundle_nest_gap, 12.0);
        assert_eq!(r.endpoint_snap, 6.0);
        assert_eq!(r.endpoint_snap_min_track, 25.0);
        assert_eq!(r.step_flatten, 1.0e6);
        assert_eq!(u.lane_gap, 8.0);
        assert_eq!(r.port_relax_clear, 10.0);
        assert_eq!(r.port_relax_step, 3.0);
        assert_eq!(r.port_relax_rounds, 48);
        assert_eq!(r.overlap_repair_rounds, 8);
        assert_eq!(r.overlap_repair_step, 8.0);
        assert!(u.bundle_parallel);
        assert_eq!(u.port_sides, PortSides::Any);
    }

    /// The effort budget does not vary with graph size, since scaling would move layouts.
    #[test]
    fn balanced_effort_is_the_defaults_at_every_size() {
        let o = OrderTuning::default();
        for (n, m) in [(0, 0), (10, 12), (4_000, 9_000), (250_000, 900_000)] {
            let b = Effort::Balanced.budget(n, m);
            assert_eq!(b.order_seeds, o.seeds);
            assert_eq!(b.order_sweeps, o.sweeps);
            assert_eq!(b.exhaustive_layer_max, o.exhaustive_layer_max);
            assert_eq!(b.sifting_rounds, o.sifting_rounds);
            assert_eq!(
                b.pullin_rounds,
                LayoutOptions::process_map().tuning.coords.pullin_rounds
            );
        }
        assert_eq!(EffortBudget::default(), Effort::Balanced.budget(0, 0));
        let custom = EffortBudget {
            order_seeds: 1,
            ..EffortBudget::default()
        };
        assert_eq!(Effort::Custom(custom).budget(99, 99), custom);
    }

    #[test]
    fn a_nan_or_negative_option_is_an_error_naming_it() {
        let mut o = LayoutOptions::default();
        assert_eq!(o.validate(), Ok(()));
        o.spacing.layer_gap = f64::NAN;
        assert_eq!(
            o.validate(),
            Err(crate::LayoutError::InvalidOption {
                field: "spacing.layer_gap"
            })
        );
        o.spacing.layer_gap = 10.0;
        o.tuning.wrap.min_gain = 1.2;
        o.tuning.routing.diagonal_clear = f64::INFINITY;
        assert_eq!(
            o.validate(),
            Err(crate::LayoutError::InvalidOption {
                field: "tuning.routing.diagonal_clear"
            })
        );
        o.tuning.routing.diagonal_clear = 8.0;
        o.tuning.wrap.min_gain = -1.0;
        let g = crate::Graph::default();
        assert_eq!(
            crate::layout(&g, &o).err(),
            Some(crate::LayoutError::InvalidOption {
                field: "tuning.wrap.min_gain"
            })
        );
        assert_eq!(LayoutOptions::process_map().validate(), Ok(()));
        assert_eq!(LayoutOptions::hub_and_spoke().validate(), Ok(()));
    }

    /// A non-`Balanced` effort overwrites the ordering counts but leaves a disabled pull-in off.
    #[test]
    fn effort_is_applied_over_the_ordering_counts() {
        let mut o = LayoutOptions::layered_default();
        o.tuning.order.seeds = 2;
        assert_eq!(o.with_effort_applied(), o);
        o.effort = Effort::Thorough;
        let t = o.with_effort_applied();
        assert_eq!(
            t.tuning.order.seeds,
            Effort::Thorough.budget(0, 0).order_seeds
        );
        assert_eq!(
            t.tuning.order.kick_patience,
            Effort::Thorough.budget(0, 0).kick_patience
        );
        assert_eq!(t.tuning.coords.pullin_rounds, o.tuning.coords.pullin_rounds);
        let mut p = LayoutOptions::process_map();
        p.effort = Effort::Fast;
        assert_eq!(p.with_effort_applied().tuning.coords.pullin_rounds, 1);
    }

    /// `SelfLoopOptions` has no `Spacing` to read, so the documented tie to the spacing defaults
    /// is checked here.
    #[test]
    fn self_loop_defaults_follow_the_spacing_defaults() {
        let l = SelfLoopOptions::default();
        assert_eq!(l.side, LoopSide::Auto);
        assert_eq!(l.size, Spacing::layered().stub);
        assert_eq!(l.gap, Spacing::layered().self_loop_gap);
        assert_eq!(l.label_gap, 4.0);
        assert_eq!(LayoutOptions::default().self_loops, l);
        for o in [LayoutOptions::process_map(), LayoutOptions::hub_and_spoke()] {
            assert_eq!(o.self_loops, l);
        }
    }

    /// A caller who never sets the seed gets the same layout every time.
    #[test]
    fn seed_default_is_the_engines_own_constant() {
        assert_eq!(LayoutOptions::default().seed, DEFAULT_SEED);
        assert_eq!(DEFAULT_SEED, 0);
        assert_eq!(rng_state(DEFAULT_SEED, 0), 0x9e37_79b9_7f4a_7c15);
        assert_ne!(rng_state(1, 0), rng_state(0, 0));
    }

    /// No limits by default. A limit that is set refuses instead of running.
    #[test]
    fn limits_refuse_oversized_inputs() {
        use crate::graph::{GraphBuilder, Node};
        use crate::Size;
        let mut b = GraphBuilder::new();
        let a = b.add_node(Node {
            size: Size::new(10.0, 10.0),
            ..Node::default()
        });
        let c = b.add_node(Node {
            size: Size::new(10.0, 10.0),
            ..Node::default()
        });
        b.connect(a, c);
        let g = b.build();

        assert!(Limits::default().check(&g).is_ok());
        assert_eq!(
            Limits {
                max_nodes: Some(1),
                ..Limits::default()
            }
            .check(&g)
            .unwrap_err(),
            LayoutError::TooLarge {
                what: "nodes",
                got: 2,
                limit: 1
            }
        );
        assert_eq!(
            Limits {
                max_edges: Some(0),
                ..Limits::default()
            }
            .check(&g)
            .unwrap_err(),
            LayoutError::TooLarge {
                what: "edges",
                got: 1,
                limit: 0
            }
        );
    }

    #[test]
    fn reroute_gap_ignores_the_compaction_preset() {
        let c = LayoutOptions::hub_and_spoke();
        assert_eq!(c.spacing.node_gap, 30.0);
        assert_eq!(c.spacing.reroute_node_gap, 46.0);
    }

    /// A misspelt field is an error. Every field is still optional.
    #[test]
    fn options_reject_unknown_fields() {
        assert!(serde_json::from_str::<LayoutOptions>(r#"{"routng":{}}"#).is_err());
        assert!(
            serde_json::from_str::<LayoutOptions>(r#"{"routing":{"styl":"diagonal"}}"#).is_err()
        );
        // A tuning knob outside its `tuning` group is unknown too.
        assert!(serde_json::from_str::<LayoutOptions>(r#"{"order":{"seeds":3}}"#).is_err());
        assert!(
            serde_json::from_str::<LayoutOptions>(r#"{"tuning":{"order":{"seedz":3}}}"#).is_err()
        );
        assert!(serde_json::from_str::<LayoutOptions>(r#"{"effort":{"custom":{"x":1}}}"#).is_err());

        assert_eq!(
            serde_json::from_str::<LayoutOptions>("{}").unwrap(),
            LayoutOptions::default()
        );
        let t: LayoutOptions = serde_json::from_str(
            r#"{"routing":{"style":"diagonal"},"tuning":{"order":{"seeds":3}}}"#,
        )
        .unwrap();
        assert_eq!(t.routing.style, RoutingStyle::Diagonal);
        assert_eq!(t.tuning.order.seeds, 3);
        for o in [
            LayoutOptions::layered_default(),
            LayoutOptions::process_map(),
            LayoutOptions::hub_and_spoke(),
        ] {
            let json = serde_json::to_string(&o).unwrap();
            assert_eq!(serde_json::from_str::<LayoutOptions>(&json).unwrap(), o);
        }
    }
}

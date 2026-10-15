//! Quality metrics for a laid-out graph.
//!
//! [`measure`] takes a [`Graph`], the [`LayoutOptions`] it was laid out with and the resulting
//! [`Layout`], and returns [`LayoutMetrics`]. Use it to compare two layout engines, or two
//! versions of one, on the same fixtures. [`LayoutMetrics::score`] folds the metrics into one
//! number with [`ScoreWeights`], and [`compare`] measures how far nodes moved between two layouts.
//! Everything is deterministic and serialisable, so metrics can be stored as a baseline.
//!
//! A node's box is its `width` x `height` rectangle, which also holds an ellipse node. An edge is
//! routed when it has at least two points. Unrouted edges are skipped by the geometric metrics and
//! counted in [`LayoutMetrics::unrouted_edges`].
//!
//! See `docs/quality.md` for what the numbers mean and how the score weights were calibrated.

use crate::geometry::fp;
use serde::{Deserialize, Serialize};

use crate::geometry::{hit, DEGENERATE, DEGENERATE_SQ, EPS};
use crate::graph::Graph;
use crate::layout::Layout;
use crate::options::{Direction, LayoutOptions};

mod flow;
mod labels;
mod lanes;
mod paths;
mod ports;
mod stability;

pub use stability::{compare, StabilityMetrics};

/// Points closer than this (px) count as one point when discounting intersections at a shared node.
const SHARED_ENDPOINT_TOL: f64 = 1.0;
/// Inset (px) for node-hit tests, so an edge along a border does not count as passing through.
const NODE_HIT_TOL: f64 = 1.0;
/// Two node boxes must interpenetrate by more than this (px) on both axes to count as overlapping.
const NODE_OVERLAP_TOL: f64 = 0.5;
/// Perpendicular distance (px) below which two segments are treated as collinear.
const COLLINEAR_TOL: f64 = 0.5;
/// Collinear stretch (px) that two segments of different edges must share to count as an overlap.
const OVERLAP_MIN_LEN: f64 = 2.0;
/// Direction change (degrees) above which a polyline join counts as a bend.
const BEND_MIN_DEG: f64 = 5.0;
/// A forward edge must advance at least this much (px) along the layout axis.
const BACKWARD_TOL: f64 = 1.0;
/// How far (px) a route endpoint may sit from its node's border before it counts as detached.
const ENDPOINT_OUTLINE_TOL: f64 = 0.5;
/// Unscaled node label font size (px), as drawn by `examples/eval.rs`.
const LABEL_FONT_PX: f64 = 12.5;
/// The viewport [`LayoutMetrics::fit_font_px`] scales into, a laptop browser window.
const FIT_VIEWPORT: (f64, f64) = (1600.0, 900.0);
/// Smallest readable label size on screen (px).
const READABLE_FONT_PX: f64 = 8.0;
/// Aspect ratio, either way round, beyond which the score charges for elongation.
const COMFORTABLE_ASPECT: f64 = 3.0;
/// Label boxes must interpenetrate by more than this (px) on both axes to overlap, the same
/// tolerance as [`NODE_OVERLAP_TOL`].
const LABEL_OVERLAP_TOL: f64 = 0.5;
/// How far (px) a node may sit off the main path's median line and still count as on it. Matches
/// [`crate::splits::ALIGN_TOL`] so "on the line" means one thing.
const MAIN_PATH_TOL: f64 = 2.0;

/// Tallest perpendicular jog that [`LayoutMetrics::short_steps`] counts as a step.
const STEP_MAX_PX: f64 = 12.0;
/// Crossings shallower than this (degrees) slow readers down, per Huang, Hong and Eades.
const SHALLOW_CROSSING_DEG: f64 = 30.0;
/// Segments whose undirected directions differ by less than this (degrees) count as parallel.
const PARALLEL_TOL_DEG: f64 = 10.0;
/// Multiple of the thicker of two strokes below which a parallel pair reads as one thick line.
const PARALLEL_SEPARATION_K: f64 = 1.5;

/// Measured quality of one laid-out graph.
///
/// Lower is better for every count and length. `detour_ratio` and the shares `split_alignment`,
/// `main_path_aligned_fraction`, `flow_consistency` and `lane_purity` are best at `1.0`, and the
/// `min_*` clearances and crossing angles are better when larger. An `Option` field is `None`
/// when the metric is undefined, e.g., [`LayoutMetrics::min_node_gap`] with fewer than two nodes.
/// Fields marked `#[serde(default)]` may be missing from a baseline file, so older baselines
/// still load.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct LayoutMetrics {
    /// Number of nodes.
    pub node_count: usize,
    /// Number of edges, self-loops included.
    pub edge_count: usize,
    /// Edges with fewer than two polyline points.
    pub unrouted_edges: usize,

    /// Proper intersections between segments of two different edges, once per segment pair.
    /// Touching and collinear overlap do not count, nor does an intersection within about 1px of
    /// an endpoint the two edges share.
    pub crossings: usize,
    /// Segment pairs of different edges that are collinear within 0.5px and share more than 2px.
    pub overlaps: usize,
    /// (edge, node) pairs where the edge passes through a node it does not attach to, with the
    /// box or ellipse inset by 1px. Counted once per pair.
    pub node_edge_hits: usize,
    /// Pairs of node boxes that overlap by more than 0.5px on both axes.
    pub node_overlaps: usize,
    /// Route endpoints more than 0.5px inside or outside the outline of their node. Routes run
    /// border to border, so any non-zero value is a bug. Ellipses are measured along the ray from
    /// the centre, which is exact for a circle.
    #[serde(default)]
    pub endpoint_off_outline: usize,
    /// Largest such distance (px). `None` when there are no routed endpoints.
    #[serde(default)]
    pub max_endpoint_outline_gap: Option<f64>,

    /// Direction changes greater than 5 degrees, summed over all edges.
    pub bends: usize,
    /// Number of edges with at least one such bend.
    pub edges_with_bends: usize,
    /// Bends beyond the minimum an orthogonal route needs for the borders it leaves and enters,
    /// summed over routed orthogonal edges that are not self-loops.
    ///
    /// The minimum is 0 for facing borders that share a height, 1 for perpendicular borders (an
    /// L), 2 for a Z or a U, and up to 4 for a route that must turn back on itself. Ports may
    /// slide along their borders, so this is never negative. Corners forced by obstacles count
    /// too. Diagonal routes are skipped, and the choice of border is judged by
    /// [`LayoutMetrics::perpendicular_ends`] instead.
    #[serde(default)]
    pub excess_bends: usize,

    /// Sum of all routed polyline lengths (px).
    pub total_edge_length: f64,
    /// `total_edge_length` per routed edge, 0 when there are none.
    pub mean_edge_length: f64,
    /// Longest routed polyline (px).
    pub max_edge_length: f64,
    /// Total polyline length over total straight-line distance between the same endpoints, for
    /// routed edges with distinct endpoints. `1.0` means every edge is straight, and also when
    /// there is nothing to measure.
    pub detour_ratio: f64,

    /// Edges whose target centre is not more than 1px after the source centre along the layout
    /// direction. Self-loops are excluded.
    pub backward_edges: usize,
    /// Number of self-loops.
    pub self_loops: usize,
    /// Distinct `(from, to)` pairs with more than one edge, self-loops excluded.
    pub parallel_edge_groups: usize,

    /// Width of the bounding box of all node boxes and route points (px).
    pub width: f64,
    /// Height of that bounding box (px).
    pub height: f64,
    /// `width * height` (px^2).
    pub area: f64,
    /// `width / height`, or `0.0` for an empty or degenerate box.
    pub aspect_ratio: f64,

    /// Smallest signed gap between two node boxes (px), negative when they overlap. `None` with
    /// fewer than two nodes.
    pub min_node_gap: Option<f64>,
    /// Smallest distance (px) from an edge to a node it does not attach to, `0.0` when an edge
    /// passes through a node. `None` when there is no such pair. Ellipses use their bounding box,
    /// so for them this is a lower bound.
    pub min_edge_node_clearance: Option<f64>,
    /// Smallest distance (px) between two edge ends on the same node. Small values mean crowded
    /// ports. Ends at one point ([`LayoutMetrics::converged_ends`]) are one port. `None` when no
    /// node has two separate ends.
    pub min_port_gap: Option<f64>,

    /// Size (px) of a 12.5px label once the drawing is scaled to fit 1600x900, i.e.,
    /// `12.5 * min(1600/width, 900/height).min(1.0)`. Higher is better, and below about 7px labels
    /// are unreadable whatever the other metrics say. `0.0` when nothing was drawn.
    #[serde(default)]
    pub fit_font_px: f64,
    /// Total node box area over bounding-box area, clamped to `0..=1`. Low means mostly
    /// whitespace. `0.0` when nothing was drawn.
    #[serde(default)]
    pub density: f64,
    /// `max(aspect, 1/aspect)` with `aspect = width / height`, so `1.0` for a square. `0.0` when
    /// nothing was drawn.
    #[serde(default)]
    pub aspect_penalty: f64,
    /// Fork/join pairs in the graph (see [`LayoutMetrics::split_alignment`]).
    #[serde(default)]
    pub split_pairs: usize,
    /// Fork/join pairs drawn as an aligned block.
    #[serde(default)]
    pub split_aligned: usize,
    /// `split_aligned / split_pairs`, or `1.0` when there are no pairs.
    ///
    /// A fork/join pair is a node `f`, two of its successors `b1` and `b2`, and the nearest node
    /// `j` reachable from both. It is aligned when, within 2px, either:
    ///
    /// * `f`, one of the branches and `j` sit on one line along the layout direction, or
    /// * `b1` and `b2` mirror each other about the `f`-`j` line.
    ///
    /// Forks with more than 6 outgoing edges are skipped, and the join search stops 12 edges from
    /// each branch. The layered engine optimises the same definition
    /// ([`crate::options::BranchStyle`]). A tidy tree has no joins, so it always reports
    /// `split_pairs == 0`.
    #[serde(default)]
    pub split_alignment: f64,

    /// Edges with a [`crate::Edge::label_size`] that got a
    /// [`crate::layout::EdgeRoute::label_anchor`]. The label counts below are `0` when this is.
    #[serde(default)]
    pub label_count: usize,
    /// `label_label_overlaps + label_node_overlaps + label_edge_hits`. This total enters
    /// [`LayoutMetrics::score`], weighted like [`LayoutMetrics::node_edge_hits`].
    #[serde(default)]
    pub label_overlaps: usize,
    /// Label box pairs overlapping by more than 0.5px on both axes.
    #[serde(default)]
    pub label_label_overlaps: usize,
    /// `(label, node)` pairs overlapping by more than 0.5px on both axes. The label's own source
    /// and target count too.
    #[serde(default)]
    pub label_node_overlaps: usize,
    /// `(label, edge)` pairs where another edge's polyline enters the label box. The label's own
    /// edge is excluded.
    #[serde(default)]
    pub label_edge_hits: usize,
    /// Smallest distance (px) from a label box to another label, a node or another edge. `0.0`
    /// when something overlaps a label, `None` when there is nothing to measure.
    #[serde(default)]
    pub min_label_clearance: Option<f64>,

    /// Weighted crossings (Mennens et al.), the sum of `w(e) * w(f)` over the segment pairs
    /// [`LayoutMetrics::crossings`] counts.
    #[serde(default)]
    pub weighted_crossings: f64,
    /// The same sum over [`crate::Edge::thickness`] instead of weight.
    #[serde(default)]
    pub thickness_weighted_crossings: f64,

    /// Nodes on the heaviest source-to-sink path, in the orientation the layering used
    /// ([`crate::Layout::reversed`]). Sources are nodes without incoming edges plus
    /// [`crate::RankConstraint::First`] nodes. `0` when there is no such path.
    #[serde(default)]
    pub main_path_len: usize,
    /// Share of main-path nodes within 2px of the path's median cross coordinate. `1.0` for a
    /// path shorter than two nodes. It catches a zig-zagging main path, which the local
    /// [`LayoutMetrics::split_alignment`] misses.
    #[serde(default)]
    pub main_path_aligned_fraction: f64,
    /// Mean absolute cross-axis deviation from that median over the path's extent along the
    /// layout axis. `0.0` when that extent is zero.
    #[serde(default)]
    pub main_path_deviation: f64,

    /// Burattin et al.'s `M-E2`. Each segment gets the two compass directions nearest its
    /// heading, and this is the share of drawn length in the most common pair. `1.0` when all
    /// segments agree, `0.0` when nothing is drawn.
    #[serde(default)]
    pub flow_consistency: f64,
    /// The same share with each length multiplied by its [`crate::Edge::weight`].
    #[serde(default)]
    pub flow_consistency_weighted: f64,

    /// Smallest acute crossing angle (degrees). Shallow crossings are harder to trace. `None`
    /// when nothing crosses.
    #[serde(default)]
    pub crossing_angle_min: Option<f64>,
    /// Mean acute crossing angle (degrees). `None` when nothing crosses.
    #[serde(default)]
    pub crossing_angle_mean: Option<f64>,
    /// Crossings with an acute angle below 30 degrees.
    #[serde(default)]
    pub crossings_below_30deg: usize,

    /// Length (px) over which two different edges run within 10 degrees of parallel and closer
    /// than `1.5 * max(thickness)`, so they read as one thick line.
    #[serde(default)]
    pub parallel_run_length: f64,

    /// Segments of edges that touch none of a fork/join block's four nodes but pass through the
    /// block's bounding box, counted per block.
    #[serde(default)]
    pub block_intrusion: usize,

    /// Mean share of each [`crate::Node::group`]'s cross-axis span that no other group covers.
    /// `1.0` when each group has its own lane, or with fewer than two groups.
    #[serde(default)]
    pub lane_purity: f64,
    /// Nodes of another group inside a group's interquartile cross-axis band, summed over
    /// groups. Ungrouped nodes do not count.
    #[serde(default)]
    pub lane_intruders: usize,

    /// Coefficient of variation of node-box area per grid cell, with about one cell per node.
    /// `0.0` is perfectly even.
    #[serde(default)]
    pub whitespace_balance: f64,

    /// Weighted mean over routed edges of the total absolute turn angle (degrees) along each
    /// polyline.
    #[serde(default)]
    pub path_turn_mean: f64,
    /// Largest single turn (degrees) at any polyline joint.
    #[serde(default)]
    pub path_turn_max: f64,

    /// Perpendicular jogs shorter than 12px between two runs, summed over routed edges that are
    /// not self-loops. Such a step is never a detour and reads as jitter, so the target is zero.
    #[serde(default)]
    pub short_steps: usize,
    /// How often a route moving one way along an axis later turns back, on both axes, summed
    /// over routed edges that are not self-loops. "Right, up, right, down, right" has one. Detours
    /// around nodes count too, so this is not a pure defect count.
    #[serde(default)]
    pub route_reversals: usize,
    /// Consecutive jogs in the same cross direction along a route's main axis, summed over routed
    /// edges that are not self-loops. One level change needs one jog. Jogs around obstacles count
    /// too, so the target is low, not zero.
    #[serde(default)]
    pub staircase_steps: usize,

    /// Route ends on a border across the layout axis, e.g., the top or bottom border in a
    /// left-to-right drawing, summed over routed edges that are not self-loops. Not a defect, but
    /// each is an edge arriving from an odd direction, so it should be low.
    #[serde(default)]
    pub perpendicular_ends: usize,
    /// [`LayoutMetrics::backward_edges`] whose target is more breadth-first hops from the source
    /// nodes than their source is. A loop's closing edge is fine backwards, but this one means a
    /// loop body was drawn before its entry. `0` for a graph without a source node.
    #[serde(default)]
    pub backward_nonloop_edges: usize,
    /// Route ends more than a third of the way from the middle of their border to its corner,
    /// summed over routed edges that are not self-loops. Diagonal ends are skipped. Spread ports on
    /// a busy border count too, so the target is low, not zero.
    #[serde(default)]
    pub off_centre_ends: usize,
    /// Route ends whose first segment heads back across the border it attaches to, e.g., an
    /// edge that leaves the right face of a box and turns left under it. Summed over routed
    /// edges that are not self-loops. A defect, so the target is zero.
    #[serde(default)]
    pub inward_ends: usize,
    /// Route ends that share their attachment point with another end on the same node. Not a
    /// defect, and the shared trunk is excluded from [`LayoutMetrics::overlaps`],
    /// [`LayoutMetrics::parallel_run_length`] and [`LayoutMetrics::min_port_gap`].
    #[serde(default)]
    pub converged_ends: usize,
}

/// Relative importance of each metric in [`LayoutMetrics::score`].
///
/// Edges through nodes and overlapping nodes cost most, then crossings, then bends, then
/// compactness. See `docs/quality.md` for the calibration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreWeights {
    /// Per crossing.
    pub crossing: f64,
    /// Per collinear edge overlap.
    pub overlap: f64,
    /// Per (edge, node) pair where the edge passes through the node.
    pub node_edge_hit: f64,
    /// Per overlapping pair of nodes.
    pub node_overlap: f64,
    /// Per bend.
    pub bend: f64,
    /// Per backward edge.
    pub backward_edge: f64,
    /// Per `10_000 px^2` of drawing area per node.
    pub area: f64,
    /// Per `100 px` of total edge length per node.
    pub edge_length: f64,
    /// Per unit of `(8 - fit_font_px).max(0)^2`.
    #[serde(default = "default_fit_font_weight")]
    pub fit_font: f64,
    /// Per unit of `(aspect_penalty - 3).max(0)`.
    #[serde(default = "default_aspect_weight")]
    pub aspect: f64,
    /// Per fork/join pair the drawing did not line up.
    #[serde(default = "default_split_weight")]
    pub split_misaligned: f64,
    /// Per [`LayoutMetrics::label_overlaps`]. Equal to [`ScoreWeights::node_edge_hit`] by default.
    #[serde(default = "default_label_overlap_weight")]
    pub label_overlap: f64,
    /// Per unit of `main_path_len * (1 - main_path_aligned_fraction)`, i.e., per main-path node
    /// off the line.
    #[serde(default = "default_main_path_weight")]
    pub main_path: f64,
    /// Per unit of `routed_edges * (1 - flow_consistency)`.
    ///
    /// The raw term counts every routed edge, so its default is a fifth of `split_misaligned` to
    /// keep the two contributions on one scale. See `docs/quality.md`.
    #[serde(default = "default_flow_weight")]
    pub flow: f64,
}

/// Serde defaults, so a weights file without these terms gets their real weight.
fn default_fit_font_weight() -> f64 {
    10.0
}
fn default_aspect_weight() -> f64 {
    8.0
}
fn default_label_overlap_weight() -> f64 {
    25.0
}
fn default_main_path_weight() -> f64 {
    10.0
}
fn default_flow_weight() -> f64 {
    2.0
}
fn default_split_weight() -> f64 {
    10.0
}

impl Default for ScoreWeights {
    /// [`ScoreWeights::balanced`].
    fn default() -> Self {
        Self::balanced()
    }
}

impl ScoreWeights {
    /// Readability first. Crossings and edges through nodes cost far more than area, and label
    /// size and aspect weigh more than in [`ScoreWeights::balanced`].
    pub fn readability() -> Self {
        ScoreWeights {
            crossing: 10.0,
            overlap: 4.0,
            node_edge_hit: 25.0,
            node_overlap: 50.0,
            bend: 1.0,
            backward_edge: 0.5,
            area: 1.0,
            edge_length: 1.0,
            fit_font: 20.0,
            aspect: 12.0,
            split_misaligned: 12.0,
            label_overlap: 25.0,
            main_path: 12.0,
            flow: 3.0,
        }
    }

    /// The default weighting. Area and edge length weigh about as much as crossings, so a much
    /// more compact layout can beat one with a few crossings fewer. Overlaps and edges through
    /// nodes still dominate.
    ///
    /// The label size, aspect and split weights make a drawing that is bad at one of them pay
    /// about what a drawing with many crossings pays (see `docs/quality.md`).
    pub fn balanced() -> Self {
        ScoreWeights {
            crossing: 10.0,
            overlap: 4.0,
            node_edge_hit: 25.0,
            node_overlap: 50.0,
            bend: 1.0,
            backward_edge: 0.5,
            area: 12.0,
            edge_length: 12.0,
            fit_font: 10.0,
            aspect: 8.0,
            split_misaligned: 10.0,
            label_overlap: 25.0,
            main_path: 10.0,
            flow: 2.0,
        }
    }
}

impl LayoutMetrics {
    /// A weighted sum of the metrics, lower being better.
    ///
    /// It is a summary for spotting regressions, and the individual metrics say what changed.
    /// Counts enter unnormalised, so scores are only comparable for the same graph. Area and edge
    /// length are scaled by node count, as `area / (10_000 * n)` and
    /// `total_edge_length / (100 * n)`.
    ///
    /// These terms can penalise a drawing with no defects:
    ///
    /// * `fit_font`: `(8 - fit_font_px).max(0)^2`, and zero for a drawing with no extent.
    /// * `aspect`: `(aspect_penalty - 3).max(0)`, so up to 3:1 is free.
    /// * `split_misaligned`: `split_pairs - split_aligned`.
    /// * `label_overlap`: [`LayoutMetrics::label_overlaps`].
    /// * `main_path`: `main_path_len * (1 - main_path_aligned_fraction)`.
    /// * `flow`: `routed_edges * (1 - flow_consistency)`.
    ///
    /// See `docs/quality.md` for how the weights were calibrated.
    pub fn score(&self, weights: &ScoreWeights) -> f64 {
        let n = self.node_count.max(1) as f64;
        let area_term = self.area / (10_000.0 * n);
        let length_term = self.total_edge_length / (100.0 * n);
        // `fit_font_px` is 0 for an empty drawing, which must not get the full penalty.
        let fit_term = if self.fit_font_px > 0.0 {
            fp::sq((READABLE_FONT_PX - self.fit_font_px).max(0.0))
        } else {
            0.0
        };
        let aspect_term = (self.aspect_penalty - COMFORTABLE_ASPECT).max(0.0);
        let split_term = self.split_pairs.saturating_sub(self.split_aligned) as f64;
        let main_path_term = self.main_path_len as f64 * (1.0 - self.main_path_aligned_fraction);
        let routed = self.edge_count.saturating_sub(self.unrouted_edges) as f64;
        let flow_term = routed * (1.0 - self.flow_consistency);
        weights.crossing * self.crossings as f64
            + weights.overlap * self.overlaps as f64
            + weights.node_edge_hit * self.node_edge_hits as f64
            + weights.node_overlap * self.node_overlaps as f64
            + weights.bend * self.bends as f64
            + weights.backward_edge * self.backward_edges as f64
            + weights.area * area_term
            + weights.edge_length * length_term
            + weights.fit_font * fit_term
            + weights.aspect * aspect_term
            + weights.split_misaligned * split_term
            + weights.label_overlap * self.label_overlaps as f64
            + weights.main_path * main_path_term
            + weights.flow * flow_term
    }
}

#[derive(Debug, Clone, Copy)]
struct NodeBox {
    cx: f64,
    cy: f64,
    hw: f64,
    hh: f64,
    ellipse: bool,
}

impl NodeBox {
    fn left(&self) -> f64 {
        self.cx - self.hw
    }
    fn right(&self) -> f64 {
        self.cx + self.hw
    }
    fn top(&self) -> f64 {
        self.cy - self.hh
    }
    fn bottom(&self) -> f64 {
        self.cy + self.hh
    }
}

/// One routed edge with its endpoint node indices and polyline.
#[derive(Debug, Clone)]
struct EdgeGeom {
    from: usize,
    to: usize,
    pts: Vec<[f64; 2]>,
    /// Index into [`Problem::edges`], which survives dropping the unrouted edges.
    id: usize,
    weight: f64,
    thickness: f64,
}

impl EdgeGeom {
    /// `(node, attachment point)` for both ends.
    fn ends(&self) -> [(usize, [f64; 2]); 2] {
        [
            (self.from, self.pts[0]),
            (self.to, self.pts[self.pts.len() - 1]),
        ]
    }
}

/// One reserved edge-label box.
#[derive(Debug, Clone, Copy)]
struct LabelBox {
    /// Index into [`Problem::edges`].
    edge: usize,
    center: [f64; 2],
    half: [f64; 2],
}

impl LabelBox {
    /// The label as a [`NodeBox`], so the rectangle tests apply unchanged.
    fn as_box(&self) -> NodeBox {
        NodeBox {
            cx: self.center[0],
            cy: self.center[1],
            hw: self.half[0],
            hh: self.half[1],
            ellipse: false,
        }
    }
}

/// Flat view of a graph and its layout that every metric reads. Tests build one by hand.
struct Problem {
    /// One per node placed in the layout, in node order.
    boxes: Vec<NodeBox>,
    /// Nodes in the graph, placed or not.
    node_count: usize,
    edges: Vec<(usize, usize)>,
    /// Polyline per edge, empty where the edge was not routed.
    routes: Vec<Vec<[f64; 2]>>,
    /// Whether the layout axis is `x`.
    left_right: bool,
    weights: Vec<f64>,
    thickness: Vec<f64>,
    /// Label boxes of edges that asked for a label and got an anchor, in edge order.
    labels: Vec<LabelBox>,
    groups: Vec<Option<u32>>,
    /// Per edge, whether [`crate::Layout::reversed`] lists it, so the main-path search uses the
    /// engine's orientation.
    reversed: Vec<bool>,
    /// [`crate::RankConstraint::First`] nodes, which are main-path sources even with incoming
    /// edges.
    rank_first: Vec<bool>,
    /// [`crate::Node::parent`] per node. An edge into a compound's child has to cross the
    /// compound's box, so that is not a hit.
    parents: Vec<Option<usize>>,
}

/// Whether `anc` is `node` or one of its ancestors in `parents`.
fn encloses(parents: &[Option<usize>], anc: usize, node: usize) -> bool {
    let mut at = Some(node);
    // A parent chain cannot be longer than the node count.
    for _ in 0..=parents.len() {
        match at {
            Some(x) if x == anc => return true,
            Some(x) => at = parents.get(x).copied().flatten(),
            None => return false,
        }
    }
    false
}

/// Measures `layout` as a layout of `graph` with `options`.
///
/// Node `i` is matched to `layout.nodes[i]` and edge `i` to `layout.edges[i]`. Missing entries
/// and polylines under two points are counted in [`LayoutMetrics::unrouted_edges`] and skipped,
/// so a partial layout can still be measured. Only [`LayoutOptions::direction`] is read from
/// `options`.
pub fn measure(graph: &Graph, options: &LayoutOptions, layout: &Layout) -> LayoutMetrics {
    let problem = Problem {
        boxes: graph
            .nodes
            .iter()
            .zip(layout.nodes.iter())
            .map(|(n, p)| NodeBox {
                cx: p.center.x,
                cy: p.center.y,
                hw: n.size.width / 2.0,
                hh: n.size.height / 2.0,
                ellipse: n.shape == crate::geometry::Shape::Ellipse,
            })
            .collect(),
        node_count: graph.nodes.len(),
        edges: graph
            .edges
            .iter()
            .map(|e| (e.source.index(), e.target.index()))
            .collect(),
        routes: graph
            .edges
            .iter()
            .enumerate()
            .map(|(i, _)| {
                layout
                    .edges
                    .get(i)
                    .map_or_else(Vec::new, |r| r.points.iter().map(|p| [p.x, p.y]).collect())
            })
            .collect(),
        left_right: options.direction == Direction::LeftRight,
        weights: graph.edges.iter().map(|e| e.weight).collect(),
        thickness: graph.edges.iter().map(|e| e.thickness).collect(),
        labels: graph
            .edges
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let size = e.label_size?;
                let anchor = layout.edges.get(i)?.label_anchor?;
                Some(LabelBox {
                    edge: i,
                    center: [anchor.x, anchor.y],
                    half: [size.width / 2.0, size.height / 2.0],
                })
            })
            .collect(),
        groups: graph.nodes.iter().map(|n| n.group.map(|g| g.0)).collect(),
        parents: graph
            .nodes
            .iter()
            .map(|n| n.parent.map(|p| p.index()))
            .collect(),
        reversed: {
            let mut flags = vec![false; graph.edges.len()];
            for id in &layout.reversed {
                if let Some(slot) = flags.get_mut(id.index()) {
                    *slot = true;
                }
            }
            flags
        },
        rank_first: graph
            .nodes
            .iter()
            .map(|n| n.rank == crate::graph::RankConstraint::First)
            .collect(),
    };
    measure_problem(&problem)
}

fn measure_problem(problem: &Problem) -> LayoutMetrics {
    let mut m = LayoutMetrics {
        node_count: problem.node_count,
        edge_count: problem.edges.len(),
        detour_ratio: 1.0,
        ..LayoutMetrics::default()
    };

    let boxes = &problem.boxes;
    let edges = routed_edges(problem, &mut m);

    m.self_loops = problem.edges.iter().filter(|(a, b)| a == b).count();
    m.parallel_edge_groups = count_parallel_groups(&problem.edges);
    m.backward_edges = flow::count_backward_edges(problem, boxes);
    m.backward_nonloop_edges = flow::count_backward_nonloop_edges(problem, boxes);

    // All segment-pair metrics share one O(S^2) sweep.
    flow::fill_segment_pair_metrics(&edges, &mut m);
    flow::fill_flow_consistency(&edges, &mut m);

    let (hits, clearance) = lanes::node_edge_interaction(&edges, boxes, &problem.parents);
    m.node_edge_hits = hits;
    m.min_edge_node_clearance = clearance;

    let (node_overlaps, min_gap) = lanes::node_pair_metrics(boxes, &problem.parents);
    m.node_overlaps = node_overlaps;
    m.min_node_gap = min_gap;

    let (off_outline, worst) = endpoint_outline_metrics(&edges, boxes);
    m.endpoint_off_outline = off_outline;
    m.max_endpoint_outline_gap = worst;

    let (bends, edges_with_bends) = paths::count_bends(&edges);
    m.bends = bends;
    m.edges_with_bends = edges_with_bends;
    m.excess_bends = paths::excess_bends(&edges, boxes);

    paths::fill_length_metrics(&edges, &mut m);
    paths::fill_path_continuity(&edges, &mut m);
    paths::fill_step_metrics(&edges, &mut m);
    ports::fill_port_metrics(problem, &edges, boxes, &mut m);
    lanes::fill_bounding_box(boxes, &edges, &mut m);
    lanes::fill_perceived_metrics(boxes, &mut m);
    lanes::fill_whitespace_balance(boxes, &mut m);
    lanes::fill_lane_purity(problem, boxes, &mut m);
    m.min_port_gap = min_port_gap(&edges, boxes.len());

    let (pairs, aligned) = lanes::split_alignment(problem, boxes);
    m.split_pairs = pairs;
    m.split_aligned = aligned;
    m.split_alignment = if pairs == 0 {
        1.0
    } else {
        aligned as f64 / pairs as f64
    };
    m.block_intrusion = lanes::block_intrusion(problem, boxes, &edges);

    labels::fill_label_metrics(problem, boxes, &edges, &mut m);
    paths::fill_main_path(problem, boxes, &mut m);
    m
}

/// The edges with a polyline. Counts the rest in `m.unrouted_edges`.
fn routed_edges(problem: &Problem, m: &mut LayoutMetrics) -> Vec<EdgeGeom> {
    let mut out = Vec::with_capacity(problem.edges.len());
    for (i, &(from, to)) in problem.edges.iter().enumerate() {
        match problem.routes.get(i) {
            Some(pts) if pts.len() >= 2 => out.push(EdgeGeom {
                from,
                to,
                pts: pts.clone(),
                id: i,
                weight: problem.weights.get(i).copied().unwrap_or(1.0),
                thickness: problem.thickness.get(i).copied().unwrap_or(2.0),
            }),
            _ => m.unrouted_edges += 1,
        }
    }
    out
}

fn count_parallel_groups(edges: &[(usize, usize)]) -> usize {
    let mut seen: Vec<(usize, usize)> = edges.iter().copied().filter(|(a, b)| a != b).collect();
    seen.sort_unstable();
    let mut groups = 0;
    let mut i = 0;
    while i < seen.len() {
        let mut j = i + 1;
        while j < seen.len() && seen[j] == seen[i] {
            j += 1;
        }
        if j - i > 1 {
            groups += 1;
        }
        i = j;
    }
    groups
}
/// `(endpoints off their node's outline, largest distance)`.
fn endpoint_outline_metrics(edges: &[EdgeGeom], boxes: &[NodeBox]) -> (usize, Option<f64>) {
    let mut off = 0;
    let mut worst: Option<f64> = None;
    for e in edges {
        for (node, p) in e.ends() {
            let Some(nb) = boxes.get(node) else { continue };
            let d = point_outline_distance(p, nb);
            if d > ENDPOINT_OUTLINE_TOL {
                off += 1;
            }
            worst = Some(worst.map_or(d, |w: f64| w.max(d)));
        }
    }
    (off, worst)
}

/// Unsigned distance from `p` to a node's outline, positive both inside and outside.
fn point_outline_distance(p: [f64; 2], n: &NodeBox) -> f64 {
    if n.ellipse {
        // Measured along the centre-to-`p` ray. Exact for a circle and an upper bound otherwise,
        // so it never hides a defect.
        let (dx, dy) = (p[0] - n.cx, p[1] - n.cy);
        if n.hw <= 0.0 || n.hh <= 0.0 {
            return 0.0;
        }
        let r = fp::hypot(dx / n.hw, dy / n.hh);
        if r < EPS {
            return n.hw.min(n.hh);
        }
        fp::hypot(dx * (1.0 - 1.0 / r), dy * (1.0 - 1.0 / r))
    } else {
        let dx = (p[0] - n.cx).abs() - n.hw;
        let dy = (p[1] - n.cy).abs() - n.hh;
        if dx > 0.0 || dy > 0.0 {
            fp::hypot(dx.max(0.0), dy.max(0.0))
        } else {
            (-dx).min(-dy)
        }
    }
}

fn min_port_gap(edges: &[EdgeGeom], node_count: usize) -> Option<f64> {
    let mut ports: Vec<Vec<[f64; 2]>> = vec![Vec::new(); node_count];
    for e in edges {
        for (node, p) in e.ends() {
            if let Some(slot) = ports.get_mut(node) {
                slot.push(p);
            }
        }
    }
    let mut best: Option<f64> = None;
    for node_ports in &ports {
        for (i, a) in node_ports.iter().enumerate() {
            for b in node_ports.iter().skip(i + 1) {
                let d = distance(*a, *b);
                // Two ends at one point are one converged port.
                if d <= SHARED_ENDPOINT_TOL {
                    continue;
                }
                best = Some(best.map_or(d, |x: f64| x.min(d)));
            }
        }
    }
    best
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    fp::hypot(b[0] - a[0], b[1] - a[1])
}

/// Signed distance between two boxes, negative when they overlap. Ignores the ellipse shape, so
/// label boxes can use it too.
fn box_gap(a: &NodeBox, b: &NodeBox) -> f64 {
    let gx = (b.left() - a.right()).max(a.left() - b.right());
    let gy = (b.top() - a.bottom()).max(a.top() - b.bottom());
    match (gx >= 0.0, gy >= 0.0) {
        (true, true) => fp::hypot(gx, gy),
        (true, false) => gx,
        (false, true) => gy,
        (false, false) => gx.max(gy),
    }
}

/// Overlap of two boxes per axis. Positive on both axes means they overlap.
fn box_penetration(a: &NodeBox, b: &NodeBox) -> (f64, f64) {
    (
        a.right().min(b.right()) - a.left().max(b.left()),
        a.bottom().min(b.bottom()) - a.top().max(b.top()),
    )
}

fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

/// [`hit::proper_intersection`] on array points. `None` for parallel, collinear or touching
/// segments.
fn proper_intersection(p: [f64; 2], p2: [f64; 2], q: [f64; 2], q2: [f64; 2]) -> Option<[f64; 2]> {
    hit::proper_intersection(p.into(), p2.into(), q.into(), q2.into()).map(Into::into)
}

fn collinear_overlap_len(p: [f64; 2], p2: [f64; 2], q: [f64; 2], q2: [f64; 2]) -> f64 {
    let r = [p2[0] - p[0], p2[1] - p[1]];
    let len = fp::hypot(r[0], r[1]);
    if len < EPS {
        return 0.0;
    }
    let dir = [r[0] / len, r[1] / len];
    let perp = |x: [f64; 2]| cross(dir, [x[0] - p[0], x[1] - p[1]]).abs();
    if perp(q) > COLLINEAR_TOL || perp(q2) > COLLINEAR_TOL {
        return 0.0;
    }
    let along = |x: [f64; 2]| dir[0] * (x[0] - p[0]) + dir[1] * (x[1] - p[1]);
    let (t1, t2) = (along(q), along(q2));
    let lo = t1.min(t2).max(0.0);
    let hi = t1.max(t2).min(len);
    (hi - lo).max(0.0)
}

/// Whether the segment enters the node, inset by [`NODE_HIT_TOL`].
fn segment_hits_node(a: [f64; 2], b: [f64; 2], n: &NodeBox) -> bool {
    let hw = n.hw - NODE_HIT_TOL;
    let hh = n.hh - NODE_HIT_TOL;
    if hw <= 0.0 || hh <= 0.0 {
        return false;
    }
    if n.ellipse {
        segment_hits_ellipse(a, b, [n.cx, n.cy], hw, hh)
    } else {
        segment_clip_box(a, b, n.cx - hw, n.cx + hw, n.cy - hh, n.cy + hh).is_some()
    }
}

/// Liang-Barsky clip of `a->b` against the rectangle. Returns the parameter range inside it, or
/// `None` when the segment misses or only grazes it.
fn segment_clip_box(
    a: [f64; 2],
    b: [f64; 2],
    left: f64,
    right: f64,
    top: f64,
    bottom: f64,
) -> Option<(f64, f64)> {
    let d = [b[0] - a[0], b[1] - a[1]];
    let mut t0 = 0.0f64;
    let mut t1 = 1.0f64;
    let limits = [
        (-d[0], a[0] - left),
        (d[0], right - a[0]),
        (-d[1], a[1] - top),
        (d[1], bottom - a[1]),
    ];
    for (p, q) in limits {
        if p.abs() < DEGENERATE {
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let t = q / p;
        if p < 0.0 {
            t0 = t0.max(t);
        } else {
            t1 = t1.min(t);
        }
        if t0 >= t1 {
            return None;
        }
    }
    Some((t0, t1))
}

/// Whether `a->b` enters the ellipse with centre `c` and semi-axes `hw`, `hh`.
fn segment_hits_ellipse(a: [f64; 2], b: [f64; 2], c: [f64; 2], hw: f64, hh: f64) -> bool {
    // Scale to the unit circle and test the segment's closest point to the origin.
    let pa = [(a[0] - c[0]) / hw, (a[1] - c[1]) / hh];
    let pb = [(b[0] - c[0]) / hw, (b[1] - c[1]) / hh];
    let d = [pb[0] - pa[0], pb[1] - pa[1]];
    let dd = d[0] * d[0] + d[1] * d[1];
    let t = if dd < DEGENERATE_SQ {
        0.0
    } else {
        (-(pa[0] * d[0] + pa[1] * d[1]) / dd).clamp(0.0, 1.0)
    };
    let closest = [pa[0] + d[0] * t, pa[1] + d[1] * t];
    closest[0] * closest[0] + closest[1] * closest[1] < 1.0
}

/// Distance from a segment to a node's bounding box, 0 when they touch.
fn segment_box_distance(a: [f64; 2], b: [f64; 2], n: &NodeBox) -> f64 {
    let (l, r, t, bo) = (n.left(), n.right(), n.top(), n.bottom());
    if segment_clip_box(a, b, l, r, t, bo).is_some() {
        return 0.0;
    }
    let corners = [[l, t], [r, t], [r, bo], [l, bo]];
    let mut best = f64::INFINITY;
    for i in 0..4 {
        let c1 = corners[i];
        let c2 = corners[(i + 1) % 4];
        best = best.min(segment_segment_distance(a, b, c1, c2));
    }
    best
}

fn segment_segment_distance(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> f64 {
    if proper_intersection(a, b, c, d).is_some() {
        return 0.0;
    }
    let (a, b, c, d) = (a.into(), b.into(), c.into(), d.into());
    hit::point_seg_dist(a, c, d)
        .min(hit::point_seg_dist(b, c, d))
        .min(hit::point_seg_dist(c, a, b))
        .min(hit::point_seg_dist(d, a, b))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// A node of a hand-written test problem.
    #[derive(Debug, Clone)]
    #[allow(dead_code)]
    pub(super) struct GraphNode {
        pub width: f64,
        pub height: f64,
        pub ellipse: bool,
        pub pin: Option<String>,
        pub category: Option<u32>,
        pub seed: Option<[f64; 2]>,
        pub pinned: bool,
        pub clear_after: f64,
    }

    /// A hand-written test problem.
    #[derive(Debug, Clone)]
    #[allow(dead_code)]
    pub(super) struct GraphSpec {
        pub nodes: Vec<GraphNode>,
        pub edges: Vec<(usize, usize)>,
        pub weights: Vec<f64>,
        pub direction: Option<String>,
        pub flow_edges: bool,
        pub flow_diagonal: bool,
        pub edge_label_sizes: Vec<[f64; 2]>,
        pub thickness: Vec<f64>,
        pub tree: bool,
        pub compact: bool,
    }

    /// A hand-written drawing of a [`GraphSpec`].
    #[derive(Debug, Clone)]
    pub(super) struct GraphLayout {
        pub centers: Vec<[f64; 2]>,
        pub routes: Vec<Vec<[f64; 2]>>,
        pub label_anchors: Vec<Option<[f64; 2]>>,
    }

    /// Measures a hand-written drawing through the same `Problem` view [`measure`] builds.
    pub(super) fn compute(problem: &GraphSpec, layout: &GraphLayout) -> LayoutMetrics {
        let neutral = Problem {
            boxes: problem
                .nodes
                .iter()
                .zip(layout.centers.iter())
                .map(|(n, c)| NodeBox {
                    cx: c[0],
                    cy: c[1],
                    hw: n.width / 2.0,
                    hh: n.height / 2.0,
                    ellipse: n.ellipse,
                })
                .collect(),
            node_count: problem.nodes.len(),
            edges: problem.edges.clone(),
            parents: vec![None; problem.nodes.len()],
            routes: problem
                .edges
                .iter()
                .enumerate()
                .map(|(i, _)| layout.routes.get(i).cloned().unwrap_or_default())
                .collect(),
            left_right: problem
                .direction
                .as_deref()
                .is_some_and(|d| d.eq_ignore_ascii_case("LR")),
            weights: sized_or(&problem.weights, problem.edges.len(), 1.0),
            thickness: sized_or(&problem.thickness, problem.edges.len(), 2.0),
            labels: problem
                .edges
                .iter()
                .enumerate()
                .filter_map(|(i, _)| {
                    let &[w, h] = problem.edge_label_sizes.get(i)?;
                    let &[x, y] = layout.label_anchors.get(i)?.as_ref()?;
                    (w > 0.0 && h > 0.0).then_some(LabelBox {
                        edge: i,
                        center: [x, y],
                        half: [w / 2.0, h / 2.0],
                    })
                })
                .collect(),
            groups: problem.nodes.iter().map(|n| n.category).collect(),
            // A `GraphLayout` carries no reversed edges.
            reversed: vec![false; problem.edges.len()],
            rank_first: problem
                .nodes
                .iter()
                .map(|n| {
                    n.pin
                        .as_deref()
                        .is_some_and(|p| p.eq_ignore_ascii_case("first"))
                })
                .collect(),
        };
        measure_problem(&neutral)
    }

    fn sized_or(values: &[f64], len: usize, fallback: f64) -> Vec<f64> {
        if values.len() == len {
            values.to_vec()
        } else {
            vec![fallback; len]
        }
    }

    pub(super) fn node(width: f64, height: f64, ellipse: bool) -> GraphNode {
        GraphNode {
            width,
            height,
            ellipse,
            pin: None,
            category: None,
            seed: None,
            pinned: false,
            clear_after: 0.0,
        }
    }

    pub(super) fn boxes(count: usize) -> Vec<GraphNode> {
        (0..count).map(|_| node(100.0, 50.0, false)).collect()
    }

    pub(super) fn spec(
        nodes: Vec<GraphNode>,
        edges: Vec<(usize, usize)>,
        direction: &str,
    ) -> GraphSpec {
        GraphSpec {
            nodes,
            edges,
            weights: vec![],
            direction: Some(direction.to_string()),
            flow_edges: false,
            flow_diagonal: false,
            edge_label_sizes: vec![],
            thickness: vec![],
            tree: false,
            compact: false,
        }
    }

    pub(super) fn layout(centers: Vec<[f64; 2]>, routes: Vec<Vec<[f64; 2]>>) -> GraphLayout {
        GraphLayout {
            centers,
            routes,
            label_anchors: vec![],
        }
    }

    /// Endpoints off the outline count from either side and for either shape.
    #[test]
    fn endpoints_off_the_outline_are_counted() {
        // The borders are at y=25 and y=375.
        let s = spec(boxes(2), vec![(0, 1)], "TB");
        let on = compute(
            &s,
            &layout(
                vec![[0.0, 0.0], [0.0, 400.0]],
                vec![vec![[0.0, 25.0], [0.0, 375.0]]],
            ),
        );
        assert_eq!(on.endpoint_off_outline, 0);
        assert_eq!(on.max_endpoint_outline_gap, Some(0.0));
        // 4px outside one border and 10px inside the other.
        let off = compute(
            &s,
            &layout(
                vec![[0.0, 0.0], [0.0, 400.0]],
                vec![vec![[0.0, 29.0], [0.0, 385.0]]],
            ),
        );
        assert_eq!(off.endpoint_off_outline, 2);
        assert_eq!(off.max_endpoint_outline_gap, Some(10.0));
        // On a circle, an offset point on the box border is off the outline.
        let mut ell = spec(
            vec![node(50.0, 50.0, true), node(50.0, 50.0, true)],
            vec![(0, 1)],
            "TB",
        );
        ell.direction = Some("TB".to_string());
        let m = compute(
            &ell,
            &layout(
                vec![[0.0, 0.0], [0.0, 400.0]],
                vec![vec![[20.0, 25.0], [0.0, 375.0]]],
            ),
        );
        assert_eq!(m.endpoint_off_outline, 1);
    }

    #[test]
    fn parallel_edge_groups_are_counted_once() {
        let s = spec(boxes(3), vec![(0, 1), (0, 1), (0, 1), (1, 2), (2, 2)], "TB");
        let l = layout(vec![[0.0, 0.0], [0.0, 200.0], [0.0, 400.0]], vec![]);
        let m = compute(&s, &l);
        assert_eq!(m.parallel_edge_groups, 1);
        assert_eq!(m.self_loops, 1);
        assert_eq!(m.edge_count, 5);
    }

    #[test]
    fn min_port_gap_measures_attachments() {
        let s = spec(boxes(3), vec![(0, 1), (0, 2)], "TB");
        let l = layout(
            vec![[0.0, 0.0], [-200.0, 200.0], [200.0, 200.0]],
            vec![
                vec![[-15.0, 25.0], [-200.0, 175.0]],
                vec![[15.0, 25.0], [200.0, 175.0]],
            ],
        );
        assert!((compute(&s, &l).min_port_gap.unwrap() - 30.0).abs() < 1e-6);

        let s1 = spec(boxes(2), vec![(0, 1)], "TB");
        let l1 = layout(
            vec![[0.0, 0.0], [0.0, 200.0]],
            vec![vec![[0.0, 25.0], [0.0, 175.0]]],
        );
        assert_eq!(compute(&s1, &l1).min_port_gap, None);
    }

    /// Two edges leaving one point along one trunk are a converged port, not an overlap, a
    /// parallel run or a zero port gap.
    #[test]
    fn converged_ends_are_not_overlaps() {
        let s = spec(boxes(3), vec![(0, 1), (0, 2)], "LR");
        let l = layout(
            vec![[0.0, 0.0], [400.0, -100.0], [400.0, 100.0]],
            vec![
                vec![[50.0, 0.0], [100.0, 0.0], [100.0, -100.0], [350.0, -100.0]],
                vec![[50.0, 0.0], [150.0, 0.0], [150.0, 100.0], [350.0, 100.0]],
            ],
        );
        let m = compute(&s, &l);
        assert_eq!(m.overlaps, 0, "{m:?}");
        assert_eq!(m.parallel_run_length, 0.0, "{m:?}");
        assert_eq!(m.converged_ends, 2);
        assert_eq!(m.min_port_gap, None);
        assert_eq!(m.off_centre_ends, 0);

        // Leaving from two different points, the shared run is an overlap.
        let l = layout(
            vec![[0.0, 0.0], [400.0, -100.0], [400.0, 100.0]],
            vec![
                vec![[50.0, 0.0], [150.0, 0.0], [150.0, -100.0], [350.0, -100.0]],
                vec![
                    [50.0, 20.0],
                    [100.0, 20.0],
                    [100.0, 0.0],
                    [300.0, 0.0],
                    [300.0, 100.0],
                    [350.0, 100.0],
                ],
            ],
        );
        let m = compute(&s, &l);
        assert_eq!(m.overlaps, 1, "{m:?}");
        assert_eq!(m.converged_ends, 0);
        // 20px off the middle of a 25px half-border.
        assert_eq!(m.off_centre_ends, 1);
    }

    #[test]
    fn score_grows_with_defects() {
        let w = ScoreWeights::default();
        let clean = LayoutMetrics {
            node_count: 10,
            ..LayoutMetrics::default()
        };
        let one_crossing = LayoutMetrics {
            crossings: 1,
            ..clean.clone()
        };
        let one_hit = LayoutMetrics {
            node_edge_hits: 1,
            ..clean.clone()
        };
        assert!((clean.score(&w) - 0.0).abs() < 1e-9);
        assert!(one_crossing.score(&w) > clean.score(&w));
        assert!(one_hit.score(&w) > one_crossing.score(&w));

        let big = LayoutMetrics {
            area: 10_000.0 * 10.0,
            total_edge_length: 100.0 * 10.0,
            ..clean
        };
        // One normalised unit each of area and edge length.
        assert!((big.score(&w) - (w.area + w.edge_length)).abs() < 1e-9);
    }

    /// Font size, aspect and split terms each move the score, and an empty drawing costs nothing.
    #[test]
    fn score_charges_for_perceived_quality() {
        let w = ScoreWeights::default();
        let base = LayoutMetrics {
            node_count: 10,
            fit_font_px: 12.5,
            aspect_penalty: 1.0,
            split_alignment: 1.0,
            ..LayoutMetrics::default()
        };
        assert!((base.score(&w) - 0.0).abs() < 1e-9);

        let unreadable = LayoutMetrics {
            fit_font_px: 4.0,
            ..base.clone()
        };
        assert!((unreadable.score(&w) - 16.0 * w.fit_font).abs() < 1e-9);
        // Half the shortfall costs a quarter as much.
        let squinting = LayoutMetrics {
            fit_font_px: 6.0,
            ..base.clone()
        };
        assert!((squinting.score(&w) - 4.0 * w.fit_font).abs() < 1e-9);

        let strip = LayoutMetrics {
            aspect_penalty: 9.0,
            ..base.clone()
        };
        assert!((strip.score(&w) - 6.0 * w.aspect).abs() < 1e-9);
        // Up to 3:1 is free.
        let oblong = LayoutMetrics {
            aspect_penalty: 3.0,
            ..base.clone()
        };
        assert!((oblong.score(&w) - 0.0).abs() < 1e-9);

        let misaligned = LayoutMetrics {
            split_pairs: 7,
            split_aligned: 2,
            ..base.clone()
        };
        assert!((misaligned.score(&w) - 5.0 * w.split_misaligned).abs() < 1e-9);

        let empty = LayoutMetrics {
            node_count: 0,
            ..LayoutMetrics::default()
        };
        assert!((empty.score(&w) - 0.0).abs() < 1e-9);

        let r = ScoreWeights::readability();
        assert!(r.fit_font > w.fit_font && r.aspect > w.aspect);
        assert!(r.split_misaligned > w.split_misaligned);
        assert!(r.main_path > w.main_path && r.flow > w.flow);
    }

    /// The label, main-path and flow terms each move the score.
    #[test]
    fn score_charges_for_the_literature_terms() {
        let w = ScoreWeights::balanced();
        let base = LayoutMetrics {
            node_count: 4,
            fit_font_px: READABLE_FONT_PX,
            aspect_penalty: 1.0,
            split_alignment: 1.0,
            main_path_aligned_fraction: 1.0,
            flow_consistency: 1.0,
            ..LayoutMetrics::default()
        };
        assert!(base.score(&w).abs() < 1e-9);

        let labels = LayoutMetrics {
            label_overlaps: 3,
            ..base.clone()
        };
        assert!((labels.score(&w) - 3.0 * w.label_overlap).abs() < 1e-9);
        // The same price as an edge through a box.
        let hits = LayoutMetrics {
            node_edge_hits: 3,
            ..base.clone()
        };
        assert!((labels.score(&w) - hits.score(&w)).abs() < 1e-9);

        // Six of ten main-path nodes off the median line.
        let crooked = LayoutMetrics {
            main_path_len: 10,
            main_path_aligned_fraction: 0.4,
            ..base.clone()
        };
        assert!((crooked.score(&w) - 6.0 * w.main_path).abs() < 1e-9);

        // A quarter of eight routed edges' length heads against the majority.
        let wandering = LayoutMetrics {
            edge_count: 8,
            flow_consistency: 0.75,
            ..base.clone()
        };
        assert!((wandering.score(&w) - 2.0 * w.flow).abs() < 1e-9);
        let half_unrouted = LayoutMetrics {
            unrouted_edges: 4,
            ..wandering.clone()
        };
        assert!((half_unrouted.score(&w) - 1.0 * w.flow).abs() < 1e-9);

        let no_path = LayoutMetrics {
            main_path_len: 0,
            main_path_aligned_fraction: 0.0,
            ..base.clone()
        };
        assert!(no_path.score(&w).abs() < 1e-9);
    }

    /// Older baselines load, and missing weights get their real defaults.
    #[test]
    fn old_baselines_and_weights_still_deserialise() {
        let mut old = serde_json::to_value(LayoutMetrics {
            node_count: 3,
            crossings: 1,
            ..LayoutMetrics::default()
        })
        .unwrap();
        for gone in [
            "fit_font_px",
            "density",
            "aspect_penalty",
            "split_pairs",
            "split_aligned",
            "split_alignment",
            "label_count",
            "label_overlaps",
            "label_label_overlaps",
            "label_node_overlaps",
            "label_edge_hits",
            "min_label_clearance",
            "weighted_crossings",
            "thickness_weighted_crossings",
            "main_path_len",
            "main_path_aligned_fraction",
            "main_path_deviation",
            "flow_consistency",
            "flow_consistency_weighted",
            "crossing_angle_min",
            "crossing_angle_mean",
            "crossings_below_30deg",
            "parallel_run_length",
            "block_intrusion",
            "lane_purity",
            "lane_intruders",
            "whitespace_balance",
            "path_turn_mean",
            "path_turn_max",
            "short_steps",
            "route_reversals",
            "staircase_steps",
            "perpendicular_ends",
            "backward_nonloop_edges",
            "off_centre_ends",
            "inward_ends",
            "converged_ends",
        ] {
            assert!(old.as_object_mut().unwrap().remove(gone).is_some());
        }
        let m: LayoutMetrics = serde_json::from_value(old).unwrap();
        assert_eq!(m.crossings, 1);
        assert_eq!(m.split_pairs, 0);
        assert_eq!(m.fit_font_px, 0.0);
        assert_eq!(m.label_overlaps, 0);
        assert_eq!(m.weighted_crossings, 0.0);
        assert_eq!(m.min_label_clearance, None);
        assert_eq!(m.crossing_angle_min, None);

        let old_weights = r#"{"crossing":10.0,"overlap":4.0,"node_edge_hit":25.0,
            "node_overlap":50.0,"bend":1.0,"backward_edge":0.5,"area":12.0,"edge_length":12.0}"#;
        let w: ScoreWeights = serde_json::from_str(old_weights).unwrap();
        assert_eq!(w, ScoreWeights::balanced());
    }

    #[test]
    fn short_routes_are_tolerated() {
        let s = spec(boxes(2), vec![(0, 1), (1, 0)], "TB");
        let l = layout(vec![[0.0, 0.0], [0.0, 200.0]], vec![vec![[0.0, 25.0]]]);
        let m = compute(&s, &l);
        assert_eq!(m.unrouted_edges, 2);
        assert_eq!(m.total_edge_length, 0.0);
    }
}

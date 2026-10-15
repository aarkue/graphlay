//! The mutable state every layered phase works on.
//!
//! [`LayeredState`] keeps per-node data in one [`LNode`] per node plus the arc list, so a phase
//! reads everything off one `&mut`.
//!
//! Everything here is in flow space, with `along` the layer axis and `cross` the order axis.
//! [`Axis`] maps that to output x/y, once on the way in and once on the way out.

use std::collections::BTreeMap;

use crate::geometry::shape::NodeShape;
use crate::geometry::Boxes;
use crate::graph::PortSides;
use crate::options::{Direction, LayoutOptions};

use super::prepare::{EdgePortHints, LayeredInput};
use super::selfloop::{LoopAnchors, LoopRoutes, LoopSlot};

/// Engine-side [`crate::RankConstraint`], with `Free` as `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerConstraint {
    /// The source rank.
    First,
    /// The sink rank.
    Last,
    /// A layer relative to the other fixed nodes. Equal values share a layer, and a larger value
    /// is at least that many layers further along.
    Fixed(i32),
}

impl LayerConstraint {
    /// A first- or last-layer terminal.
    pub fn is_terminal(self) -> bool {
        matches!(self, LayerConstraint::First | LayerConstraint::Last)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LKind {
    /// A caller node, with the caller's node index.
    Real,
    /// A routing waypoint of a multi-layer edge.
    Dummy,
}

/// One internal node, real or routing dummy. `Copy` so tight loops can read it by value.
#[derive(Clone, Copy, Debug)]
pub struct LNode {
    pub kind: LKind,
    /// Layer index, ascending along the layer axis.
    pub rank: i32,
    /// Centre on the layer axis.
    pub along: f64,
    /// Centre on the order axis.
    pub cross: f64,
    /// Extent along the layer axis, 0 for a dummy that reserves no space.
    pub w_along: f64,
    /// Extent along the order axis, 0 for a dummy that reserves no space.
    pub w_cross: f64,
    /// Drawn as an ellipse, so the router projects ports onto the ellipse outline.
    pub ellipse: bool,
    /// Clearance beyond the positive-order border, for caller decorations and self-loops.
    pub clear_after: f64,
    /// Clearance beyond the negative-order border.
    pub clear_before: f64,
    /// Lane group id. `None` for a waypoint or a node shared between groups.
    pub group: Option<u32>,
    /// Sort key inside its group's band ([`crate::Node::lane_key`]).
    pub lane_key: Option<i64>,
    /// Layer-axis clearance before the node ([`crate::Margins::along_neg`]), `0` for a waypoint.
    pub along_before: f64,
    /// Layer-axis clearance after the node ([`crate::Margins::along_pos`]), `0` for a waypoint.
    pub along_after: f64,
    pub constraint: Option<LayerConstraint>,
    /// Warm-start centre in flow space.
    pub seed: Option<(f64, f64)>,
    /// Hold `seed` exactly and make other nodes yield, e.g., for the node just dragged.
    pub pinned: bool,
    /// Borders an edge may attach to. `None` defers to [`crate::Routing::port_sides`].
    pub port_sides: Option<PortSides>,
}

impl LNode {
    pub fn is_dummy(&self) -> bool {
        self.kind == LKind::Dummy
    }

    /// [`LNode::rank`] as a layer index. Ranks start at 0 after layering, so a negative one is a
    /// bug.
    pub fn layer(&self) -> usize {
        debug_assert!(
            self.rank >= 0,
            "negative rank {} read as a layer",
            self.rank
        );
        self.rank as usize
    }

    /// [`LNode::layer`] with a negative rank read as layer 0.
    pub fn layer_clamped(&self) -> usize {
        self.rank.max(0) as usize
    }
}

/// One internal arc. A long edge is a chain of arcs with the same `edge`.
#[derive(Clone, Copy, Debug)]
pub struct LArc {
    pub from: usize,
    pub to: usize,
    pub edge: usize,
    /// Flipped by the cycle breaker, so the polyline is turned back on output.
    pub reversed: bool,
}

/// How flow space maps onto output space. Top-to-bottom is left-to-right with the axes swapped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis {
    /// Layer axis is x, for left-to-right.
    AlongX,
    /// Layer axis is y, for top-to-bottom.
    AlongY,
}

impl Axis {
    pub fn of(direction: Direction) -> Self {
        match direction {
            Direction::LeftRight => Axis::AlongX,
            Direction::TopBottom => Axis::AlongY,
        }
    }

    pub fn extents(self, width: f64, height: f64) -> (f64, f64) {
        match self {
            Axis::AlongX => (width, height),
            Axis::AlongY => (height, width),
        }
    }

    pub fn split(self, p: (f64, f64)) -> (f64, f64) {
        match self {
            Axis::AlongX => (p.0, p.1),
            Axis::AlongY => (p.1, p.0),
        }
    }

    pub fn to_output(self, p: (f64, f64)) -> (f64, f64) {
        match self {
            Axis::AlongX => p,
            Axis::AlongY => (p.1, p.0),
        }
    }

    /// In `AlongY` the flow `Right` (`+along`) is screen `Bottom` and flow `Bottom` (`+cross`) is
    /// screen `Right`.
    pub fn to_output_side(self, side: crate::geometry::Side) -> crate::geometry::Side {
        use crate::geometry::Side;
        match self {
            Axis::AlongX => side,
            Axis::AlongY => match side {
                Side::Right => Side::Bottom,
                Side::Left => Side::Top,
                Side::Bottom => Side::Right,
                Side::Top => Side::Left,
            },
        }
    }

    /// The inverse of [`Axis::to_output_side`], which is its own inverse.
    pub fn to_flow_side(self, side: crate::geometry::Side) -> crate::geometry::Side {
        self.to_output_side(side)
    }
}

/// A strict swimlane in flow space: its group (`None` for ungrouped nodes) and its
/// `(along_lo, along_hi, cross_lo, cross_hi)` extent.
pub type FlowLane = (Option<u32>, (f64, f64, f64, f64));

/// What a run starts from, decided once so every phase agrees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    /// No node carries a seed.
    Fresh,
    /// Some node carries a seed ([`crate::Node::position`]). Cross-axis placement follows the
    /// seeds, while layering and ordering still run fresh.
    Seeded,
    /// [`crate::options::SeedMode::Structure`] with a usable seeded layering. Layers and order
    /// come from the seeds and no phase rearranges them.
    KeepStructure,
}

/// The whole mutable state of one layered layout run.
pub struct LayeredState {
    /// Real nodes at `0..n_real`, then routing dummies in insertion order.
    pub nodes: Vec<LNode>,
    pub arcs: Vec<LArc>,
    /// `layers[rank][order]` is a node index.
    pub layers: Vec<Vec<usize>>,
    /// `order_of[i]` is `i`'s index within its layer. Stale after a reorder until
    /// [`LayeredState::reindex`] runs.
    pub order_of: Vec<usize>,
    /// Node chain per caller edge, layer-ascending: source, dummies, target.
    pub chains: BTreeMap<usize, Vec<usize>>,
    /// Layout weight per caller edge, 1.0 when the caller gave none.
    pub weight: Vec<f64>,
    /// Stroke width per caller edge, 2.0 when the caller gave none.
    pub thickness: Vec<f64>,
    /// Routed polyline per caller edge, in flow space.
    pub routes: Vec<Vec<(f64, f64)>>,
    /// Per edge, the stroke width of each lane [`super::bundle`] merged into it, in corridor
    /// order. Empty when the run has no bundle. Routing gives every lane its own port.
    pub lanes: Vec<Vec<f64>>,
    /// One polyline per lane of each bundled edge, in flow space. Empty for an unbundled edge.
    pub lane_routes: Vec<Vec<Vec<(f64, f64)>>>,
    /// `(width, height)` of each edge's label in output space, `(0, 0)` for none. Empty when no
    /// edge has a label.
    pub label_sizes: Vec<(f64, f64)>,
    /// Space after each layer, the layer gap widened where a label must fit.
    pub gap_after: Vec<f64>,
    /// Centre of each edge's reserved label box, in flow space.
    pub label_anchors: Vec<Option<(f64, f64)>>,
    /// Lane band per internal node. Empty when the order is not banded, e.g., with fewer than
    /// two groups.
    pub band_of: Vec<usize>,
    pub band_count: usize,
    /// Group per band, `None` for the band of ungrouped nodes.
    pub band_groups: Vec<Option<u32>>,
    /// Strict swimlanes, measured after the last move.
    pub swimlanes: Vec<FlowLane>,
    /// Per arc, whether it is ranked against its arrow. Empty when none is.
    pub ranked_backwards: Vec<bool>,
    /// Port hints `[at source, at target]` per edge in the caller's orientation, which may differ
    /// from the arc's. Empty when no edge has one.
    pub port_hints: Vec<EdgePortHints>,
    /// Raised to [`RunMode::KeepStructure`] by the cycle phase when the seeded layers are usable.
    pub mode: RunMode,
    /// Branch points of merged edges ([`crate::Routing::merge_edges`]), in flow space.
    pub junctions: Vec<(f64, f64)>,
    pub n_real: usize,
    pub axis: Axis,
    /// One slot per node with self-loops. Loops never enter `arcs`, so structural phases skip
    /// them.
    pub self_loops: Vec<LoopSlot>,
    /// Loop polyline per caller edge index, in flow space.
    pub loop_routes: LoopRoutes,
    /// Loop label anchor per caller edge index, in flow space.
    pub loop_label_anchors: LoopAnchors,
    /// The last cross-axis placement the network simplex found, per node, as the next solve's
    /// starting point.
    pub warm_cross: Option<Vec<i64>>,
}

impl LayeredState {
    /// One [`LNode`] per caller node in flow space and one [`LArc`] per caller edge.
    pub(crate) fn new(input: &LayeredInput) -> Self {
        let axis = Axis::of(input.options.direction);
        let n_real = input.nodes.len();
        let n_edges = input.edges.len();
        let nodes: Vec<LNode> = input
            .nodes
            .iter()
            .map(|nd| {
                let (w_along, w_cross) = axis.extents(nd.width, nd.height);
                LNode {
                    kind: LKind::Real,
                    rank: 0,
                    along: 0.0,
                    cross: 0.0,
                    w_along,
                    w_cross,
                    ellipse: nd.shape == NodeShape::Ellipse,
                    clear_after: nd.clear_after.max(0.0),
                    clear_before: nd.clear_before.max(0.0),
                    group: nd.category,
                    lane_key: nd.lane_key,
                    along_before: nd.along_before,
                    along_after: nd.along_after,
                    constraint: nd.constraint,
                    seed: None,
                    pinned: false,
                    port_sides: nd.port_sides,
                }
            })
            .collect();
        let mut nodes = nodes;
        for (i, node) in nodes.iter_mut().enumerate() {
            node.seed = input
                .seed
                .get(i)
                .copied()
                .flatten()
                .map(|(x, y)| axis.split((x, y)));
            node.pinned = input.pinned.get(i).copied().unwrap_or(false);
        }
        let mode = if nodes.iter().any(|nd| nd.seed.is_some()) {
            RunMode::Seeded
        } else {
            RunMode::Fresh
        };
        let arcs: Vec<LArc> = input
            .edges
            .iter()
            .enumerate()
            .map(|(edge, &(from, to))| LArc {
                from,
                to,
                edge,
                reversed: false,
            })
            .collect();
        // Uniform 1.0 makes every weighted step reduce exactly to its unweighted form.
        let weight = if input.weights.len() == n_edges {
            input.weights.clone()
        } else {
            vec![1.0; n_edges]
        };
        let thickness = if input.thickness.len() == n_edges {
            input.thickness.clone()
        } else {
            vec![2.0; n_edges]
        };
        LayeredState {
            nodes,
            arcs,
            layers: vec![],
            order_of: vec![0; n_real],
            chains: BTreeMap::new(),
            weight,
            thickness,
            routes: vec![vec![]; n_edges],
            lanes: vec![],
            lane_routes: vec![vec![]; n_edges],
            label_sizes: input.edge_label_sizes.clone(),
            gap_after: vec![],
            label_anchors: vec![None; n_edges],
            band_of: vec![],
            band_count: 0,
            band_groups: vec![],
            swimlanes: vec![],
            ranked_backwards: input.ranked_backwards.clone(),
            port_hints: Vec::new(),
            mode,
            junctions: vec![],
            n_real,
            axis,
            self_loops: input.self_loops.clone(),
            loop_routes: LoopRoutes::new(),
            loop_label_anchors: LoopAnchors::new(),
            warm_cross: None,
        }
    }

    /// Whether any caller node carries a warm-start centre.
    pub fn has_seeds(&self) -> bool {
        self.mode != RunMode::Fresh
    }

    /// Whether the layers and order come from the seeds ([`RunMode::KeepStructure`]).
    pub fn keeps_structure(&self) -> bool {
        self.mode == RunMode::KeepStructure
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn add_dummy(&mut self, rank: i32) -> usize {
        let idx = self.nodes.len();
        self.nodes.push(LNode {
            kind: LKind::Dummy,
            rank,
            along: 0.0,
            cross: 0.0,
            w_along: 0.0,
            w_cross: 0.0,
            ellipse: false,
            clear_after: 0.0,
            clear_before: 0.0,
            group: None,
            lane_key: None,
            along_before: 0.0,
            along_after: 0.0,
            constraint: None,
            seed: None,
            pinned: false,
            port_sides: None,
        });
        idx
    }

    /// `(pred, succ)` per node in arc order. Self-loops cross no layer gap and are left out.
    pub fn adjacency(&self) -> (Vec<Vec<usize>>, Vec<Vec<usize>>) {
        let n = self.len();
        let mut pred: Vec<Vec<usize>> = vec![vec![]; n];
        let mut succ: Vec<Vec<usize>> = vec![vec![]; n];
        for a in &self.arcs {
            if a.from != a.to {
                pred[a.to].push(a.from);
                succ[a.from].push(a.to);
            }
        }
        (pred, succ)
    }

    /// Rebuilds [`LayeredState::order_of`] from [`LayeredState::layers`].
    pub fn reindex(&mut self) {
        self.order_of.resize(self.nodes.len(), 0);
        for layer in &self.layers {
            for (order, &node) in layer.iter().enumerate() {
                self.order_of[node] = order;
            }
        }
    }

    /// Fills [`LayeredState::layers`] from the ranks in node-index order, then reindexes.
    pub fn rebuild_layers(&mut self) {
        let n_layers = self
            .nodes
            .iter()
            .map(LNode::layer_clamped)
            .max()
            .unwrap_or(0)
            + 1;
        self.layers = vec![vec![]; n_layers];
        for (i, node) in self.nodes.iter().enumerate() {
            self.layers[node.layer()].push(i);
        }
        self.reindex();
    }

    /// Minimum centre distance between order neighbours `a` and `b`, with `a` first. Either
    /// node's clearance into the gap widens it when larger than the plain gap.
    pub fn sep(&self, a: usize, b: usize, o: &LayoutOptions) -> f64 {
        let (na, nb) = (&self.nodes[a], &self.nodes[b]);
        let gap = self.gap(a, b, o);
        na.w_cross / 2.0 + gap.max(na.clear_after).max(nb.clear_before) + nb.w_cross / 2.0
    }

    /// The plain order-axis gap between neighbours `a` and `b`, before either's clearance. A
    /// routing waypoint is a line, so it keeps the tighter [`crate::Spacing::dummy_gap`].
    pub fn gap(&self, a: usize, b: usize, o: &LayoutOptions) -> f64 {
        if self.nodes[a].is_dummy() || self.nodes[b].is_dummy() {
            o.spacing.dummy_gap
        } else {
            o.spacing.node_gap
        }
    }

    pub(crate) fn boxes(&self) -> Boxes {
        Boxes {
            cx: self.nodes.iter().map(|n| n.along).collect(),
            cy: self.nodes.iter().map(|n| n.cross).collect(),
            layer_w: self.nodes.iter().map(|n| n.w_along).collect(),
            order_w: self.nodes.iter().map(|n| n.w_cross).collect(),
            ellipse: self.nodes.iter().map(|n| n.ellipse).collect(),
            n_real: self.n_real,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::layered::prepare::LayeredInputNode;

    fn input(direction: Direction) -> LayeredInput {
        LayeredInput {
            nodes: vec![
                LayeredInputNode {
                    width: 100.0,
                    height: 40.0,
                    shape: NodeShape::Box,
                    constraint: None,
                    category: None,
                    clear_after: 0.0,
                    clear_before: 0.0,
                    port_sides: None,
                    lane_key: None,
                    along_before: 0.0,
                    along_after: 0.0,
                },
                LayeredInputNode {
                    width: 60.0,
                    height: 20.0,
                    shape: NodeShape::Ellipse,
                    constraint: None,
                    category: Some(7),
                    clear_after: 90.0,
                    clear_before: 0.0,
                    port_sides: Some(PortSides::FacingPreferred),
                    lane_key: None,
                    along_before: 0.0,
                    along_after: 0.0,
                },
            ],
            edges: vec![(0, 1)],
            weights: vec![],
            thickness: vec![],
            edge_label_sizes: vec![],
            seed: vec![None, Some((10.0, 20.0))],
            pinned: vec![false, true],
            self_loops: vec![],
            ranked_backwards: Vec::new(),
            options: LayoutOptions {
                direction,
                ..LayoutOptions::layered_default()
            },
        }
    }

    #[test]
    fn axis_transposes_extents_and_points() {
        assert_eq!(Axis::AlongX.extents(100.0, 40.0), (100.0, 40.0));
        assert_eq!(Axis::AlongY.extents(100.0, 40.0), (40.0, 100.0));
        for axis in [Axis::AlongX, Axis::AlongY] {
            assert_eq!(axis.to_output(axis.split((3.0, 5.0))), (3.0, 5.0));
        }
        assert_eq!(Axis::AlongY.split((3.0, 5.0)), (5.0, 3.0));
        use crate::geometry::Side;
        assert_eq!(Axis::AlongX.to_output_side(Side::Bottom), Side::Bottom);
        assert_eq!(Axis::AlongY.to_output_side(Side::Bottom), Side::Right);
        assert_eq!(Axis::AlongY.to_output_side(Side::Right), Side::Bottom);
        for axis in [Axis::AlongX, Axis::AlongY] {
            for side in [Side::Right, Side::Left, Side::Top, Side::Bottom] {
                assert_eq!(axis.to_output_side(axis.to_flow_side(side)), side);
            }
        }
        assert_eq!(Axis::AlongY.to_flow_side(Side::Top), Side::Left);
    }

    #[test]
    fn new_reads_the_input_into_flow_space() {
        let st = LayeredState::new(&input(Direction::TopBottom));
        assert_eq!(st.n_real, 2);
        assert_eq!(st.len(), 2);
        assert_eq!((st.nodes[0].w_along, st.nodes[0].w_cross), (40.0, 100.0));
        assert!(!st.nodes[0].ellipse && st.nodes[1].ellipse);
        assert_eq!(st.nodes[1].group, Some(7));
        assert_eq!(st.nodes[1].clear_after, 90.0);
        assert_eq!(st.nodes[0].port_sides, None);
        assert_eq!(st.nodes[1].port_sides, Some(PortSides::FacingPreferred));
        // (x, y) = (10, 20) is (along, cross) = (20, 10).
        assert_eq!(st.nodes[1].seed, Some((20.0, 10.0)));
        assert!(!st.nodes[0].pinned && st.nodes[1].pinned);
        assert_eq!(st.weight, vec![1.0]);
        assert_eq!(st.thickness, vec![2.0]);
        assert_eq!(st.arcs.len(), 1);
        assert_eq!((st.arcs[0].from, st.arcs[0].to), (0, 1));
        assert!(!st.arcs[0].reversed);
    }

    #[test]
    fn new_clamps_negative_clearance() {
        let mut inp = input(Direction::LeftRight);
        inp.nodes[1].clear_after = -30.0;
        assert_eq!(LayeredState::new(&inp).nodes[1].clear_after, 0.0);
    }

    #[test]
    fn sep_adds_the_gap_between_half_extents() {
        let mut st = LayeredState::new(&input(Direction::LeftRight));
        let p = LayoutOptions::layered_default();
        // Node 0 is 40 tall and reserves nothing, node 1 is 20 tall.
        assert_eq!(st.sep(0, 1, &p), 20.0 + 46.0 + 10.0);
        // Node 1's 90px clearance beats the node gap.
        assert_eq!(st.sep(1, 0, &p), 10.0 + 90.0 + 20.0);
        let d = st.add_dummy(0);
        assert_eq!(st.sep(0, d, &p), 20.0 + 30.0 + 0.0);
        // The second node's negative-side clearance widens the gap too.
        st.nodes[1].clear_before = 70.0;
        assert_eq!(st.sep(0, 1, &p), 20.0 + 70.0 + 10.0);
    }

    #[test]
    fn layers_and_order_stay_in_step() {
        let mut st = LayeredState::new(&input(Direction::LeftRight));
        st.nodes[0].rank = 1;
        st.nodes[1].rank = 0;
        st.rebuild_layers();
        assert_eq!(st.layers, vec![vec![1], vec![0]]);
        assert_eq!(st.order_of, vec![0, 0]);

        st.layers = vec![vec![1, 0], vec![]];
        st.reindex();
        assert_eq!(st.order_of, vec![1, 0]);
    }
}

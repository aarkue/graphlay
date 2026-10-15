//! Ingest turns the public [`Graph`] into the input the layered phases run on.
//!
//! Self-loops leave the arc list for [`LayeredInput::self_loops`], so no phase sees `from == to`.
//! An edge with [`crate::Edge::reverse_for_ranking`] enters as `target -> source`, recorded in
//! [`Prepared::pre_reversed`], while its port hints keep the caller's orientation. Per-edge vectors
//! are filled to full length so no phase needs a fallback.

use crate::error::LayoutError;
use crate::geometry::shape::NodeShape;
use crate::geometry::Side;
use crate::graph::{Graph, PortHint, PortSides, RankConstraint};
use crate::options::LayoutOptions;

use super::selfloop::{LoopEdge, LoopSlot};
use super::state::LayerConstraint;

/// One node as the layered phases see it.
#[derive(Clone)]
pub(crate) struct LayeredInputNode {
    pub width: f64,
    pub height: f64,
    pub shape: NodeShape,
    pub constraint: Option<LayerConstraint>,
    /// Lane group id. Same-group nodes keep a consistent order across layers.
    pub category: Option<u32>,
    /// Minimum clearance beyond this node's positive-order-side border.
    pub clear_after: f64,
    /// Minimum clearance beyond this node's negative-order-side border.
    pub clear_before: f64,
    /// Which borders an edge end may attach to. `None` defers to the drawing-wide setting.
    pub port_sides: Option<PortSides>,
    /// Sort key inside the node's group band ([`crate::Node::lane_key`]).
    pub lane_key: Option<i64>,
    /// Minimum clearance before this node's border on the layer axis, towards the previous layer
    /// ([`crate::Margins::along_neg`]).
    pub along_before: f64,
    /// Minimum clearance after this node's border on the layer axis, towards the next layer
    /// ([`crate::Margins::along_pos`]).
    pub along_after: f64,
}

/// The engine's input. Real nodes are indexed `0..n`, and an edge's index is its identity
/// throughout the engine.
#[derive(Clone)]
pub(crate) struct LayeredInput {
    pub nodes: Vec<LayeredInputNode>,
    pub edges: Vec<(usize, usize)>,
    /// Relative importance per edge, always the same length as `edges`.
    pub weights: Vec<f64>,
    /// Drawn stroke width per edge, always the same length as `edges`.
    pub thickness: Vec<f64>,
    /// `(width, height)` of each edge's mid-point label, `(0, 0)` for none. Empty when no edge has
    /// a label.
    pub edge_label_sizes: Vec<(f64, f64)>,
    /// Per-node warm-start centre, always the same length as `nodes`.
    pub seed: Vec<Option<(f64, f64)>>,
    /// Per-node hard-pin flag, always the same length as `nodes`.
    pub pinned: Vec<bool>,
    /// The self-loops, one slot per node that has any, in node order. They never become arcs.
    pub self_loops: Vec<LoopSlot>,
    /// Per edge, whether [`crate::Edge::reverse_for_ranking`] reversed it on entry. Empty when no
    /// edge was.
    pub ranked_backwards: Vec<bool>,
    /// Direction, spacing, routing style and effort.
    pub options: LayoutOptions,
}

impl LayeredInput {
    /// Re-checks endpoints and sizes. [`Graph::validate`] already rejects these, so this only
    /// guards callers that bypass ingest.
    pub fn validate(&self) -> Result<(), LayoutError> {
        let nodes = self.nodes.len();
        for (edge, &(a, b)) in self.edges.iter().enumerate() {
            for node in [a, b] {
                if node >= nodes {
                    return Err(LayoutError::EdgeEndpointOutOfRange {
                        edge: crate::graph::EdgeId(edge as u32),
                        node: node as u32,
                        nodes: nodes as u32,
                    });
                }
            }
        }
        for (node, nd) in self.nodes.iter().enumerate() {
            if !nd.width.is_finite() || !nd.height.is_finite() {
                return Err(LayoutError::NonFiniteSize {
                    node: crate::graph::NodeId(node as u32),
                });
            }
            if nd.width <= 0.0 || nd.height <= 0.0 {
                return Err(LayoutError::NonPositiveSize {
                    node: crate::graph::NodeId(node as u32),
                });
            }
        }
        Ok(())
    }
}

/// The caller's port hints for one edge, `[at source, at target]`, in the caller's orientation,
/// which may differ from the engine arc's.
pub(crate) type EdgePortHints = [Option<PortHint>; 2];

/// An ingested graph, with the engine's input and the map back to the caller's edge ids.
pub(crate) struct Prepared {
    /// The phases' input.
    pub input: LayeredInput,
    /// Engine edge index per graph edge, or `None` for a self-loop.
    pub engine_edge: Vec<Option<usize>>,
    /// Per graph edge, whether the engine arc runs `target -> source` because of
    /// [`crate::Edge::reverse_for_ranking`]. `finish::emit` flips the polyline back.
    pub pre_reversed: Vec<bool>,
    /// Per engine edge, the caller's port hints (see [`EdgePortHints`]). Empty when no engine edge
    /// carries one.
    pub port_hints: Vec<EdgePortHints>,
    /// Per graph edge, the hinted border of each end `[source, target]` in output space, as
    /// reported in [`crate::layout::PortInfo::side`]. `None` for an end without a hint, and for a
    /// self-loop whose hint conflicts with its node's other loops.
    pub hinted_sides: Vec<[Option<Side>; 2]>,
}

/// Builds the engine's input from a [`Graph`]. Run [`Graph::validate`] first, because this
/// assumes finite positive sizes and in-range endpoints.
pub(crate) fn ingest(g: &Graph, o: &LayoutOptions) -> Result<Prepared, LayoutError> {
    let nodes: Vec<LayeredInputNode> = g
        .nodes
        .iter()
        .map(|nd| LayeredInputNode {
            width: nd.size.width,
            height: nd.size.height,
            shape: nd.shape,
            constraint: match nd.rank {
                RankConstraint::First => Some(LayerConstraint::First),
                RankConstraint::Last => Some(LayerConstraint::Last),
                RankConstraint::Fixed(k) => Some(LayerConstraint::Fixed(k)),
                RankConstraint::Free => None,
            },
            category: nd.group.map(|gid| gid.0),
            clear_after: nd.margin.cross_pos,
            clear_before: nd.margin.cross_neg,
            port_sides: nd.port_sides,
            lane_key: nd.lane_key,
            along_before: nd.margin.along_neg.max(0.0),
            along_after: nd.margin.along_pos.max(0.0),
        })
        .collect();

    let mut engine_edge: Vec<Option<usize>> = Vec::with_capacity(g.edges.len());
    let mut pre_reversed: Vec<bool> = vec![false; g.edges.len()];
    let mut edges: Vec<(usize, usize)> = Vec::with_capacity(g.edges.len());
    let mut ranked_backwards: Vec<bool> = Vec::with_capacity(g.edges.len());
    let mut weights: Vec<f64> = Vec::with_capacity(g.edges.len());
    let mut thickness: Vec<f64> = Vec::with_capacity(g.edges.len());
    let mut label_sizes: Vec<(f64, f64)> = Vec::with_capacity(g.edges.len());
    let mut any_label = false;
    let mut loop_of: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
    let mut self_loops: Vec<LoopSlot> = Vec::new();
    let mut loop_hints: Vec<Vec<Side>> = Vec::new();
    let mut port_hints: Vec<EdgePortHints> = Vec::with_capacity(g.edges.len());
    let mut hinted_sides: Vec<[Option<Side>; 2]> = Vec::with_capacity(g.edges.len());
    for (id, e) in g.edges() {
        hinted_sides.push([e.source_port, e.target_port].map(|h| h.map(PortHint::side)));
        if e.is_self_loop() {
            engine_edge.push(None);
            let node = e.source.index();
            let slot = *loop_of.entry(node).or_insert_with(|| {
                self_loops.push(LoopSlot::new(node));
                loop_hints.push(Vec::new());
                self_loops.len() - 1
            });
            self_loops[slot].edges.push(LoopEdge {
                edge: id.index(),
                thickness: e.thickness,
                label: e.label_size.map_or((0.0, 0.0), |s| (s.width, s.height)),
            });
            loop_hints[slot].extend(
                [e.source_port, e.target_port]
                    .iter()
                    .flatten()
                    .map(|h| h.side()),
            );
            continue;
        }
        engine_edge.push(Some(edges.len()));
        port_hints.push([e.source_port, e.target_port]);
        ranked_backwards.push(e.reverse_for_ranking);
        if e.reverse_for_ranking {
            pre_reversed[id.index()] = true;
            edges.push((e.target.index(), e.source.index()));
        } else {
            edges.push((e.source.index(), e.target.index()));
        }
        weights.push(e.weight);
        thickness.push(e.thickness);
        let (lw, lh) = e.label_size.map_or((0.0, 0.0), |s| (s.width, s.height));
        any_label |= lw > 0.0 && lh > 0.0;
        label_sizes.push((lw, lh));
    }
    // A zero box already means no label, so the vector is either empty or complete.
    if !any_label {
        label_sizes.clear();
    }
    // A node's loops share one border, so hints count only when every hinted loop agrees.
    for (slot, sides) in self_loops.iter_mut().zip(&loop_hints) {
        if let Some(&first) = sides.first() {
            if sides.iter().all(|&s| s == first) {
                slot.hint = Some(first);
            }
        }
        if slot.hint.is_none() {
            for e in &slot.edges {
                if let Some(h) = hinted_sides.get_mut(e.edge) {
                    *h = [None, None];
                }
            }
        }
    }
    if port_hints.iter().all(|h| h.iter().all(Option::is_none)) {
        port_hints.clear();
    }

    let seed = g
        .nodes
        .iter()
        .map(|nd| nd.position.map(|p| (p.x, p.y)))
        .collect();
    let pinned = g.nodes.iter().map(|nd| nd.pinned).collect();

    Ok(Prepared {
        input: LayeredInput {
            nodes,
            edges,
            weights,
            thickness,
            edge_label_sizes: label_sizes,
            seed,
            pinned,
            self_loops,
            ranked_backwards: if ranked_backwards.contains(&true) {
                ranked_backwards
            } else {
                Vec::new()
            },
            options: *o,
        },
        engine_edge,
        pre_reversed,
        port_hints,
        hinted_sides,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::geometry::{Point, Size};
    use crate::graph::{Edge, GraphBuilder, GroupId, Node, NodeId};

    fn node(size: Size) -> Node {
        Node {
            size,
            ..Node::default()
        }
    }

    /// Self-loops stay out of the arc list and are collected per node in input order.
    #[test]
    fn ingest_holds_self_loops_out_of_the_engine() {
        let mut b = GraphBuilder::new();
        let a = b.add_node(node(Size::new(60.0, 40.0)));
        let c = b.add_node(node(Size::new(60.0, 40.0)));
        b.connect(a, a);
        b.connect(a, c);
        b.connect(c, c);
        b.connect(c, a);
        let g = b.build();

        let p = ingest(&g, &LayoutOptions::layered_default()).unwrap();
        assert_eq!(p.input.edges, vec![(0, 1), (1, 0)]);
        assert_eq!(p.engine_edge, vec![None, Some(0), None, Some(1)]);
        let slots = &p.input.self_loops;
        assert_eq!(slots.len(), 2);
        assert_eq!((slots[0].node, slots[1].node), (0, 1));
        assert_eq!(slots[0].edges.len(), 1);
        assert_eq!(slots[0].edges[0].edge, 0);
        assert_eq!(slots[1].edges[0].edge, 2);
        assert!(slots.iter().all(|s| s.side.is_none()));
    }

    /// Loops on one node share a slot in caller edge order, which is their nesting order.
    #[test]
    fn ingest_groups_the_loops_of_one_node_in_input_order() {
        let mut b = GraphBuilder::new();
        let a = b.add_node(node(Size::new(60.0, 40.0)));
        let c = b.add_node(node(Size::new(60.0, 40.0)));
        b.connect(a, c);
        for _ in 0..3 {
            b.connect(c, c);
        }
        let g = b.build();
        let p = ingest(&g, &LayoutOptions::layered_default()).unwrap();
        assert_eq!(p.input.self_loops.len(), 1);
        let edges: Vec<usize> = p.input.self_loops[0].edges.iter().map(|e| e.edge).collect();
        assert_eq!(edges, vec![1, 2, 3]);
    }

    #[test]
    fn ingest_fills_in_the_per_edge_defaults() {
        let mut b = GraphBuilder::new();
        let a = b.add_node(node(Size::new(60.0, 40.0)));
        let c = b.add_node(node(Size::new(60.0, 40.0)));
        b.connect(a, c);
        let g = b.build();
        let p = ingest(&g, &LayoutOptions::layered_default()).unwrap();
        assert_eq!(p.input.weights, vec![1.0]);
        assert_eq!(p.input.thickness, vec![2.0]);
        assert!(p.input.edge_label_sizes.is_empty());
    }

    /// One labelled edge makes the whole vector present, zero-filled for the others.
    #[test]
    fn ingest_pads_the_label_vector_when_any_edge_has_one() {
        let mut b = GraphBuilder::new();
        let a = b.add_node(node(Size::new(60.0, 40.0)));
        let c = b.add_node(node(Size::new(60.0, 40.0)));
        b.connect(a, c);
        b.add_edge(Edge {
            source: c,
            target: a,
            label_size: Some(Size::new(40.0, 14.0)),
            ..Edge::default()
        });
        let g = b.build();
        let p = ingest(&g, &LayoutOptions::layered_default()).unwrap();
        assert_eq!(p.input.edge_label_sizes, vec![(0.0, 0.0), (40.0, 14.0)]);
    }

    #[test]
    fn ingest_flips_an_edge_ranked_against_its_direction() {
        let mut b = GraphBuilder::new();
        let a = b.add_node(node(Size::new(60.0, 40.0)));
        let c = b.add_node(node(Size::new(60.0, 40.0)));
        b.add_edge(Edge {
            source: a,
            target: c,
            reverse_for_ranking: true,
            ..Edge::default()
        });
        b.connect(a, c);
        let g = b.build();

        let p = ingest(&g, &LayoutOptions::layered_default()).unwrap();
        assert_eq!(p.input.edges, vec![(1, 0), (0, 1)]);
        assert_eq!(p.pre_reversed, vec![true, false]);
    }

    /// Port hints keep the caller's orientation even for an edge ranked backwards. A loop's hint
    /// becomes its slot's side only when the node's hinted loops agree.
    #[test]
    fn ingest_carries_port_hints_and_agreed_loop_sides() {
        use crate::geometry::Side;
        let mut b = GraphBuilder::new();
        let a = b.add_node(node(Size::new(60.0, 40.0)));
        let c = b.add_node(node(Size::new(60.0, 40.0)));
        b.add_edge(Edge {
            source_port: Some(PortHint::Side(Side::Top)),
            reverse_for_ranking: true,
            ..Edge::new(a, c)
        });
        b.connect(c, a);
        for (n, side) in [
            (a, Side::Bottom),
            (a, Side::Bottom),
            (c, Side::Left),
            (c, Side::Top),
        ] {
            b.add_edge(Edge {
                target_port: Some(PortHint::Side(side)),
                ..Edge::new(n, n)
            });
        }
        let g = b.build();
        let p = ingest(&g, &LayoutOptions::layered_default()).unwrap();
        assert_eq!(p.input.edges, vec![(1, 0), (1, 0)]);
        assert_eq!(
            p.port_hints,
            vec![[Some(PortHint::Side(Side::Top)), None], [None, None]]
        );
        let hint_of = |node: usize| {
            p.input
                .self_loops
                .iter()
                .find(|s| s.node == node)
                .and_then(|s| s.hint)
        };
        assert_eq!(hint_of(0), Some(Side::Bottom));
        assert_eq!(hint_of(1), None);
        assert_eq!(p.hinted_sides[0], [Some(Side::Top), None]);
        assert_eq!(p.hinted_sides[2], [None, Some(Side::Bottom)]);
        assert_eq!(
            p.hinted_sides[4],
            [None, None],
            "a loop side its node could not honour"
        );

        let mut plain = g.clone();
        plain.edges[0].source_port = None;
        assert!(ingest(&plain, &LayoutOptions::layered_default())
            .unwrap()
            .port_hints
            .is_empty());
    }

    #[test]
    fn ingest_maps_the_node_properties() {
        let mut b = GraphBuilder::new();
        b.add_node(Node {
            size: Size::new(60.0, 40.0),
            shape: NodeShape::Ellipse,
            rank: RankConstraint::Last,
            group: Some(GroupId(3)),
            position: Some(Point::new(7.0, 9.0)),
            pinned: true,
            margin: crate::graph::Margins {
                cross_pos: 90.0,
                ..crate::graph::Margins::default()
            },
            port_sides: Some(PortSides::FacingOnly),
            ..Node::default()
        });
        let g = b.build();
        let p = ingest(&g, &LayoutOptions::layered_default()).unwrap();
        let nd = &p.input.nodes[0];
        assert_eq!(nd.shape, NodeShape::Ellipse);
        assert_eq!(nd.constraint, Some(LayerConstraint::Last));
        assert_eq!(nd.category, Some(3));
        assert_eq!(nd.clear_after, 90.0);
        assert_eq!(nd.clear_before, 0.0);
        assert_eq!(nd.port_sides, Some(PortSides::FacingOnly));
        assert_eq!(p.input.seed[0], Some((7.0, 9.0)));
        assert!(p.input.pinned[0]);
        assert_eq!(NodeId(0).index(), 0);
    }
}

//! Deterministic graph layout in pure Rust.
//!
//! Build a [`Graph`], pass it to [`layout()`] and draw the [`Layout`] that comes back. The main
//! engine is layered (Sugiyama-style): cycle breaking, layering, crossing minimisation,
//! network-simplex or Brandes-Kopf coordinates, and orthogonal or diagonal edge routing. A
//! tidy-tree engine ([`tree::layout`]) and a re-route pass for nodes the user has moved
//! ([`reroute`]) sit beside it. A node is a box with a shape, and an edge is a weighted link
//! between two nodes.
//!
//! ```
//! use graphlay::{layout, GraphBuilder, LayoutOptions, Node, RouteKind, Size};
//!
//! let mut b = GraphBuilder::new();
//! let start = b.add_node(Node { size: Size::new(120.0, 52.0), ..Node::default() });
//! let check = b.add_node(Node { size: Size::new(120.0, 52.0), ..Node::default() });
//! let ship = b.add_node(Node { size: Size::new(120.0, 52.0), ..Node::default() });
//! b.connect(start, check);
//! b.connect(check, ship);
//! b.connect(start, ship);          // skips a layer, so it is routed through a waypoint chain
//! let graph = b.build();
//!
//! let drawing = layout(&graph, &LayoutOptions::layered_default())?;
//!
//! for (id, placement) in graph.nodes().map(|(id, _)| id).zip(&drawing.nodes) {
//!     println!("node {} at ({:.0}, {:.0}), layer {:?}",
//!              id.0, placement.center.x, placement.center.y, placement.rank);
//! }
//! for (id, route) in graph.edges().map(|(id, _)| id).zip(&drawing.edges) {
//!     match route.kind {
//!         RouteKind::SelfLoop => println!("edge {} loops back on its own node", id.0),
//!         _ => println!("edge {} runs {} points from the {:?} border",
//!                       id.0, route.points.len(),
//!                       route.source_port.map(|p| p.side)),
//!     }
//! }
//! println!("{} layers, bounds {:?}", drawing.layer_count, drawing.bounds);
//! assert_eq!(drawing.layer_count, 3);
//! assert_eq!(drawing.edges[2].kind, RouteKind::Chained { spans: 2 });
//! # Ok::<(), graphlay::LayoutError>(())
//! ```
//!
//! # Entry points
//!
//! | Call | Does |
//! |---|---|
//! | [`layout()`] | A layered drawing, from scratch or from [`Node::position`] seeds. |
//! | [`reroute()`] | Keeps every node in place and recomputes layers and routes. |
//! | [`tree::layout`] | A tidy tree, parents centred over their children. |
//! | [`json::LayoutRequest::split`] | The JSON form: a graph, a [`Preset`] and an options patch. |
//! | [`svg::sketch()`] | Draws a layout as SVG text. [`svg::render()`] draws a styled [`svg::Scene`]. |
//!
//! Coordinates are px in output space, with `x` growing right and `y` growing down for every
//! [`Direction`]. Options whose meaning depends on the direction ([`Margins`], [`LoopSide`],
//! [`PortSides`]) are named in flow space. "Along" is the axis the layers follow, "cross" the
//! axis one layer's nodes spread on.
//!
//! The input model also covers lanes per group ([`Node::group`], [`GroupLanes`],
//! [`Node::lane_key`]), compounds ([`Node::parent`]), port hints ([`PortHint`]), fixed layers
//! ([`RankConstraint`]), edges ranked against their direction ([`Edge::reverse_for_ranking`]),
//! edge labels ([`Edge::label_size`]), self-loops and stable relayout from seeds ([`SeedMode`]).
//!
//! # Modules
//!
//! * [`mod@graph`]: the input model, [`Graph`], [`Node`], [`Edge`] and [`GraphBuilder`].
//! * [`options`]: every knob with its default, and the presets such as
//!   [`LayoutOptions::layered_default`] and [`LayoutOptions::process_map`].
//! * [`mod@layout`]: the output model, [`Layout`], [`Placement`], [`EdgeRoute`] and [`PortInfo`].
//! * [`mod@tree`]: the tidy-tree engine and its [`TreeOptions`].
//! * [`error`]: [`LayoutError`], the one error type.
//! * [`geometry`]: points, sizes, rectangles and node outlines.
//! * [`pipeline`]: a look inside the layered phases, for evaluation.
//! * [`metrics`]: quality measures for comparing two layouts of the same graph.
//! * [`svg`]: an SVG renderer that makes no layout decisions ([`svg::Scene`]).
//!   Feature `svg`, on by default.
//! * [`json`]: layout requests as JSON. Feature `serde`, on by default.
//!
//! The README and the `docs/` folder of the repository have a guide, the architecture and how
//! layout quality is measured.
//!
//! # Guarantees
//!
//! * **No panics on caller data.** Bad input returns a [`LayoutError`]. `unwrap` and `expect` are
//!   denied outside tests, and every float sort uses `total_cmp`.
//! * **Deterministic.** The same input gives the same output bytes on every platform and with or
//!   without the `parallel` feature. There is one seeded generator and no wall clock.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]
#![warn(missing_docs)]

mod canonical;
mod compound;
pub mod error;
#[cfg(feature = "fixtures")]
pub mod fixture;
pub mod geometry;
pub mod graph;
#[cfg(feature = "serde")]
pub mod json;
mod layered;
pub mod layout;
pub mod metrics;
pub mod options;
pub mod pipeline;
mod sort;
mod splits;
#[cfg(feature = "svg")]
pub mod svg;
#[cfg(feature = "fixtures")]
pub mod synthetic;
pub mod tree;

pub use error::{LayoutError, TreeReason};
pub use geometry::{Point, Rect, Shape, Side, Size};
pub use graph::{
    Edge, EdgeId, Graph, GraphBuilder, GroupId, Margins, Node, NodeId, PortHint, PortSides,
    RankConstraint,
};
pub use layout::{EdgeRoute, Lane, Layout, LayoutStats, Placement, PortInfo, RouteKind};
/// Which optimiser places the order axis ([`CoordOptions::placement`]).
///
/// Renamed at the crate root because [`Placement`] there is the output model's per-node placement.
pub use options::Placement as CrossPlacement;
pub use options::{
    BranchStyle, Compaction, CompactionOptions, CompactionTuning, CoordOptions, CoordTuning,
    CycleOptions, CycleStrategy, Direction, Effort, EffortBudget, GroupLanes, LabelOptions,
    LabelPlacement, LabelReservation, LabelTuning, LayoutOptions, Limits, LoopSide, MergeEdges,
    OrderHeuristic, OrderOptions, OrderTuning, PortCentring, Preset, Routing, RoutingStyle,
    RoutingTuning, SeedMode, SelfLoopOptions, Spacing, Tuning, WrapOptions, WrapTuning,
};
pub use tree::{TreeEdgeStyle, TreeOptions};

/// Lays `g` out with the layered (Sugiyama-style) engine.
///
/// The result is indexed like the input. `layout.nodes[i]` is where `g.nodes[i]` went and
/// `layout.edges[i]` is how `g.edges[i]` was drawn, in output coordinates for
/// [`LayoutOptions::direction`].
///
/// A graph with compounds ([`Node::parent`]) is laid out bottom-up, each compound sized to hold
/// its children's drawing. Nodes with a [`Node::position`] steer the result as [`SeedMode`] says.
///
/// ```
/// use graphlay::{layout, GraphBuilder, LayoutOptions, Size};
///
/// let mut b = GraphBuilder::new();
/// let start = b.add_box(Size::new(120.0, 50.0));
/// let order = b.add_box(Size::new(120.0, 50.0));
/// let ship = b.add_box(Size::new(120.0, 50.0));
/// b.connect(start, order);
/// b.connect(order, ship);
/// let retry = b.connect(ship, order); // closes a cycle
/// let l = layout(&b.build(), &LayoutOptions::layered_default())?;
///
/// // Left to right in flow order, with the cycle's back edge drawn against the flow...
/// assert!(l.nodes[0].center.x < l.nodes[1].center.x);
/// assert!(l.nodes[1].center.x < l.nodes[2].center.x);
/// assert_eq!(l.reversed, vec![retry]);
/// // ...but its polyline still starts at its own source, `ship`.
/// let route = &l.edges[retry.index()];
/// assert_eq!(route.points.first(), route.source_port.map(|p| p.point).as_ref());
/// assert!((route.points[0].x - l.nodes[2].center.x).abs() <= 60.0);
/// # Ok::<(), graphlay::LayoutError>(())
/// ```
///
/// # Errors
///
/// Returns the errors of [`LayoutOptions::validate`], [`Graph::validate`] and [`Limits::check`],
/// which all run before any layout work.
pub fn layout(g: &Graph, o: &LayoutOptions) -> Result<Layout, LayoutError> {
    let o = &o.with_effort_applied();
    o.validate()?;
    g.validate()?;
    o.limits.check(g)?;
    if compound::has_compounds(g) {
        return compound::layout(g, o, layout_flat);
    }
    layout_flat(g, o)
}

/// Fails on the first node with a parent, for entry points that do not support compounds.
pub(crate) fn reject_compounds(g: &Graph) -> Result<(), LayoutError> {
    match g.nodes().find(|(_, nd)| nd.parent.is_some()) {
        Some((node, _)) => Err(LayoutError::NestedNodesUnsupported { node }),
        None => Ok(()),
    }
}

/// [`layout()`] of a graph without compounds, options already validated.
fn layout_flat(g: &Graph, o: &LayoutOptions) -> Result<Layout, LayoutError> {
    if let Some(c) = canonical::canonical(g) {
        return Ok(canonical::restore(&c, layout_as_given(&c.graph, o)?));
    }
    layout_as_given(g, o)
}

fn layout_as_given(g: &Graph, o: &LayoutOptions) -> Result<Layout, LayoutError> {
    let prepared = layered::prepare::ingest(g, o)?;
    let out = layered::layout_layered(&prepared.input, &prepared.port_hints)?;
    Ok(layered::finish::emit(g, o, &out, &prepared))
}

/// Re-routes `g`'s edges around the node positions the caller already has.
///
/// Every node keeps its [`Node::position`], so every node must have one. Use it after the user
/// drops a dragged node. It does not bundle parallel edges, wrap into rows or place labels.
///
/// ```
/// use graphlay::{layout, reroute, GraphBuilder, LayoutOptions, Point, Size};
///
/// let mut b = GraphBuilder::new();
/// let a = b.add_box(Size::new(120.0, 50.0));
/// let c = b.add_box(Size::new(120.0, 50.0));
/// b.connect(a, c);
/// let mut g = b.build();
/// let o = LayoutOptions::layered_default();
/// let first = layout(&g, &o)?;
/// for (node, placed) in g.nodes.iter_mut().zip(&first.nodes) {
///     node.position = Some(placed.center);
/// }
/// // The user drags `c` 200px down.
/// let dropped = Point::new(first.nodes[1].center.x, first.nodes[1].center.y + 200.0);
/// g.nodes[1].position = Some(dropped);
///
/// let l = reroute(&g, &o)?;
/// assert_eq!(l.nodes[0].center, first.nodes[0].center);
/// assert_eq!(l.nodes[1].center, dropped);
/// // The edge ends on the border of the box where `c` was dropped.
/// let end = *l.edges[0].points.last().expect("routed");
/// let (dx, dy) = ((end.x - dropped.x).abs(), (end.y - dropped.y).abs());
/// assert!(dx <= 60.5 && dy <= 25.5 && (dx >= 59.5 || dy >= 24.5));
/// # Ok::<(), graphlay::LayoutError>(())
/// ```
///
/// # Errors
///
/// As [`layout()`], plus [`LayoutError::MissingPosition`] for any node without a position and
/// [`LayoutError::NestedNodesUnsupported`] for a graph with compounds.
pub fn reroute(g: &Graph, o: &LayoutOptions) -> Result<Layout, LayoutError> {
    let o = &o.with_effort_applied();
    o.validate()?;
    g.validate()?;
    reject_compounds(g)?;
    g.validate_positions()?;
    o.limits.check(g)?;
    if let Some(c) = canonical::canonical(g) {
        return Ok(canonical::restore(&c, reroute_as_given(&c.graph, o)?));
    }
    reroute_as_given(g, o)
}

fn reroute_as_given(g: &Graph, o: &LayoutOptions) -> Result<Layout, LayoutError> {
    let prepared = layered::prepare::ingest(g, o)?;
    let out = layered::reroute::reroute_from_positions(&prepared.input, &prepared.port_hints)?;
    Ok(layered::finish::emit(g, o, &out, &prepared))
}
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod api_tests {
    use super::*;
    use crate::geometry::fp;

    fn boxes(n: usize) -> GraphBuilder {
        let mut b = GraphBuilder::new();
        for _ in 0..n {
            b.add_box(Size::new(100.0, 50.0));
        }
        b
    }

    #[test]
    fn an_empty_graph_lays_out() {
        let g = Graph::new();
        let o = LayoutOptions::layered_default();
        assert_eq!(layout(&g, &o).unwrap(), Layout::default());
        assert_eq!(reroute(&g, &o).unwrap(), Layout::default());
    }

    /// `reverse_for_ranking` puts the target in the lower layer, but the polyline still runs
    /// source to target so the arrowhead lands at the target.
    #[test]
    fn an_edge_can_be_ranked_against_its_direction() {
        let mut b = boxes(2);
        let (a, c) = (NodeId(0), NodeId(1));
        b.add_edge(Edge {
            source: a,
            target: c,
            reverse_for_ranking: true,
            ..Edge::default()
        });
        let g = b.build();
        let l = layout(&g, &LayoutOptions::layered_default()).unwrap();

        let (ra, rc) = (l.nodes[0].rank.unwrap(), l.nodes[1].rank.unwrap());
        assert!(rc < ra, "expected b above a, got ranks {ra} / {rc}");
        let route = &l.edges[0];
        assert!(route.points.len() >= 2);
        assert_eq!(
            route.source_port.map(|p| p.point),
            route.points.first().copied()
        );
        let (first, last) = (route.points[0], route.points[route.points.len() - 1]);
        let near = |p: crate::Point, at: NodeId| {
            let c = l.nodes[at.index()].center;
            fp::hypot(p.x - c.x, p.y - c.y)
        };
        assert!(
            near(first, a) < near(first, c),
            "route must start at the source"
        );
        assert!(
            near(last, c) < near(last, a),
            "route must end at the target"
        );
        assert_eq!(l.reversed, vec![EdgeId(0)]);

        let mut b = boxes(2);
        b.connect(a, c);
        let plain = layout(&b.build(), &LayoutOptions::layered_default()).unwrap();
        assert!(plain.nodes[0].rank.unwrap() < plain.nodes[1].rank.unwrap());
        assert!(plain.reversed.is_empty());
    }

    #[test]
    fn a_layout_reports_the_structure_it_found() {
        let mut b = boxes(4);
        let (n0, n1, n2, n3) = (NodeId(0), NodeId(1), NodeId(2), NodeId(3));
        b.connect(n0, n1);
        b.connect(n1, n2);
        b.connect(n2, n3);
        b.connect(n0, n3); // spans three layers, so it needs a chain of waypoints
        let g = b.build();
        let l = layout(&g, &LayoutOptions::layered_default()).unwrap();

        assert_eq!(l.layer_count, 4);
        let ranks: Vec<Option<i32>> = l.nodes.iter().map(|p| p.rank).collect();
        assert_eq!(ranks, vec![Some(0), Some(1), Some(2), Some(3)]);
        assert!(l.nodes.iter().all(|p| p.size == Size::new(100.0, 50.0)));
        assert_eq!(l.edges[3].kind, RouteKind::Chained { spans: 3 });
        assert_eq!(l.edges[0].kind, RouteKind::Direct);
        assert_eq!(l.stats.dummy_count, 2);
        assert!(l.bounds.width() > 0.0 && l.bounds.height() > 0.0);
        for e in &l.edges {
            assert!(e.source_port.is_some() && e.target_port.is_some());
            assert_eq!(
                e.points.first().map(|p| p.x),
                e.source_port.map(|p| p.point.x)
            );
        }
    }

    #[test]
    fn a_reversed_edge_is_named_and_still_emitted_forwards() {
        let mut b = boxes(2);
        b.connect(NodeId(0), NodeId(1));
        b.connect(NodeId(1), NodeId(0));
        let g = b.build();
        let l = layout(&g, &LayoutOptions::layered_default()).unwrap();
        assert_eq!(l.reversed, vec![EdgeId(1)]);
        let route = &l.edges[1];
        let start = route.points[0];
        let target = l.nodes[0].center;
        let source = l.nodes[1].center;
        let d = |a: Point, b: Point| fp::hypot(a.x - b.x, a.y - b.y);
        assert!(
            d(start, source) < d(start, target),
            "the reversed edge's polyline must still start at its source"
        );
    }

    #[test]
    fn a_self_loop_is_routed_and_changes_nothing_else() {
        let mut b = boxes(2);
        b.connect(NodeId(0), NodeId(1));
        let plain = layout(&b.clone().build(), &LayoutOptions::layered_default()).unwrap();

        b.connect(NodeId(1), NodeId(1));
        let with_loop = layout(&b.build(), &LayoutOptions::layered_default()).unwrap();

        assert_eq!(with_loop.edges.len(), 2);
        assert_eq!(with_loop.edges[1].kind, RouteKind::SelfLoop);
        assert_eq!(with_loop.edges[1].points.len(), 4);
        assert!(with_loop.edges[1].source_port.is_some());
        // With one node per layer the loop's reservation widens nothing.
        assert_eq!(with_loop.nodes, plain.nodes);
        assert_eq!(with_loop.edges[0], plain.edges[0]);
        // The bounds still grow to hold the bump.
        assert!(with_loop.bounds != plain.bounds);
    }

    #[test]
    fn parallel_edges_come_back_as_bundle_lanes() {
        let mut b = boxes(2);
        for _ in 0..3 {
            b.connect(NodeId(0), NodeId(1));
        }
        let g = b.build();
        let l = layout(&g, &LayoutOptions::layered_default()).unwrap();
        assert_eq!(
            l.edges.iter().map(|e| e.kind).collect::<Vec<_>>(),
            vec![
                RouteKind::Bundled { lane: 0 },
                RouteKind::Bundled { lane: 1 },
                RouteKind::Bundled { lane: 2 },
            ]
        );
    }

    /// Edgeless nodes are packed outside the layers and have no rank.
    #[test]
    fn an_isolated_node_has_no_rank() {
        let mut b = boxes(3);
        b.connect(NodeId(0), NodeId(1));
        let g = b.build();
        let l = layout(&g, &LayoutOptions::layered_default()).unwrap();
        assert_eq!(l.nodes[0].rank, Some(0));
        assert_eq!(l.nodes[2].rank, None);
    }

    #[test]
    fn reroute_requires_a_position_per_node() {
        let mut b = boxes(2);
        b.connect(NodeId(0), NodeId(1));
        let mut g = b.build();
        let o = LayoutOptions::layered_default();
        assert_eq!(
            reroute(&g, &o).unwrap_err(),
            LayoutError::MissingPosition { node: NodeId(0) }
        );
        g.nodes[0].position = Some(Point::new(0.0, 0.0));
        g.nodes[1].position = Some(Point::new(300.0, 0.0));
        let l = reroute(&g, &o).unwrap();
        assert_eq!(l.nodes[0].center, Point::new(0.0, 0.0));
        assert_eq!(l.nodes[1].center, Point::new(300.0, 0.0));
    }

    #[test]
    fn limits_are_enforced_by_the_entry_points() {
        let mut b = boxes(3);
        b.connect(NodeId(0), NodeId(1));
        let g = b.build();
        let o = LayoutOptions {
            limits: Limits {
                max_nodes: Some(2),
                ..Limits::default()
            },
            ..LayoutOptions::layered_default()
        };
        assert_eq!(
            layout(&g, &o).unwrap_err(),
            LayoutError::TooLarge {
                what: "nodes",
                got: 3,
                limit: 2
            }
        );
    }

    #[test]
    fn nan_and_zero_weights_are_rejected_not_laid_out() {
        let mut b = boxes(2);
        let e = b.connect(NodeId(0), NodeId(1));
        for (weight, expected) in [
            (f64::NAN, LayoutError::NonFiniteWeight { edge: e }),
            (f64::INFINITY, LayoutError::NonFiniteWeight { edge: e }),
            (0.0, LayoutError::NonPositiveWeight { edge: e }),
            (-1.0, LayoutError::NonPositiveWeight { edge: e }),
        ] {
            let mut g = b.clone().build();
            g.edges[0].weight = weight;
            assert_eq!(
                layout(&g, &LayoutOptions::layered_default()).unwrap_err(),
                expected,
                "weight {weight}"
            );
        }
    }

    #[test]
    fn tree_layout_uses_the_same_output_model() {
        let mut b = boxes(3);
        b.connect(NodeId(0), NodeId(1));
        b.connect(NodeId(0), NodeId(2));
        let g = b.build();
        let l = tree::layout(&g, &TreeOptions::default()).unwrap();
        assert_eq!(l.nodes.len(), 3);
        assert_eq!(l.edges.len(), 2);
        // The default org-chart elbow gives a side child four points.
        assert!(l
            .edges
            .iter()
            .all(|e| e.points.len() == 4 && e.kind == RouteKind::Direct));
        assert!(l
            .edges
            .iter()
            .all(|e| e.source_port.is_some() && e.target_port.is_some()));
    }
}

/// Runs the README's Rust examples as doctests.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

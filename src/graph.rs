//! The input model for [`crate::layout()`], [`crate::reroute`] and [`crate::tree::layout`].
//!
//! A [`Graph`] is two plain vectors, and [`GraphBuilder`] is an optional convenience. Nodes and
//! edges are addressed by position, wrapped in [`NodeId`] and [`EdgeId`] so an index into the
//! wrong vector does not type-check. Every entry point runs [`Graph::validate`] first, so nothing
//! panics on caller data.

use serde::{Deserialize, Serialize};

use crate::error::LayoutError;
use crate::geometry::{Point, Shape, Side, Size};

/// Index of a node in [`Graph::nodes`].
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize,
)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct NodeId(pub u32);

/// Index of an edge in [`Graph::edges`].
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize,
)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct EdgeId(pub u32);

/// A lane group. Nodes sharing one are drawn in a lane of their own.
///
/// Groups are opaque ids, only compared and ordered. Lanes appear only when nodes carry at least
/// two distinct groups (see [`Node::group`]).
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize,
)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct GroupId(pub u32);

impl NodeId {
    /// The index as a `usize`, for indexing [`Graph::nodes`] or a layout's placements.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl EdgeId {
    /// The index as a `usize`, for indexing [`Graph::edges`] or a layout's routes.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Where a node is allowed to land on the layer axis.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum RankConstraint {
    /// Wherever layering puts it.
    #[default]
    Free,
    /// Pinned to the first layer (a source terminal).
    First,
    /// Pinned to the last layer (a sink terminal).
    Last,
    /// A layer relative to the other fixed nodes. Equal values share a layer, and a larger value
    /// lands at least the difference further along the flow.
    ///
    /// An edge between two equal values is an error ([`LayoutError::EdgeWithinFixedLayer`]). If a
    /// path through free nodes contradicts the fixed layers, all of them are treated as free.
    Fixed(i32),
}

/// Extra clearance in px outside a node's border, per side, for content the layout cannot see
/// (e.g., a badge).
///
/// Names are in flow terms, so they hold in every [`crate::Direction`]. `along` is the layer
/// axis, `cross` the order axis. Margins keep other nodes clear, but edges may pass through.
/// Self-loops reserve their own room on top.
#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct Margins {
    /// Towards the previous layer.
    pub along_neg: f64,
    /// Towards the next layer.
    pub along_pos: f64,
    /// Towards the negative order side (screen up in `LeftRight`, screen left in `TopBottom`).
    pub cross_neg: f64,
    /// Towards the positive order side (screen down in `LeftRight`, screen right in `TopBottom`).
    pub cross_pos: f64,
}

/// Where an edge end must attach to its node, set in [`Edge::source_port`] and
/// [`Edge::target_port`].
///
/// Sides are in output (screen) terms, like [`crate::layout::PortInfo::side`], so [`Side::Top`]
/// is the drawn top border whatever the [`crate::Direction`]. A hint overrides
/// [`crate::Routing::port_sides`] and [`Node::port_sides`].
///
/// A hinted edge is never bundled ([`crate::Routing::bundle_parallel`]) or merged
/// ([`crate::Routing::merge_edges`]). A self-loop honours only the side, and only when all hinted
/// loops on its node agree. [`crate::tree::layout`] ignores hints.
///
/// ```
/// use graphlay::{layout, Edge, GraphBuilder, LayoutOptions, PortHint, Side, Size};
///
/// let mut b = GraphBuilder::new();
/// let a = b.add_box(Size::new(120.0, 50.0));
/// let c = b.add_box(Size::new(120.0, 50.0));
/// // Leave `a` through the middle of its bottom border, and enter `c` from above.
/// b.add_edge(Edge {
///     source_port: Some(PortHint::Offset { side: Side::Bottom, t: 0.0 }),
///     target_port: Some(PortHint::Side(Side::Top)),
///     ..Edge::new(a, c)
/// });
/// let l = layout(&b.build(), &LayoutOptions::layered_default())?;
///
/// let route = &l.edges[0];
/// let (pa, pc) = (l.nodes[0].center, l.nodes[1].center);
/// assert_eq!(route.source_port.map(|p| p.side), Some(Side::Bottom));
/// assert_eq!(route.target_port.map(|p| p.side), Some(Side::Top));
/// let (first, last) = (route.points[0], route.points[route.points.len() - 1]);
/// assert!((first.x - pa.x).abs() < 0.5 && (first.y - (pa.y + 25.0)).abs() < 0.5);
/// assert!((last.y - (pc.y - 25.0)).abs() < 0.5);
/// # Ok::<(), graphlay::LayoutError>(())
/// ```
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum PortHint {
    /// Force a border side, leaving the position along it to the router.
    Side(Side),
    /// Force a border side and the position along it.
    Offset {
        /// The border to attach to.
        side: Side,
        /// Position along the border in `-1..=1`, as a share of its half-extent. `0` is the
        /// middle, `-1` the left end of a top or bottom border and the top end of a side border.
        /// Values near the corners are pulled in to the router's port limit.
        t: f64,
    },
}

impl PortHint {
    /// The border this hint names, in output terms.
    pub fn side(self) -> Side {
        match self {
            PortHint::Side(side) | PortHint::Offset { side, .. } => side,
        }
    }

    /// The position along the border for a [`PortHint::Offset`], else `None`.
    pub fn offset(self) -> Option<f64> {
        match self {
            PortHint::Side(_) => None,
            PortHint::Offset { t, .. } => Some(t),
        }
    }
}

/// How readily an edge end may attach to a perpendicular border (e.g., top or bottom in a
/// left-to-right drawing) instead of the border facing along the flow.
///
/// The router moves an end to a perpendicular border to save a bend, only when that adds no
/// crossing in the layer gap before the node. Set per drawing in [`crate::Routing::port_sides`]
/// and per node in [`Node::port_sides`]. Under [`crate::RoutingStyle::Diagonal`],
/// `FacingPreferred` behaves like `Any`, and ellipse ports are radial and follow neither.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum PortSides {
    /// A perpendicular border whenever it is free of crossings. The default.
    #[default]
    Any,
    /// Always the facing border, even at the cost of a bend.
    FacingOnly,
    /// A perpendicular border only when it is free of crossings and the facing border would cost
    /// a crossing or overfill its ports at the port gap. Otherwise the facing border.
    FacingPreferred,
    /// [`PortSides::FacingPreferred`] for an edge between adjacent layers, [`PortSides::Any`] for
    /// a longer one, which would otherwise jog at each end to reach its track.
    LongEdgesAny,
}

/// One node of the input graph.
///
/// Only [`Node::size`] must be set, so `Node { size, ..Node::default() }` is the usual form. The
/// default size is `0 x 0`, which [`Graph::validate`] rejects.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct Node {
    /// Width and height of the node's box, in px. Both must be finite and `> 0`.
    pub size: Size,
    /// Border shape. The box in [`Node::size`] is the bounding box either way.
    pub shape: Shape,
    /// Where the node may land on the layer axis.
    pub rank: RankConstraint,
    /// Lane group, e.g., an object type or a swimlane.
    ///
    /// Nodes sharing a group are drawn in a lane, a band of the cross axis. Lanes cost crossings,
    /// so [`crate::OrderOptions`] budgets them. With fewer than two distinct groups there are no
    /// lanes.
    pub group: Option<GroupId>,
    /// Sort key within a group's lane. In each layer, keyed members of a lane are sorted among
    /// the positions they already hold, and unkeyed members stay put.
    pub lane_key: Option<i64>,
    /// Warm-start centre in output space. If any node has one, cross-axis placement follows the
    /// seeds, which keeps a relayout stable, and [`crate::SeedMode`] says whether layers and order
    /// are kept too. Required on every node by [`crate::reroute`]. Ignored in a graph with
    /// compounds ([`Node::parent`]).
    pub position: Option<Point>,
    /// Hold [`Node::position`] exactly on both axes, e.g., for a just-dragged node. Other nodes
    /// make room. No effect without a position.
    pub pinned: bool,
    /// Extra clearance outside the node's border, for caller-drawn content.
    pub margin: Margins,
    /// Which borders an edge end may attach to on this node. `None` inherits
    /// [`crate::Routing::port_sides`].
    pub port_sides: Option<PortSides>,
    /// The compound this node is drawn inside, if any. Parents must not form a cycle.
    /// [`crate::reroute`] and [`crate::tree::layout`] reject graphs that use it.
    ///
    /// A compound's box is its children's drawing plus [`crate::Spacing::compound_padding`] and
    /// [`crate::Spacing::compound_header`], and at least its own [`Node::size`]. Its
    /// [`crate::Placement`] carries that size.
    ///
    /// ```
    /// use graphlay::{layout, GraphBuilder, LayoutOptions, Node, Rect, Size};
    ///
    /// let mut b = GraphBuilder::new();
    /// let order = b.add_box(Size::new(80.0, 40.0));
    /// let billing = b.add_box(Size::new(1.0, 1.0)); // the compound, sized by its children
    /// let inside = Node { size: Size::new(80.0, 40.0), parent: Some(billing), ..Node::default() };
    /// let invoice = b.add_node(inside.clone());
    /// let pay = b.add_node(inside);
    /// b.connect(order, invoice); // crosses billing's border
    /// b.connect(invoice, pay);
    /// let l = layout(&b.build(), &LayoutOptions::layered_default())?;
    ///
    /// let boxed = |i: usize| Rect::from_center(l.nodes[i].center, l.nodes[i].size);
    /// let outer = boxed(billing.index());
    /// assert!(outer.width() > 2.0 * 80.0);
    /// for child in [invoice, pay] {
    ///     let inner = boxed(child.index());
    ///     assert!(inner.min.x >= outer.min.x && inner.max.x <= outer.max.x);
    ///     assert!(inner.min.y >= outer.min.y && inner.max.y <= outer.max.y);
    /// }
    /// # Ok::<(), graphlay::LayoutError>(())
    /// ```
    pub parent: Option<NodeId>,
    /// A stable identity for the node, such as an activity name.
    ///
    /// When every node has a unique key, ties are broken by key and the drawing does not depend
    /// on input order. Otherwise ties go by input position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

/// One directed edge of the input graph.
///
/// `source == target` is a self-loop, drawn as a [`crate::layout::RouteKind::SelfLoop`] bump on
/// one border ([`crate::SelfLoopOptions`]).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct Edge {
    /// Where the edge starts.
    pub source: NodeId,
    /// Where the edge ends. Equal to `source` for a self-loop.
    pub target: NodeId,
    /// Layout importance, finite and `> 0`, default 1. Heavier edges are drawn shorter and
    /// straighter.
    pub weight: f64,
    /// Stroke width in px, default 2. Used only for port separation.
    pub thickness: f64,
    /// Box to reserve for a mid-edge label. Its centre comes back as
    /// [`crate::layout::EdgeRoute::label_anchor`].
    pub label_size: Option<Size>,
    /// Where the edge attaches to `source` (see [`PortHint`]), even if ranking reversed it.
    pub source_port: Option<PortHint>,
    /// Where the edge attaches to `target` (see [`PortHint`]), even if ranking reversed it.
    pub target_port: Option<PortHint>,
    /// Group of the edge (e.g., an object type). Used only as a bundling and lane tiebreak.
    pub group: Option<GroupId>,
    /// Lay this edge out as if it ran `target -> source`, but draw it `source -> target`.
    ///
    /// The target gets the lower rank, the route still runs source to target, and the edge is
    /// listed in [`crate::Layout::reversed`]. Use it for edges that point backwards, e.g., a retry.
    ///
    /// ```
    /// use graphlay::{layout, Edge, GraphBuilder, LayoutOptions, Size};
    ///
    /// let mut b = GraphBuilder::new();
    /// let refund = b.add_box(Size::new(100.0, 40.0));
    /// let order = b.add_box(Size::new(100.0, 40.0));
    /// // An arrow from refund to order, drawn as a step back.
    /// let cancels = b.add_edge(Edge { reverse_for_ranking: true, ..Edge::new(refund, order) });
    /// let l = layout(&b.build(), &LayoutOptions::layered_default())?;
    ///
    /// assert!(l.nodes[order.index()].center.x < l.nodes[refund.index()].center.x);
    /// assert_eq!(l.reversed, vec![cancels]);
    /// // The polyline still starts on refund's left border.
    /// let start = l.edges[0].points[0];
    /// assert!((start.x - (l.nodes[refund.index()].center.x - 50.0)).abs() < 0.5);
    /// # Ok::<(), graphlay::LayoutError>(())
    /// ```
    pub reverse_for_ranking: bool,
}

impl Default for Edge {
    /// A weight-1, 2px-wide edge from node 0 to node 0.
    fn default() -> Self {
        Edge {
            source: NodeId(0),
            target: NodeId(0),
            weight: 1.0,
            thickness: 2.0,
            label_size: None,
            source_port: None,
            target_port: None,
            group: None,
            reverse_for_ranking: false,
        }
    }
}

impl Edge {
    /// A plain edge between two nodes, with the default weight and thickness.
    pub fn new(source: NodeId, target: NodeId) -> Self {
        Edge {
            source,
            target,
            ..Edge::default()
        }
    }

    /// Whether both endpoints are the same node.
    pub fn is_self_loop(&self) -> bool {
        self.source == self.target
    }
}

/// The graph to lay out: nodes addressed by index, edges referring to those indices.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct Graph {
    /// Nodes, in [`NodeId`] order.
    pub nodes: Vec<Node>,
    /// Edges, in [`EdgeId`] order, which is also their order in a [`crate::Layout`].
    pub edges: Vec<Edge>,
}

impl Graph {
    /// An empty graph. Laying it out produces an empty [`crate::Layout`].
    pub fn new() -> Self {
        Graph::default()
    }

    /// Every node, paired with its id.
    pub fn nodes(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        self.nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (NodeId(i as u32), n))
    }

    /// Every edge, paired with its id.
    pub fn edges(&self) -> impl Iterator<Item = (EdgeId, &Edge)> {
        self.edges
            .iter()
            .enumerate()
            .map(|(i, e)| (EdgeId(i as u32), e))
    }

    /// The node with this id, or `None` if the id is out of range.
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.index())
    }

    /// The edge with this id, or `None` if the id is out of range.
    pub fn edge(&self, id: EdgeId) -> Option<&Edge> {
        self.edges.get(id.index())
    }

    /// Rejects input that has no layout. Every entry point runs this first.
    ///
    /// Checks endpoints, sizes, positions, weights, [`RankConstraint::Fixed`] layers,
    /// [`PortHint::Offset`] ranges and [`Node::parent`] cycles, and returns the first failure.
    pub fn validate(&self) -> Result<(), LayoutError> {
        let nodes = self.nodes.len() as u32;
        for (edge, e) in self.edges() {
            for node in [e.source, e.target] {
                if node.0 >= nodes {
                    return Err(LayoutError::EdgeEndpointOutOfRange {
                        edge,
                        node: node.0,
                        nodes,
                    });
                }
            }
        }
        for (node, nd) in self.nodes() {
            if !nd.size.width.is_finite() || !nd.size.height.is_finite() {
                return Err(LayoutError::NonFiniteSize { node });
            }
            if nd.size.width <= 0.0 || nd.size.height <= 0.0 {
                return Err(LayoutError::NonPositiveSize { node });
            }
        }
        for (node, nd) in self.nodes() {
            if let Some(p) = nd.position {
                if !p.is_finite() {
                    return Err(LayoutError::NonFinitePosition { node });
                }
            }
        }
        for (edge, e) in self.edges() {
            if !e.weight.is_finite() {
                return Err(LayoutError::NonFiniteWeight { edge });
            }
            if e.weight <= 0.0 {
                return Err(LayoutError::NonPositiveWeight { edge });
            }
            let fixed = |id: NodeId| match self.nodes[id.index()].rank {
                RankConstraint::Fixed(k) => Some(k),
                _ => None,
            };
            if e.source != e.target
                && fixed(e.source).is_some()
                && fixed(e.source) == fixed(e.target)
            {
                return Err(LayoutError::EdgeWithinFixedLayer { edge });
            }
        }
        for (edge, e) in self.edges() {
            for (source, hint) in [(true, e.source_port), (false, e.target_port)] {
                if let Some(t) = hint.and_then(PortHint::offset) {
                    if !t.is_finite() || t.abs() > 1.0 {
                        return Err(LayoutError::InvalidPortOffset { edge, source });
                    }
                }
            }
        }
        crate::compound::validate(self)
    }

    /// Checks that every node has a [`Node::position`], as [`crate::reroute`] requires.
    pub fn validate_positions(&self) -> Result<(), LayoutError> {
        for (node, nd) in self.nodes() {
            if nd.position.is_none() {
                return Err(LayoutError::MissingPosition { node });
            }
        }
        Ok(())
    }
}

/// Builder for a [`Graph`] that returns ids from each `add_*` call, so callers never count
/// indices.
///
/// ```
/// use graphlay::{Edge, GraphBuilder, Node, RankConstraint, Shape, Size};
///
/// let mut b = GraphBuilder::new();
/// let start = b.add_node(Node {
///     size: Size::new(40.0, 40.0),
///     shape: Shape::Ellipse,
///     rank: RankConstraint::First,
///     ..Node::default()
/// });
/// let review = b.add_box(Size::new(120.0, 50.0));
/// b.connect(start, review);
/// // A light, labelled self-loop with a 60x14 label box.
/// let rework = b.add_edge(Edge {
///     weight: 0.5,
///     label_size: Some(Size::new(60.0, 14.0)),
///     ..Edge::new(review, review)
/// });
/// // Keep 20px free below the node (the positive cross side in a left-to-right drawing).
/// b.node_mut(review).expect("just added").margin.cross_pos = 20.0;
///
/// let graph = b.build();
/// assert_eq!((graph.nodes.len(), graph.edges.len()), (2, 2));
/// assert!(graph.edges[rework.index()].is_self_loop());
/// assert_eq!(graph.validate(), Ok(()));
/// ```
#[derive(Clone, Debug, Default)]
pub struct GraphBuilder {
    graph: Graph,
}

impl GraphBuilder {
    /// A builder over an empty graph.
    pub fn new() -> Self {
        GraphBuilder::default()
    }

    /// A builder that keeps adding to an existing graph.
    pub fn from_graph(graph: Graph) -> Self {
        GraphBuilder { graph }
    }

    /// Appends `node` and returns its id.
    pub fn add_node(&mut self, node: Node) -> NodeId {
        let id = NodeId(self.graph.nodes.len() as u32);
        self.graph.nodes.push(node);
        id
    }

    /// Appends a plain node of `size` and returns its id.
    pub fn add_box(&mut self, size: Size) -> NodeId {
        self.add_node(Node {
            size,
            ..Node::default()
        })
    }

    /// Appends a default-weight edge from `source` to `target` and returns its id.
    pub fn connect(&mut self, source: NodeId, target: NodeId) -> EdgeId {
        self.add_edge(Edge::new(source, target))
    }

    /// Appends `edge` and returns its id.
    pub fn add_edge(&mut self, edge: Edge) -> EdgeId {
        let id = EdgeId(self.graph.edges.len() as u32);
        self.graph.edges.push(edge);
        id
    }

    /// Mutable access to an added node, or `None` if `id` is out of range.
    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.graph.nodes.get_mut(id.index())
    }

    /// Mutable access to an added edge, or `None` if `id` is out of range.
    pub fn edge_mut(&mut self, id: EdgeId) -> Option<&mut Edge> {
        self.graph.edges.get_mut(id.index())
    }

    /// The graph built so far.
    pub fn build(self) -> Graph {
        self.graph
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn two() -> Graph {
        let mut b = GraphBuilder::new();
        let a = b.add_box(Size::new(60.0, 40.0));
        let c = b.add_box(Size::new(60.0, 40.0));
        b.connect(a, c);
        b.build()
    }

    #[test]
    fn builder_hands_back_usable_ids() {
        let g = two();
        assert_eq!(g.nodes.len(), 2);
        assert_eq!(g.edges.len(), 1);
        assert_eq!(g.edges[0].source, NodeId(0));
        assert_eq!(g.edges[0].target, NodeId(1));
        assert_eq!(g.edges[0].weight, 1.0);
        assert_eq!(g.edges[0].thickness, 2.0);
        assert!(g.validate().is_ok());
    }

    #[test]
    fn a_self_loop_is_valid_input() {
        let mut g = two();
        g.edges.push(Edge::new(NodeId(1), NodeId(1)));
        assert!(g.edges[1].is_self_loop());
        assert!(g.validate().is_ok());
    }

    #[test]
    fn an_empty_graph_is_valid() {
        assert!(Graph::new().validate().is_ok());
    }

    #[test]
    fn validate_rejects_out_of_range_endpoints() {
        let mut g = two();
        g.edges.push(Edge::new(NodeId(1), NodeId(9)));
        assert_eq!(
            g.validate().unwrap_err(),
            LayoutError::EdgeEndpointOutOfRange {
                edge: EdgeId(1),
                node: 9,
                nodes: 2
            }
        );
    }

    #[test]
    fn validate_rejects_bad_sizes() {
        let mut g = two();
        g.nodes[1].size.height = f64::NAN;
        assert_eq!(
            g.validate().unwrap_err(),
            LayoutError::NonFiniteSize { node: NodeId(1) }
        );

        let mut g = two();
        g.nodes[0].size.width = 0.0;
        assert_eq!(
            g.validate().unwrap_err(),
            LayoutError::NonPositiveSize { node: NodeId(0) }
        );
    }

    #[test]
    fn validate_rejects_non_finite_positions() {
        let mut g = two();
        g.nodes[1].position = Some(Point::new(0.0, f64::INFINITY));
        assert_eq!(
            g.validate().unwrap_err(),
            LayoutError::NonFinitePosition { node: NodeId(1) }
        );
    }

    #[test]
    fn validate_rejects_bad_weights() {
        let mut g = two();
        g.edges[0].weight = f64::NAN;
        assert_eq!(
            g.validate().unwrap_err(),
            LayoutError::NonFiniteWeight { edge: EdgeId(0) }
        );
        g.edges[0].weight = 0.0;
        assert_eq!(
            g.validate().unwrap_err(),
            LayoutError::NonPositiveWeight { edge: EdgeId(0) }
        );
    }

    #[test]
    fn validate_rejects_bad_port_offsets() {
        let mut g = two();
        g.edges[0].source_port = Some(PortHint::Offset {
            side: Side::Top,
            t: 1.0,
        });
        g.edges[0].target_port = Some(PortHint::Side(Side::Bottom));
        assert!(g.validate().is_ok());
        for t in [f64::NAN, f64::INFINITY, 1.5, -1.01] {
            g.edges[0].target_port = Some(PortHint::Offset {
                side: Side::Left,
                t,
            });
            assert_eq!(
                g.validate().unwrap_err(),
                LayoutError::InvalidPortOffset {
                    edge: EdgeId(0),
                    source: false
                }
            );
        }
    }

    #[test]
    fn validate_checks_parents() {
        let mut g = two();
        g.nodes[1].parent = Some(NodeId(0));
        assert_eq!(g.validate(), Ok(()));
        g.nodes[0].parent = Some(NodeId(1));
        assert_eq!(
            g.validate().unwrap_err(),
            LayoutError::InvalidParent { node: NodeId(0) }
        );
        g.nodes[0].parent = Some(NodeId(0));
        assert_eq!(
            g.validate().unwrap_err(),
            LayoutError::InvalidParent { node: NodeId(0) }
        );
    }

    #[test]
    fn validate_positions_wants_one_per_node() {
        let mut g = two();
        assert_eq!(
            g.validate_positions().unwrap_err(),
            LayoutError::MissingPosition { node: NodeId(0) }
        );
        for nd in &mut g.nodes {
            nd.position = Some(Point::new(0.0, 0.0));
        }
        assert!(g.validate_positions().is_ok());
    }
}

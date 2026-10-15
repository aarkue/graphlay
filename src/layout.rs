//! The output model returned by [`crate::layout()`], [`crate::reroute`] and
//! [`crate::tree::layout`].
//!
//! A [`Layout`] is indexed like the [`crate::Graph`] that produced it. `nodes[i]` is where node
//! `i` was placed and `edges[i]` is how edge `i` was routed.
//!
//! Every polyline runs from source to target, border to border. An edge the cycle breaker
//! reversed is turned back before it is emitted, and its id is listed in [`Layout::reversed`].

use serde::{Deserialize, Serialize};

use crate::geometry::{Point, Rect, Side, Size};
use crate::graph::EdgeId;

/// Where one node ended up.
#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Placement {
    /// Centre of the node's box, in output space.
    pub center: Point,
    /// The node's size, echoed from the input so a renderer needs only the layout.
    pub size: Size,
    /// Layer index. `None` for an edgeless node packed into the grid below the layered drawing.
    pub rank: Option<i32>,
    /// Position within the layer. Routing waypoints count too, so two nodes with a long edge's
    /// track between them are two apart.
    pub order: u32,
}

/// How an edge attaches to one of its nodes.
#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PortInfo {
    /// The point on the node's border the polyline starts (or ends) at.
    pub point: Point,
    /// Which border that is, in output space. For an end placed by a port hint
    /// ([`crate::Edge::source_port`] / [`crate::Edge::target_port`]), the hinted border.
    pub side: Side,
    /// Signed distance in px along the border from its middle to [`PortInfo::point`]. Positive
    /// is down on the left and right borders and right on the top and bottom borders.
    pub offset: f64,
}

impl PortInfo {
    /// The port at `point` on the `side` border of the node placed at `at`.
    pub fn on(point: Point, side: Side, at: &Placement) -> PortInfo {
        let offset = match side {
            Side::Left | Side::Right => point.y - at.center.y,
            Side::Top | Side::Bottom => point.x - at.center.x,
        };
        PortInfo {
            point,
            side,
            offset,
        }
    }
}

/// How an edge was routed.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum RouteKind {
    /// A route between two adjacent layers, with no routing waypoints.
    #[default]
    Direct,
    /// A long edge, routed through a chain of waypoints.
    Chained {
        /// How many layer gaps it crosses (always `> 1`).
        spans: u32,
    },
    /// An edge from a node to itself, drawn as a rounded bump out of one of its borders
    /// ([`crate::SelfLoopOptions`]). Loops on one node nest by edge order.
    SelfLoop,
    /// One lane of a bundle: several edges sharing a source and a target, routed once as a single
    /// corridor and then spread into parallel lanes.
    Bundled {
        /// Index of this edge's lane within its bundle, in edge order.
        lane: u32,
    },
    /// No polyline of at least two points. The layered pipeline does not return it.
    Unrouted,
}

/// How one edge was drawn.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct EdgeRoute {
    /// Border-to-border polyline, source first. At least two points for a routed edge, four for
    /// a [`RouteKind::SelfLoop`] and none for [`RouteKind::Unrouted`].
    pub points: Vec<Point>,
    /// Where the polyline leaves the source node, or `None` when there is no polyline.
    pub source_port: Option<PortInfo>,
    /// Where the polyline enters the target node, or `None` when there is no polyline.
    pub target_port: Option<PortInfo>,
    /// What kind of route this is.
    pub kind: RouteKind,
    /// Where to centre this edge's label, when [`crate::Edge::label_size`] is set.
    pub label_anchor: Option<Point>,
}

/// Counters from the run that produced a [`Layout`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct LayoutStats {
    /// Routing waypoints (dummy nodes) the long edges were cut into.
    pub dummy_count: u32,
}

/// One laid-out drawing.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Layout {
    /// One entry per input node, in [`crate::NodeId`] order.
    pub nodes: Vec<Placement>,
    /// One entry per input edge, in [`crate::EdgeId`] order.
    pub edges: Vec<EdgeRoute>,
    /// Bounding box of every node box, polyline point and label anchor, without padding.
    pub bounds: Rect,
    /// Number of layers in a layered drawing, or of depth rows in a tidy tree. `0` for an empty
    /// graph.
    pub layer_count: u32,
    /// Edges the layering saw the other way round: Those the cycle breaker reversed, plus those
    /// with [`crate::Edge::reverse_for_ranking`] (an edge that is both is not listed). Their
    /// polylines still run source to target.
    pub reversed: Vec<EdgeId>,
    /// Counters about the run.
    pub stats: LayoutStats,
    /// Where a merged edge branches off a shared trunk ([`crate::Routing::merge_edges`]), for a
    /// renderer to mark with a dot. Empty unless edges were merged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub junctions: Vec<Point>,
    /// The swimlanes of a drawing with [`crate::GroupLanes::Strict`] lanes, in order along the
    /// order axis. Empty otherwise.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lanes: Vec<Lane>,
}

/// One swimlane: a strip across the whole drawing that holds one group's nodes.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Lane {
    /// The group the lane holds, or `None` for the lane of nodes without a group.
    pub group: Option<crate::graph::GroupId>,
    /// The lane's rectangle. Adjacent lanes share their border, halfway across the gap between
    /// their nodes, and every lane spans the drawing's full extent along the layer axis.
    pub rect: Rect,
}

impl Layout {
    /// The placement of node `id`, or `None` if the id is out of range.
    pub fn placement(&self, id: crate::graph::NodeId) -> Option<&Placement> {
        self.nodes.get(id.index())
    }

    /// The route of edge `id`, or `None` if the id is out of range.
    pub fn route(&self, id: EdgeId) -> Option<&EdgeRoute> {
        self.edges.get(id.index())
    }
}

/// Which border of the box `size` centred on `center` the point `p` sits on.
///
/// Uses the larger normalised offset, which also gives the nearest border for an ellipse. A corner
/// resolves to the left or right side.
pub(crate) fn side_at(p: Point, center: Point, size: Size) -> Side {
    let (hw, hh) = (size.width / 2.0, size.height / 2.0);
    let (dx, dy) = (p.x - center.x, p.y - center.y);
    let nx = if hw > 0.0 { (dx / hw).abs() } else { 0.0 };
    let ny = if hh > 0.0 { (dy / hh).abs() } else { 0.0 };
    if nx >= ny {
        if dx >= 0.0 {
            Side::Right
        } else {
            Side::Left
        }
    } else if dy >= 0.0 {
        Side::Bottom
    } else {
        Side::Top
    }
}

impl EdgeRoute {
    /// Builds a routed edge from its polyline, deriving both ports from where it meets its nodes,
    /// e.g., to feed another engine's geometry to the metrics or the renderer. Fewer than two
    /// points give [`RouteKind::Unrouted`].
    pub fn routed(
        points: Vec<Point>,
        kind: RouteKind,
        source: &Placement,
        target: &Placement,
        label_anchor: Option<Point>,
    ) -> EdgeRoute {
        let port = |p: Point, at: &Placement| PortInfo::on(p, side_at(p, at.center, at.size), at);
        match (points.first().copied(), points.last().copied()) {
            (Some(a), Some(b)) if points.len() >= 2 => EdgeRoute {
                source_port: Some(port(a, source)),
                target_port: Some(port(b, target)),
                points,
                kind,
                label_anchor,
            },
            _ => EdgeRoute {
                points: Vec::new(),
                source_port: None,
                target_port: None,
                kind: RouteKind::Unrouted,
                label_anchor,
            },
        }
    }
}

impl Layout {
    /// Sets [`Layout::bounds`] to the box holding every node, polyline point and label anchor.
    /// An empty drawing gets the degenerate rectangle at the origin.
    pub fn recompute_bounds(&mut self) {
        let mut acc: Option<Rect> = None;
        let mut add = |r: Rect| {
            acc = Some(match acc {
                Some(a) => a.union(r),
                None => r,
            });
        };
        for placement in &self.nodes {
            add(Rect::from_center(placement.center, placement.size));
        }
        for route in &self.edges {
            for &p in &route.points {
                add(Rect::from_point(p));
            }
            if let Some(p) = route.label_anchor {
                add(Rect::from_point(p));
            }
        }
        self.bounds = acc.unwrap_or_default();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::geometry::{Point, Size};

    fn placement(cx: f64, cy: f64) -> Placement {
        Placement {
            center: Point::new(cx, cy),
            size: Size::new(100.0, 50.0),
            rank: Some(0),
            order: 0,
        }
    }

    #[test]
    fn side_at_names_the_border_the_point_is_on() {
        let (c, s) = (Point::new(0.0, 0.0), Size::new(100.0, 50.0));
        assert_eq!(side_at(Point::new(50.0, 0.0), c, s), Side::Right);
        assert_eq!(side_at(Point::new(-50.0, 10.0), c, s), Side::Left);
        assert_eq!(side_at(Point::new(10.0, -25.0), c, s), Side::Top);
        assert_eq!(side_at(Point::new(-10.0, 25.0), c, s), Side::Bottom);
        assert_eq!(side_at(Point::new(40.0, -25.0), c, s), Side::Top);
    }

    #[test]
    fn routed_derives_ports_and_demotes_an_empty_polyline() {
        let (a, b) = (placement(0.0, 0.0), placement(0.0, 200.0));
        let r = EdgeRoute::routed(
            vec![Point::new(0.0, 25.0), Point::new(0.0, 175.0)],
            RouteKind::Direct,
            &a,
            &b,
            None,
        );
        assert_eq!(r.source_port.map(|p| p.side), Some(Side::Bottom));
        assert_eq!(r.target_port.map(|p| p.side), Some(Side::Top));
        assert_eq!(r.kind, RouteKind::Direct);

        let empty = EdgeRoute::routed(vec![], RouteKind::Direct, &a, &b, None);
        assert_eq!(empty.kind, RouteKind::Unrouted);
        assert!(empty.source_port.is_none() && empty.target_port.is_none());
    }

    #[test]
    fn bounds_cover_nodes_routes_and_labels() {
        let mut l = Layout {
            nodes: vec![placement(0.0, 0.0)],
            ..Layout::default()
        };
        l.edges.push(EdgeRoute {
            points: vec![Point::new(0.0, 25.0), Point::new(0.0, 300.0)],
            label_anchor: Some(Point::new(-400.0, 10.0)),
            ..EdgeRoute::default()
        });
        l.recompute_bounds();
        assert_eq!(l.bounds.min, Point::new(-400.0, -25.0));
        assert_eq!(l.bounds.max, Point::new(50.0, 300.0));

        let mut empty = Layout::default();
        empty.recompute_bounds();
        assert_eq!(empty.bounds, Rect::default());
    }

    #[test]
    fn total_cmp_matches_partial_cmp_without_nan() {
        let values = [
            -1e300,
            -2.5,
            -1.0,
            0.0,
            1e-9,
            1.0,
            2.5,
            1e300,
            f64::INFINITY,
        ];
        for &a in &values {
            for &b in &values {
                assert_eq!(a.total_cmp(&b), a.partial_cmp(&b).unwrap(), "{a} vs {b}");
            }
        }
    }
}

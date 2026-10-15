//! Self-loops: the room reserved for them, where they attach, and what they look like.
//!
//! A self-loop never reaches cycle breaking, layering or ordering, so the checks are geometric:
//! the bump is outside the node, its ports keep clear of other edges' ports, `k` loops nest, and
//! the label anchor is somewhere a label can be drawn.

use graphlay::{
    layout, Edge, EdgeRoute, Graph, GraphBuilder, Layout, LayoutOptions, LoopSide, Node, NodeId,
    Point, RouteKind, Shape, Size,
};

fn boxes(n: usize, size: Size) -> GraphBuilder {
    let mut b = GraphBuilder::new();
    for _ in 0..n {
        b.add_node(Node {
            size,
            ..Node::default()
        });
    }
    b
}

fn loops_of(l: &Layout) -> Vec<&EdgeRoute> {
    l.edges
        .iter()
        .filter(|e| e.kind == RouteKind::SelfLoop)
        .collect()
}

/// How far outside the box centred on `p` the point sits: positive when it is clear of the border.
fn outside_by(p: Point, center: Point, size: Size) -> f64 {
    ((p.x - center.x).abs() - size.width / 2.0).max((p.y - center.y).abs() - size.height / 2.0)
}

/// Every loop comes back as a routed `SelfLoop`: a bump that starts and ends on the node's border
/// and reaches outside it in between.
#[test]
fn a_self_loop_is_routed_as_a_bump_outside_its_node() {
    let size = Size::new(120.0, 52.0);
    let mut b = boxes(2, size);
    let (a, c) = (NodeId(0), NodeId(1));
    b.connect(a, c);
    b.connect(c, c);
    let g = b.build();
    let l = layout(&g, &LayoutOptions::layered_default()).unwrap();

    let route = &l.edges[1];
    assert_eq!(route.kind, RouteKind::SelfLoop);
    assert_eq!(route.points.len(), 4, "a bump is port, out, across, port");
    assert!(route.source_port.is_some() && route.target_port.is_some());

    let at = l.nodes[c.index()];
    let ends = [route.points[0], route.points[3]];
    for p in ends {
        assert!(
            outside_by(p, at.center, at.size).abs() < 0.5,
            "{p:?} is not on the border of {:?}",
            at.center
        );
    }
    for p in [route.points[1], route.points[2]] {
        assert!(
            outside_by(p, at.center, at.size) > 0.0,
            "{p:?} is not outside the node"
        );
    }
    // The bump is inside the drawing's bounds.
    for p in &route.points {
        assert!(l.bounds.min.x <= p.x && p.x <= l.bounds.max.x, "{p:?}");
        assert!(l.bounds.min.y <= p.y && p.y <= l.bounds.max.y, "{p:?}");
    }
}

/// `k` loops on one node nest: each is further out than the last and no two share a point.
#[test]
fn stacked_loops_nest_without_touching() {
    let size = Size::new(160.0, 60.0);
    let mut b = boxes(2, size);
    let (a, c) = (NodeId(0), NodeId(1));
    b.connect(a, c);
    for _ in 0..4 {
        b.connect(c, c);
    }
    let g = b.build();
    let l = layout(&g, &LayoutOptions::layered_default()).unwrap();

    let at = l.nodes[c.index()];
    let loops = loops_of(&l);
    assert_eq!(loops.len(), 4);
    let mut previous: Option<f64> = None;
    for (i, route) in loops.iter().enumerate() {
        assert_eq!(route.points.len(), 4, "loop {i}");
        let depth = route
            .points
            .iter()
            .map(|&p| outside_by(p, at.center, at.size))
            .fold(f64::NEG_INFINITY, f64::max);
        if let Some(inner) = previous {
            assert!(depth > inner, "loop {i} is not outside loop {}", i - 1);
        }
        previous = Some(depth);
    }
    // Half a pixel is the tolerance the collinear-overlap metric calls "the same line".
    for (i, one) in loops.iter().enumerate() {
        for two in loops.iter().skip(i + 1) {
            for p in &one.points {
                for q in &two.points {
                    assert!(
                        (p.x - q.x).abs() > 0.5 || (p.y - q.y).abs() > 0.5,
                        "{p:?} and {q:?} coincide"
                    );
                }
            }
        }
    }
}

/// A loop and the ordinary edges on the same border keep the minimum port gap. `AlongPositive`
/// puts the loop on the forward face, where every outgoing arc of the fan attaches.
#[test]
fn a_loop_and_an_arc_on_one_border_keep_the_port_gap() {
    // A tall hub, so its forward border is busy but has room to keep the gap.
    let size = Size::new(200.0, 200.0);
    let mut b = boxes(6, size);
    let hub = NodeId(0);
    for i in 1..5 {
        b.connect(hub, NodeId(i));
    }
    b.connect(NodeId(5), hub);
    b.connect(hub, hub);
    let g = b.build();
    let mut o = LayoutOptions::layered_default();
    o.self_loops.side = LoopSide::AlongPositive;
    let l = layout(&g, &o).unwrap();

    let loop_route = loops_of(&l)[0];
    let loop_ends = [loop_route.points[0], loop_route.points[3]];

    let mut checked = 0;
    for (id, e) in g.edges() {
        if e.is_self_loop() {
            continue;
        }
        let route = &l.edges[id.index()];
        for (node, p) in [
            (e.source, route.points[0]),
            (e.target, route.points[route.points.len() - 1]),
        ] {
            if node != hub {
                continue;
            }
            for end in loop_ends {
                let d = (p.x - end.x).hypot(p.y - end.y);
                assert!(
                    d >= o.spacing.port_gap - 1e-6,
                    "edge {} attaches {d:.2}px from a loop port (port_gap is {})",
                    id.0,
                    o.spacing.port_gap
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "the hub should carry ordinary ports too");
}

/// The label anchor lies outside the node and past its bump.
#[test]
fn a_loop_label_is_anchored_outside_the_node() {
    let size = Size::new(120.0, 52.0);
    let mut b = boxes(2, size);
    let (a, c) = (NodeId(0), NodeId(1));
    b.connect(a, c);
    b.add_edge(Edge {
        source: c,
        target: c,
        label_size: Some(Size::new(60.0, 16.0)),
        ..Edge::default()
    });
    let g = b.build();
    let l = layout(&g, &LayoutOptions::layered_default()).unwrap();

    let at = l.nodes[c.index()];
    let route = &l.edges[1];
    let anchor = route.label_anchor.expect("a labelled loop reserves a slot");
    assert!(
        outside_by(anchor, at.center, at.size) > 0.0,
        "{anchor:?} is inside the node"
    );
    let outer = route
        .points
        .iter()
        .map(|&p| outside_by(p, at.center, at.size))
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(
        outside_by(anchor, at.center, at.size) > outer,
        "the label sits on the bump rather than beyond it"
    );
    assert!(l.bounds.min.x <= anchor.x && anchor.x <= l.bounds.max.x);
    assert!(l.bounds.min.y <= anchor.y && anchor.y <= l.bounds.max.y);
}

/// On an ellipse the loop leaves and re-enters the curve, not the bounding box.
#[test]
fn a_loop_on_an_ellipse_starts_and_ends_on_the_outline() {
    let mut b = GraphBuilder::new();
    let a = b.add_node(Node {
        size: Size::new(80.0, 80.0),
        shape: Shape::Ellipse,
        ..Node::default()
    });
    let c = b.add_node(Node {
        size: Size::new(80.0, 80.0),
        shape: Shape::Ellipse,
        ..Node::default()
    });
    b.connect(a, c);
    b.connect(c, c);
    let g = b.build();
    let l = layout(&g, &LayoutOptions::layered_default()).unwrap();

    let at = l.nodes[c.index()];
    let route = &l.edges[1];
    for p in [route.points[0], route.points[3]] {
        let r = ((p.x - at.center.x) / (at.size.width / 2.0))
            .hypot((p.y - at.center.y) / (at.size.height / 2.0));
        assert!(
            (r - 1.0).abs() < 1e-6,
            "{p:?} is not on the ellipse (r = {r})"
        );
    }
}

/// With the loops on the positive order side, the neighbour on that side is pushed clear of them.
#[test]
fn the_reservation_keeps_a_neighbour_clear_of_the_bump() {
    let size = Size::new(120.0, 52.0);
    let mut b = boxes(4, size);
    let root = NodeId(0);
    for i in 1..4 {
        b.connect(root, NodeId(i));
    }
    for _ in 0..3 {
        b.connect(NodeId(1), NodeId(1));
    }
    let g = b.build();
    let mut o = LayoutOptions::layered_default();
    o.self_loops.side = LoopSide::CrossPositive;
    let l = layout(&g, &o).unwrap();

    for route in loops_of(&l) {
        for w in route.points.windows(2) {
            for (id, nd) in g.nodes() {
                if id == NodeId(1) {
                    continue;
                }
                let at = l.nodes[id.index()];
                let (hw, hh) = (nd.size.width / 2.0, nd.size.height / 2.0);
                let inside = |p: Point| {
                    (p.x - at.center.x).abs() < hw - 0.5 && (p.y - at.center.y).abs() < hh - 0.5
                };
                assert!(
                    !inside(w[0]) && !inside(w[1]),
                    "a loop runs into node {}",
                    id.0
                );
            }
        }
    }
}

/// An edgeless node, placed by the orphan grid, still gets its loops drawn.
#[test]
fn an_edgeless_node_still_gets_its_loops() {
    let size = Size::new(120.0, 52.0);
    let mut b = boxes(3, size);
    b.connect(NodeId(0), NodeId(1));
    b.connect(NodeId(2), NodeId(2));
    b.connect(NodeId(2), NodeId(2));
    let g = b.build();
    let l = layout(&g, &LayoutOptions::layered_default()).unwrap();

    let at = l.nodes[2];
    assert_eq!(at.rank, None, "node 2 has no edges, so no rank");
    let loops = loops_of(&l);
    assert_eq!(loops.len(), 2);
    for route in loops {
        assert_eq!(route.points.len(), 4);
        for p in [route.points[0], route.points[3]] {
            assert!(outside_by(p, at.center, at.size).abs() < 0.5, "{p:?}");
        }
    }
}

/// A reroute re-emits the loops from the seeded positions, like every other edge.
#[test]
fn a_reroute_re_emits_the_loops() {
    let size = Size::new(120.0, 52.0);
    let mut b = boxes(2, size);
    b.connect(NodeId(0), NodeId(1));
    b.connect(NodeId(1), NodeId(1));
    let mut g = b.build();
    let o = LayoutOptions::layered_default();
    let fresh = layout(&g, &o).unwrap();
    for (i, nd) in g.nodes.iter_mut().enumerate() {
        nd.position = Some(fresh.nodes[i].center);
    }
    let again = graphlay::reroute(&g, &o).unwrap();

    let at = again.nodes[1];
    let route = &again.edges[1];
    assert_eq!(route.kind, RouteKind::SelfLoop);
    assert_eq!(route.points.len(), 4);
    for p in [route.points[0], route.points[3]] {
        assert!(outside_by(p, at.center, at.size).abs() < 0.5, "{p:?}");
    }
}

/// Each fixed side option puts the bump on the border it names, in output space, for both layout
/// directions.
#[test]
fn the_loop_side_option_picks_the_border() {
    let size = Size::new(120.0, 52.0);
    for direction in [
        graphlay::Direction::LeftRight,
        graphlay::Direction::TopBottom,
    ] {
        for (side, expect) in [
            (LoopSide::CrossPositive, (0.0, 1.0)),
            (LoopSide::CrossNegative, (0.0, -1.0)),
            (LoopSide::AlongPositive, (1.0, 0.0)),
        ] {
            let mut b = boxes(2, size);
            b.connect(NodeId(0), NodeId(1));
            b.connect(NodeId(1), NodeId(1));
            let g = b.build();
            let mut o = LayoutOptions::layered_default();
            o.direction = direction;
            o.self_loops.side = side;
            let l = layout(&g, &o).unwrap();

            // The flow-space direction the bump reaches in, transposed the way the drawing is.
            let want = match direction {
                graphlay::Direction::LeftRight => expect,
                graphlay::Direction::TopBottom => (expect.1, expect.0),
            };
            let at = l.nodes[1];
            let outer = l.edges[1].points[1];
            let (dx, dy) = (outer.x - at.center.x, outer.y - at.center.y);
            assert!(
                dx * want.0 + dy * want.1 > 0.0,
                "{side:?} in {direction:?}: bump at {outer:?} for a node at {:?}",
                at.center
            );
        }
    }
}

/// An empty graph and a graph that is nothing but loops both lay out.
#[test]
fn a_graph_of_nothing_but_loops_lays_out() {
    let mut b = boxes(2, Size::new(90.0, 40.0));
    b.connect(NodeId(0), NodeId(0));
    b.connect(NodeId(1), NodeId(1));
    let g = b.build();
    let l = layout(&g, &LayoutOptions::layered_default()).unwrap();
    assert_eq!(loops_of(&l).len(), 2);
    for route in loops_of(&l) {
        assert_eq!(route.points.len(), 4);
    }
    assert!(Graph::new()
        .validate()
        .and_then(|()| layout(&Graph::new(), &LayoutOptions::layered_default()).map(|_| ()))
        .is_ok());
}

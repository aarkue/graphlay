//! Compound (nested) layout: [`Node::parent`].

use graphlay::{
    layout, reroute, Direction, Edge, Graph, GraphBuilder, Layout, LayoutError, LayoutOptions,
    Node, NodeId, Point, Size,
};

const EPS: f64 = 0.75;

fn node(parent: Option<u32>) -> Node {
    Node {
        size: Size::new(60.0, 30.0),
        parent: parent.map(NodeId),
        ..Node::default()
    }
}

fn graph(nodes: &[Option<u32>], edges: &[(u32, u32)]) -> Graph {
    let mut b = GraphBuilder::new();
    for &p in nodes {
        b.add_node(node(p));
    }
    for &(s, t) in edges {
        b.add_edge(Edge::new(NodeId(s), NodeId(t)));
    }
    b.build()
}

fn inside(l: &Layout, child: usize, compound: usize) -> bool {
    let (c, k) = (&l.nodes[child], &l.nodes[compound]);
    c.center.x - c.size.width / 2.0 >= k.center.x - k.size.width / 2.0 - EPS
        && c.center.x + c.size.width / 2.0 <= k.center.x + k.size.width / 2.0 + EPS
        && c.center.y - c.size.height / 2.0 >= k.center.y - k.size.height / 2.0 - EPS
        && c.center.y + c.size.height / 2.0 <= k.center.y + k.size.height / 2.0 + EPS
}

fn on_border(p: Point, l: &Layout, node: usize) -> bool {
    let k = &l.nodes[node];
    let (hw, hh) = (k.size.width / 2.0, k.size.height / 2.0);
    let (dx, dy) = ((p.x - k.center.x).abs(), (p.y - k.center.y).abs());
    ((dx - hw).abs() < EPS && dy <= hh + EPS) || ((dy - hh).abs() < EPS && dx <= hw + EPS)
}

/// Every route is one connected polyline from the source's border to the target's border,
/// orthogonal throughout.
fn check_routes(g: &Graph, l: &Layout) {
    for (i, (e, r)) in g.edges.iter().zip(&l.edges).enumerate() {
        assert!(r.points.len() >= 2, "edge {i}: no route");
        let (a, z) = (r.points[0], *r.points.last().expect("points"));
        assert!(
            on_border(a, l, e.source.index()),
            "edge {i}: starts at {a:?}, off its source"
        );
        assert!(
            on_border(z, l, e.target.index()),
            "edge {i}: ends at {z:?}, off its target"
        );
        for w in r.points.windows(2) {
            let straight = (w[0].x - w[1].x).abs() < EPS || (w[0].y - w[1].y).abs() < EPS;
            assert!(
                straight,
                "edge {i}: diagonal segment {:?} -> {:?}",
                w[0], w[1]
            );
        }
        // A route may cross a compound's border but never run along it.
        for k in 0..g.nodes.len() {
            if !g.nodes.iter().any(|c| c.parent == Some(NodeId(k as u32))) {
                continue;
            }
            let b = &l.nodes[k];
            let (x0, x1) = (
                b.center.x - b.size.width / 2.0,
                b.center.x + b.size.width / 2.0,
            );
            let (y0, y1) = (
                b.center.y - b.size.height / 2.0,
                b.center.y + b.size.height / 2.0,
            );
            for w in r.points.windows(2) {
                let len = (w[0].x - w[1].x).abs() + (w[0].y - w[1].y).abs();
                let overlap =
                    |a: f64, b: f64, lo: f64, hi: f64| a.max(b) > lo + EPS && a.min(b) < hi - EPS;
                let on_x = [x0, x1].iter().any(|&x| {
                    (w[0].x - x).abs() < EPS
                        && (w[1].x - x).abs() < EPS
                        && overlap(w[0].y, w[1].y, y0, y1)
                });
                let on_y = [y0, y1].iter().any(|&y| {
                    (w[0].y - y).abs() < EPS
                        && (w[1].y - y).abs() < EPS
                        && overlap(w[0].x, w[1].x, x0, x1)
                });
                assert!(
                    len < 1.0 || !(on_x || on_y),
                    "edge {i}: runs along compound {k}'s border: {:?} -> {:?}",
                    w[0],
                    w[1]
                );
            }
        }
    }
}

#[test]
fn children_are_drawn_inside_their_compound() {
    // 0 -> [1: 2 -> 3] -> 4, with edges into and out of the compound's children.
    let g = graph(
        &[None, None, Some(1), Some(1), None],
        &[(0, 2), (2, 3), (3, 4)],
    );
    for direction in [Direction::LeftRight, Direction::TopBottom] {
        let o = LayoutOptions {
            direction,
            ..LayoutOptions::default()
        };
        let l = layout(&g, &o).expect("lays out");
        assert!(inside(&l, 2, 1) && inside(&l, 3, 1), "{direction:?}");
        // The compound grew to hold its children.
        assert!(l.nodes[1].size.width > 60.0 && l.nodes[1].size.height > 30.0);
        assert!(!inside(&l, 0, 1) && !inside(&l, 4, 1));
        check_routes(&g, &l);
    }
}

#[test]
fn compounds_nest() {
    // 0 -> [1: [2: 3 -> 4] -> 5] -> 6
    let g = graph(
        &[None, None, Some(1), Some(2), Some(2), Some(1), None],
        &[(0, 3), (3, 4), (4, 5), (5, 6), (0, 6)],
    );
    let l = layout(&g, &LayoutOptions::default()).expect("lays out");
    assert!(inside(&l, 2, 1) && inside(&l, 3, 2) && inside(&l, 4, 2) && inside(&l, 5, 1));
    assert!(inside(&l, 3, 1) && !inside(&l, 5, 2));
    check_routes(&g, &l);
}

#[test]
fn a_compound_can_connect_to_its_own_children() {
    // [0: 1 -> 2], with 0 -> 1 and 2 -> 0.
    let g = graph(&[None, Some(0), Some(0)], &[(0, 1), (1, 2), (2, 0)]);
    let l = layout(&g, &LayoutOptions::default()).expect("lays out");
    assert!(inside(&l, 1, 0) && inside(&l, 2, 0));
    check_routes(&g, &l);
}

#[test]
fn compounds_are_laid_out_as_nodes_of_their_level() {
    // Two compounds in a row, each with an internal chain, linked child to child.
    let g = graph(
        &[None, Some(0), Some(0), None, Some(3), Some(3)],
        &[(1, 2), (4, 5), (2, 4)],
    );
    let l = layout(&g, &LayoutOptions::default()).expect("lays out");
    let (a, b) = (&l.nodes[0], &l.nodes[3]);
    let apart = a.center.x + a.size.width / 2.0 <= b.center.x - b.size.width / 2.0 + EPS
        || b.center.x + b.size.width / 2.0 <= a.center.x - a.size.width / 2.0 + EPS;
    assert!(apart, "{a:?} {b:?}");
    check_routes(&g, &l);
}

#[test]
fn invalid_parents_are_rejected() {
    let g = graph(&[Some(0)], &[]);
    assert!(matches!(
        layout(&g, &LayoutOptions::default()),
        Err(LayoutError::InvalidParent { .. })
    ));
    let g = graph(&[Some(1), Some(0)], &[]);
    assert!(matches!(
        layout(&g, &LayoutOptions::default()),
        Err(LayoutError::InvalidParent { .. })
    ));
    let g = graph(&[Some(7)], &[]);
    assert!(matches!(
        layout(&g, &LayoutOptions::default()),
        Err(LayoutError::InvalidParent { .. })
    ));
}

#[test]
fn reroute_takes_flat_graphs_only() {
    let mut g = graph(&[None, Some(0)], &[]);
    for nd in &mut g.nodes {
        nd.position = Some(Point::new(0.0, 0.0));
    }
    assert!(matches!(
        reroute(&g, &LayoutOptions::default()),
        Err(LayoutError::NestedNodesUnsupported { .. })
    ));
}

#[test]
fn a_header_adds_room_along_the_top() {
    let g = graph(&[None, Some(0), Some(0)], &[(1, 2)]);
    let mut o = LayoutOptions::default();
    let plain = layout(&g, &o).expect("lays out");
    o.spacing.compound_header = 24.0;
    let titled = layout(&g, &o).expect("lays out");
    let room_above = |l: &Layout| {
        let k = &l.nodes[0];
        let top = k.center.y - k.size.height / 2.0;
        (1..3)
            .map(|i| l.nodes[i].center.y - l.nodes[i].size.height / 2.0 - top)
            .fold(f64::INFINITY, f64::min)
    };
    assert!((room_above(&titled) - room_above(&plain) - 24.0).abs() < EPS);
    assert!(inside(&titled, 1, 0) && inside(&titled, 2, 0));
}

/// Random forests of compounds with random edges, in both directions: every layout succeeds,
/// every child lies inside its parent, and every route is one orthogonal polyline from border to
/// border that never runs along a compound's border.
#[test]
fn random_compound_graphs_hold_together() {
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = |m: u64| {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        (seed >> 33) % m
    };
    for case in 0..300 {
        let n = 3 + next(12) as u32;
        // A parent always has a smaller index, so the parents form a forest.
        let parents: Vec<Option<u32>> = (0..n)
            .map(|i| (i > 0 && next(3) > 0).then(|| next(u64::from(i)) as u32))
            .collect();
        let edges: Vec<(u32, u32)> = (0..next(2 * u64::from(n)) + 1)
            .map(|_| (next(u64::from(n)) as u32, next(u64::from(n)) as u32))
            .collect();
        let g = graph(&parents, &edges);
        let direction = if case % 2 == 0 {
            Direction::LeftRight
        } else {
            Direction::TopBottom
        };
        let o = LayoutOptions {
            direction,
            ..LayoutOptions::default()
        };
        let l = layout(&g, &o).unwrap_or_else(|e| panic!("case {case}: {e}"));
        for (i, p) in parents.iter().enumerate() {
            if let Some(p) = p {
                assert!(inside(&l, i, *p as usize), "case {case}: {i} outside {p}");
            }
        }
        check_routes(&g, &l);
        for (i, (e, r)) in g.edges.iter().zip(&l.edges).enumerate() {
            if e.source != e.target {
                assert!(r.points.len() >= 2, "case {case}: edge {i} unrouted");
            }
        }
    }
}

/// A child sits inside its compound and an edge into it crosses the compound's border. The
/// metrics count neither as a defect.
#[test]
fn metrics_see_no_defects_in_a_compound_drawing() {
    let g = graph(&[None, Some(0), Some(0), None], &[(3, 1), (1, 2)]);
    let o = LayoutOptions::default();
    let l = layout(&g, &o).expect("lays out");
    let m = graphlay::metrics::measure(&g, &o, &l);
    assert_eq!(
        (m.node_overlaps, m.node_edge_hits, m.overlaps),
        (0, 0, 0),
        "{m:?}"
    );
}

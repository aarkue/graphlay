//! Seeded relayout, pinning, reroute, flow spines and lanes through the public API: the calls an
//! interactive editor makes after the first layout.

use graphlay::{
    layout, reroute, Direction, Edge, Graph, GraphBuilder, GroupId, Layout, LayoutOptions, Node,
    NodeId, Point, PortSides, RoutingStyle, SeedMode, Shape, Size,
};

// ---------------------------------------------------------------------------------------------
// Graph and options helpers
// ---------------------------------------------------------------------------------------------

/// A plain node of `width` x `height` with the given border shape and nothing else set.
fn node(width: f64, height: f64, shape: Shape) -> Node {
    Node {
        size: Size::new(width, height),
        shape,
        ..Node::default()
    }
}

/// A Petri-net place: a 52px circle.
fn place() -> Node {
    node(52.0, 52.0, Shape::Ellipse)
}

/// A Petri-net transition: a 120 x 52 box.
fn transition() -> Node {
    node(120.0, 52.0, Shape::Box)
}

/// A process-map activity: a 150 x 58 box (half-extents 75 x 29).
fn activity() -> Node {
    node(150.0, 58.0, Shape::Box)
}

/// `nd`, seeded at `(x, y)` and pinned there, like every node a [`reroute()`] sees.
fn pinned_at(nd: Node, x: f64, y: f64) -> Node {
    Node {
        position: Some(Point::new(x, y)),
        pinned: true,
        ..nd
    }
}

/// `weights` and `thickness` are per edge. An empty slice means weight 1 and thickness 2 for all.
fn graph(nodes: Vec<Node>, edges: &[(usize, usize)], weights: &[f64], thickness: &[f64]) -> Graph {
    assert!(weights.is_empty() || weights.len() == edges.len());
    assert!(thickness.is_empty() || thickness.len() == edges.len());
    let mut b = GraphBuilder::new();
    for nd in nodes {
        b.add_node(nd);
    }
    for (i, &(s, t)) in edges.iter().enumerate() {
        let defaults = Edge::new(NodeId(s as u32), NodeId(t as u32));
        b.add_edge(Edge {
            weight: weights.get(i).copied().unwrap_or(defaults.weight),
            thickness: thickness.get(i).copied().unwrap_or(defaults.thickness),
            ..defaults
        });
    }
    b.build()
}

/// The layered preset, left to right.
fn layered_lr() -> LayoutOptions {
    LayoutOptions {
        direction: Direction::LeftRight,
        ..LayoutOptions::layered_default()
    }
}

/// The process-map (DFG) preset, top to bottom, with the given router.
fn process_map(style: RoutingStyle) -> LayoutOptions {
    let mut o = LayoutOptions::process_map();
    o.direction = Direction::TopBottom;
    o.routing.style = style;
    o
}

/// Seed every node of `g` at its centre in `from`.
fn seed_all(g: &mut Graph, from: &Layout) {
    for (nd, p) in g.nodes.iter_mut().zip(&from.nodes) {
        nd.position = Some(p.center);
    }
}

/// `p` as an `[x, y]` pair.
fn xy(p: Point) -> [f64; 2] {
    [p.x, p.y]
}

/// Node `i`'s centre in `l`.
fn center(l: &Layout, i: usize) -> [f64; 2] {
    xy(l.nodes[i].center)
}

/// Edge `i`'s border-to-border polyline in `l`.
fn route(l: &Layout, i: usize) -> Vec<[f64; 2]> {
    l.edges[i].points.iter().map(|&p| xy(p)).collect()
}

// ---------------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------------

/// Two parallel lanes of places, one group each, through shared ungrouped transitions.
fn two_lane_graph(lane_a_category: u32) -> Graph {
    // 0 A_start,1 B_start | 2 T | 3 A1,4 B1 | 5 T,6 T | 7 A2,8 B2 | 9 T | 10 A_end,11 B_end
    let other = 1 - lane_a_category;
    let place = |c: u32| Node {
        group: Some(GroupId(c)),
        ..place()
    };
    let (a, b) = (lane_a_category, other);
    let nodes = vec![
        place(a),
        place(b),
        transition(),
        place(a),
        place(b),
        transition(),
        transition(),
        place(a),
        place(b),
        transition(),
        place(a),
        place(b),
    ];
    let edges = [
        (0, 2),
        (1, 2),
        (2, 3),
        (2, 4),
        (3, 5),
        (4, 6),
        (5, 7),
        (6, 8),
        (7, 9),
        (8, 9),
        (9, 10),
        (9, 11),
    ];
    graph(nodes, &edges, &[], &[])
}

/// Whether all of category 0 is on one side of all of category 1, the same side in every column.
/// Which side does not matter.
fn lanes_consistent(g: &Graph, layout: &Layout) -> bool {
    use std::collections::BTreeMap;
    let mut columns: BTreeMap<i64, Vec<(u32, f64)>> = BTreeMap::new();
    for (i, nd) in g.nodes.iter().enumerate() {
        if let Some(GroupId(c)) = nd.group {
            let x = layout.nodes[i].center.x.round() as i64;
            columns
                .entry(x)
                .or_default()
                .push((c, layout.nodes[i].center.y));
        }
    }
    let mut zero_first: Option<bool> = None;
    for col in columns.values() {
        let range = |want: u32| {
            col.iter()
                .filter(|(c, _)| *c == want)
                .map(|(_, y)| *y)
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), y| {
                    (lo.min(y), hi.max(y))
                })
        };
        let (lo0, hi0) = range(0);
        let (lo1, hi1) = range(1);
        if !lo0.is_finite() || !lo1.is_finite() {
            continue; // only one of the two lanes reaches this column
        }
        let above = if hi0 < lo1 {
            true
        } else if hi1 < lo0 {
            false
        } else {
            return false; // the two lanes interleave here
        };
        if *zero_first.get_or_insert(above) != above {
            return false; // ...or they swapped sides between two columns
        }
    }
    true
}

/// `n` places in a chain, left to right.
fn line_graph(n: usize) -> Graph {
    let edges: Vec<(usize, usize)> = (0..n.saturating_sub(1)).map(|i| (i, i + 1)).collect();
    graph((0..n).map(|_| place()).collect(), &edges, &[], &[])
}

/// Whether segment `p->q` passes through the interior (inset 2px) of the box centred at `c` with
/// half-extents `hw`, `hh`, checked at sample points.
fn seg_hits_box(p: [f64; 2], q: [f64; 2], c: [f64; 2], hw: f64, hh: f64) -> bool {
    let inset = 2.0;
    let (l, r, t, b) = (
        c[0] - hw + inset,
        c[0] + hw - inset,
        c[1] - hh + inset,
        c[1] + hh - inset,
    );
    for k in 0..=64 {
        let f = k as f64 / 64.0;
        let (x, y) = (p[0] + (q[0] - p[0]) * f, p[1] + (q[1] - p[1]) * f);
        if x > l && x < r && y > t && y < b {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------------------------
// Seeded relayout and pinning
// ---------------------------------------------------------------------------------------------

/// Seeding every node at its current centre and dragging one (pinned) must move only that
/// node: every other node stays exactly put, and the dragged node holds its drop position.
#[test]
fn seeded_relayout_keeps_undragged_nodes_put() {
    let o = layered_lr();
    let base = line_graph(5);
    let o0 = layout(&base, &o).unwrap();

    let mut seeded = line_graph(5);
    seed_all(&mut seeded, &o0);
    let target_y = center(&o0, 2)[1] + 120.0;
    seeded.nodes[2].position = Some(Point::new(center(&o0, 2)[0], target_y));
    seeded.nodes[2].pinned = true;
    let o1 = layout(&seeded, &o).unwrap();

    assert!(
        (center(&o1, 2)[1] - target_y).abs() < 1e-6,
        "pinned node must hold its drop y"
    );
    assert!(
        (center(&o1, 2)[0] - center(&o0, 2)[0]).abs() < 1e-6,
        "layer-x is structural, unchanged"
    );
    for i in [0usize, 1, 3, 4] {
        assert!(
            (center(&o1, i)[0] - center(&o0, i)[0]).abs() < 1e-6
                && (center(&o1, i)[1] - center(&o0, i)[1]).abs() < 1e-6,
            "node {i} must stay put on a stable relayout"
        );
    }
}

/// A pinned node holds its dropped position on the layer axis too, off its own column.
#[test]
fn seeded_pin_holds_layer_axis() {
    let o = layered_lr();
    let base = line_graph(5);
    let o0 = layout(&base, &o).unwrap(); // LR: layer axis = x.
    let mut seeded = line_graph(5);
    seed_all(&mut seeded, &o0);
    let target_x = center(&o0, 2)[0] + 200.0; // drag node 2 far along the layer (x) axis
    seeded.nodes[2].position = Some(Point::new(target_x, center(&o0, 2)[1]));
    seeded.nodes[2].pinned = true;
    let o1 = layout(&seeded, &o).unwrap();
    assert!(
        (center(&o1, 2)[0] - target_x).abs() < 1e-6,
        "pinned node must hold its dropped layer-axis position, got {} want {target_x}",
        center(&o1, 2)[0]
    );
    // Un-pinned neighbours keep their layer columns.
    for i in [0usize, 1, 3, 4] {
        assert!(
            (center(&o1, i)[0] - center(&o0, i)[0]).abs() < 1e-6,
            "node {i} column must not move"
        );
    }
}

/// A pinned node dropped on a sibling in the same layer holds its spot, and the sibling moves
/// aside so the two do not overlap.
#[test]
fn seeded_relayout_respects_separation() {
    // Diamond: 0 -> {1,2} -> 3, so nodes 1 and 2 share the middle layer.
    let o = layered_lr();
    let g = graph(
        vec![place(), place(), place(), place()],
        &[(0, 1), (0, 2), (1, 3), (2, 3)],
        &[],
        &[],
    );
    let o0 = layout(&g, &o).unwrap();
    // Which of 1/2 is upper vs lower is structural. Drag the lower one up onto the upper one.
    let (upper, lower) = if center(&o0, 1)[1] < center(&o0, 2)[1] {
        (1, 2)
    } else {
        (2, 1)
    };
    let mut seeded = g;
    seed_all(&mut seeded, &o0);
    seeded.nodes[lower].position = Some(Point::new(center(&o0, lower)[0], center(&o0, upper)[1]));
    seeded.nodes[lower].pinned = true;
    let o1 = layout(&seeded, &o).unwrap();

    assert!(
        (center(&o1, lower)[1] - center(&o0, upper)[1]).abs() < 1e-6,
        "pinned node holds its (overlapping) drop position"
    );
    assert!(
        (center(&o1, lower)[1] - center(&o1, upper)[1]).abs() >= 51.9,
        "the sibling must yield so the two never overlap (>= one node height apart)"
    );
}

// ---------------------------------------------------------------------------------------------
// Flow spines and corridors
// ---------------------------------------------------------------------------------------------

/// The heavy flow stays in one straight column even when a rare-path node sits in the layer a
/// heavy skip edge passes through.
#[test]
fn flow_node_centers_between_predecessors() {
    // 0 create -> 1 send -> 2 failed -> 3 delivered, plus the heavy skip 1 -> 3.
    let g = graph(
        vec![activity(), activity(), activity(), activity()],
        &[(0, 1), (1, 3), (1, 2), (2, 3)],
        &[
            1.0 + 7100.0f64.ln(),
            1.0 + 5917.0f64.ln(),
            1.0 + 214.0f64.ln(),
            1.0 + 214.0f64.ln(),
        ],
        &[7.7, 7.2, 4.1, 4.1],
    );
    let o = layout(&g, &process_map(RoutingStyle::Orthogonal)).unwrap();
    let x = |i: usize| o.nodes[i].center.x;
    // The heavy skip 1->3 and the light detour 1->2->3 form a fork/join block. The preset draws
    // its heaviest branch straight (`BranchStyle::StraightMain`), so 0, 1 and 3 share a column.
    assert!(
        (x(0) - x(1)).abs() < 1.0,
        "heavy forward edge 0->1 still aligns: got {:.1} / {:.1}",
        x(0),
        x(1)
    );
    assert!(
        (x(3) - x(1)).abs() < 1.0,
        "the heavy branch of the block is the straight one: x3={:.1}, x1={:.1}",
        x(3),
        x(1)
    );
    assert!(
        (x(2) - x(1)).abs() > 40.0,
        "the light branch node still sits clear of the main line: |{:.1} - {:.1}|",
        x(2),
        x(1)
    );
}

/// A lighter parallel skip edge that detours round the off-spine node ends on the target's
/// border and arrives head-on.
#[test]
fn detour_skip_edge_enters_target_border() {
    let g = graph(
        vec![activity(), activity(), activity(), activity()],
        &[(0, 1), (1, 3), (1, 3), (1, 2), (2, 3)],
        &[
            1.0 + 7100.0f64.ln(),
            1.0 + 5917.0f64.ln(),
            1.0 + 914.0f64.ln(),
            1.0 + 214.0f64.ln(),
            1.0 + 214.0f64.ln(),
        ],
        &[7.7, 7.2, 7.3, 4.1, 4.1],
    );
    let o = layout(&g, &process_map(RoutingStyle::Orthogonal)).unwrap();
    let [cx3, cy3] = center(&o, 3);
    let detour = route(&o, 2);
    assert!(detour.len() >= 2, "detour must have a route");
    let end = detour[detour.len() - 1];
    let on_border = (end[1] - (cy3 - 29.0)).abs() < 1.0 && (end[0] - cx3).abs() <= 76.0
        || (end[0] - cx3).abs() >= 74.0
            && (end[0] - cx3).abs() <= 76.0
            && (end[1] - cy3).abs() <= 30.0;
    assert!(
        on_border,
        "detour end {end:?} must lie ON the target border (target centre ({cx3:.1},{cy3:.1}))"
    );
    let prev = detour[detour.len() - 2];
    let (dx, dy) = (end[0] - prev[0], end[1] - prev[1]);
    // An end on the top border must arrive mostly vertically. A near-flat approach leaves the
    // arrowhead floating.
    if (end[1] - (cy3 - 29.0)).abs() < 1.0 {
        assert!(
            dy.abs() >= dx.abs() * 0.5,
            "top-border entry must approach downward, got segment ({dx:.1},{dy:.1})"
        );
    }
}

/// Parallel arcs between the same two nodes share one corridor: the same route shape in offset
/// lanes, each ending on the node borders.
#[test]
fn parallel_arcs_ride_one_corridor() {
    let g = graph(
        vec![activity(), activity()],
        &[(0, 1), (0, 1), (0, 1)],
        &[9.0, 8.0, 7.0],
        &[8.0, 7.0, 4.0],
    );
    let o = layout(&g, &process_map(RoutingStyle::Orthogonal)).unwrap();
    let routes: Vec<Vec<[f64; 2]>> = (0..o.edges.len()).map(|i| route(&o, i)).collect();
    assert_eq!(routes.len(), 3);
    let len0 = routes[0].len();
    assert!(len0 >= 2);
    for r in &routes {
        assert_eq!(r.len(), len0, "all lanes must share the bundle's shape");
    }
    // Lanes are parallel: the lateral delta between two lanes is the same at every point.
    for pair in [(0usize, 1usize), (1, 2)] {
        let (lane_a, lane_b) = (&routes[pair.0], &routes[pair.1]);
        let d0 = (lane_b[0][0] - lane_a[0][0], lane_b[0][1] - lane_a[0][1]);
        // Both lanes are `len0` long (asserted above), so this visits every point.
        for (k, (a, b)) in lane_a.iter().zip(lane_b).enumerate() {
            let dk = (b[0] - a[0], b[1] - a[1]);
            assert!(
                (dk.0 - d0.0).abs() < 0.6 && (dk.1 - d0.1).abs() < 0.6,
                "lane {pair:?} not parallel at point {k}: {dk:?} vs {d0:?}"
            );
        }
        assert!(
            d0.0.hypot(d0.1) >= 10.0,
            "lanes must be visibly separated, got {:.1}",
            d0.0.hypot(d0.1)
        );
    }
    // Every lane starts on the source's bottom border and ends on the target's top.
    let (sy, ty) = (center(&o, 0)[1] + 29.0, center(&o, 1)[1] - 29.0);
    for r in &routes {
        assert!(
            (r[0][1] - sy).abs() < 0.6,
            "lane start off the source border: {:?}",
            r[0]
        );
        assert!(
            (r[r.len() - 1][1] - ty).abs() < 0.6,
            "lane end off the target border: {:?}",
            r[r.len() - 1]
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Lanes
// ---------------------------------------------------------------------------------------------

/// With the lane budget on, each category keeps a lane of its own, and which lane is which
/// follows the drawing, not the category numbering.
#[test]
fn category_controls_lane_membership() {
    let lanes = |g: &Graph| -> Layout {
        let mut options = layered_lr();
        options.order.group_lane_budget_percent = 100;
        options.tuning.order.group_lane_budget_floor = 8;
        options.order.group_lane_area_percent = 50;
        layout(g, &options).unwrap()
    };
    // Lane A carries category 0.
    let g0 = two_lane_graph(0);
    let o0 = lanes(&g0);
    assert!(
        lanes_consistent(&g0, &o0),
        "the two categories must not interleave in any column"
    );
    let a_above_b_0 = center(&o0, 0)[1] < center(&o0, 1)[1];

    // Swap the categories. The flow is symmetric, so the same two lanes come out.
    let g1 = two_lane_graph(1);
    let o1 = lanes(&g1);
    assert!(
        lanes_consistent(&g1, &o1),
        "the two categories must not interleave after the swap either"
    );
    let a_above_b_1 = center(&o1, 0)[1] < center(&o1, 1)[1];

    assert_eq!(
        a_above_b_0, a_above_b_1,
        "the band order follows the drawing, so relabelling the categories cannot move a lane"
    );
}

// ---------------------------------------------------------------------------------------------
// Reroute
// ---------------------------------------------------------------------------------------------

/// A reroute after a drop keeps every node where it was dropped and routes the dragged node's
/// edge around the boxes.
#[test]
fn reroute_dragged_node_edge_clears_boxes() {
    // 0 = send, 1 = delivered. Heavy 0 -> 1 arc.
    let options = process_map(RoutingStyle::Orthogonal);
    let g = graph(
        vec![activity(), activity()],
        &[(0, 1)],
        &[1.0 + 5917.0f64.ln()],
        &[7.2],
    );
    let base = layout(&g, &options).unwrap();
    let send = center(&base, 0);
    let dropped = [send[0] + 480.0, send[1] - 130.0]; // delivered dragged up-and-right of send

    let mut drag = g;
    drag.nodes[0] = pinned_at(drag.nodes[0].clone(), send[0], send[1]);
    drag.nodes[1] = pinned_at(drag.nodes[1].clone(), dropped[0], dropped[1]);
    let out = reroute(&drag, &options).unwrap();

    assert!(
        (center(&out, 0)[0] - send[0]).abs() < 1e-6 && (center(&out, 0)[1] - send[1]).abs() < 1e-6,
        "send must stay exactly put"
    );
    assert!(
        (center(&out, 1)[0] - dropped[0]).abs() < 1e-6
            && (center(&out, 1)[1] - dropped[1]).abs() < 1e-6,
        "delivered must hold its drop position exactly"
    );

    let route = route(&out, 0);
    assert!(route.len() >= 2, "the arc must be routed");
    for w in route.windows(2) {
        for c in [center(&out, 0), center(&out, 1)] {
            assert!(
                !seg_hits_box(w[0], w[1], c, 75.0, 29.0),
                "route segment {:?}->{:?} rakes through the box at {:?}",
                w[0],
                w[1],
                c
            );
        }
    }
}

/// An identity reroute (nothing dragged) reproduces the fresh routes and never puts a multi-layer
/// edge's waypoints onto the nodes its chain skips.
#[test]
fn reroute_identity_reproduces_fresh_and_clears_boxes() {
    let options = process_map(RoutingStyle::Orthogonal);
    let edges = [(0usize, 1usize), (1, 2), (2, 3), (3, 4), (0, 4)];
    let g = graph(vec![activity(); 5], &edges, &[5.0; 5], &[]);
    let fresh = layout(&g, &options).unwrap();
    let mut seeded = g;
    seed_all(&mut seeded, &fresh);
    for nd in &mut seeded.nodes {
        nd.pinned = true;
    }
    let re = reroute(&seeded, &options).unwrap();

    for i in 0..re.edges.len() {
        let (r, f) = (route(&re, i), route(&fresh, i));
        assert_eq!(
            r.len(),
            f.len(),
            "edge {i} route length changed on identity reroute"
        );
        for (a, b) in r.iter().zip(&f) {
            assert!(
                (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6,
                "edge {i} route changed on identity reroute: {a:?} vs {b:?}"
            );
        }
    }
    for (ei, &(s, t)) in edges.iter().enumerate() {
        for w in route(&re, ei).windows(2) {
            for ni in 0..5 {
                if ni == s || ni == t {
                    continue;
                }
                assert!(
                    !seg_hits_box(w[0], w[1], center(&re, ni), 75.0, 29.0),
                    "edge {ei} rakes node {ni} on identity reroute"
                );
            }
        }
    }
}

/// In orthogonal mode, a reroute moves an end to a perpendicular border where needed, so no
/// route crosses a box it does not end on.
#[test]
fn reroute_orthogonal_end_approach_clears_boxes() {
    let act = |x: f64, y: f64| pinned_at(activity(), x, y);
    // 0 create, 1 send, 2 delivered (dragged up-right), 3 failed.
    let edges = [(0usize, 1usize), (1, 2), (1, 3), (3, 2)];
    let g = graph(
        vec![
            act(120.75, 495.0),
            act(118.65, 725.0),
            act(601.96, 594.61),
            act(370.54, 509.32),
        ],
        &edges,
        &[9.0, 9.0, 4.0, 4.0],
        &[7.7, 7.2, 4.1, 4.1],
    );
    let out = reroute(&g, &process_map(RoutingStyle::Orthogonal)).unwrap();
    for (ei, &(s, t)) in edges.iter().enumerate() {
        for w in route(&out, ei).windows(2) {
            for ni in 0..4 {
                if ni == s || ni == t {
                    continue;
                }
                assert!(
                    !seg_hits_box(w[0], w[1], center(&out, ni), 75.0, 29.0),
                    "orthogonal edge {ei} rakes node {ni}"
                );
            }
        }
    }
}

/// A long edge whose straight channel crosses two nodes, with waypoints on opposite sides, is
/// routed to one side, and no route crosses a non-endpoint box.
#[test]
fn reroute_long_edge_single_lane_clears_boxes() {
    // Positions from an object-centric DFG: (x, y, terminal).
    let pos: [(f64, f64, bool); 11] = [
        (0.0, 18.0, true),
        (0.0, 129.0, false),
        (120.75, 495.0, false),
        (120.75, 617.0, false),
        (180.35, 861.0, false),
        (92.91, 229.34, false),
        (-164.6, 251.0, false),
        (-164.6, 1105.0, false),
        (-59.6, 983.0, false),
        (61.15, 739.0, false),
        (-164.6, 1216.0, true),
    ];
    let nodes: Vec<Node> = pos
        .iter()
        .map(|&(x, y, t)| {
            let nd = if t {
                node(36.0, 36.0, Shape::Ellipse)
            } else {
                activity()
            };
            pinned_at(nd, x, y)
        })
        .collect();
    let e: [(usize, usize, f64); 26] = [
        (0, 1, 7659.0),
        (2, 3, 7100.0),
        (3, 4, 5917.0),
        (5, 2, 5290.0),
        (1, 6, 5232.0),
        (7, 10, 4278.0),
        (6, 5, 3528.0),
        (4, 7, 3381.0),
        (4, 8, 2562.0),
        (1, 5, 1915.0),
        (8, 7, 1669.0),
        (4, 8, 1606.0),
        (5, 6, 1495.0),
        (6, 7, 1363.0),
        (0, 1, 2000.0),
        (7, 10, 2000.0),
        (1, 6, 2000.0),
        (6, 7, 1557.0),
        (6, 8, 443.0),
        (8, 7, 443.0),
        (0, 2, 1128.0),
        (2, 3, 1128.0),
        (4, 7, 1128.0),
        (3, 4, 914.0),
        (9, 4, 214.0),
        (3, 9, 214.0),
    ];
    let edges: Vec<(usize, usize)> = e.iter().map(|&(a, b, _)| (a, b)).collect();
    let weights: Vec<f64> = e.iter().map(|&(_, _, c)| 1.0 + c.ln()).collect();
    let thickness: Vec<f64> = e
        .iter()
        .map(|&(_, _, c)| 1.0 + 7.0 * (c / 7659.0).sqrt())
        .collect();
    let g = graph(nodes, &edges, &weights, &thickness);
    for flow_diagonal in [false, true] {
        let style = if flow_diagonal {
            RoutingStyle::Diagonal
        } else {
            RoutingStyle::Orthogonal
        };
        let out = reroute(&g, &process_map(style)).unwrap();
        for (ei, &(s, t)) in edges.iter().enumerate() {
            for w in route(&out, ei).windows(2) {
                for (ni, &(_, _, term)) in pos.iter().enumerate() {
                    if ni == s || ni == t {
                        continue;
                    }
                    let (hw, hh) = if term { (18.0, 18.0) } else { (75.0, 29.0) };
                    assert!(
                        !seg_hits_box(w[0], w[1], center(&out, ni), hw, hh),
                        "diagonal={flow_diagonal}: edge {ei} ({s}->{t}) rakes node {ni}"
                    );
                }
            }
        }
    }
}

/// Anti-parallel edges between two same-row nodes (a 2-cycle) do not cross.
#[test]
fn reroute_antiparallel_same_row_no_cross() {
    // A and B sit on about the same row, with edges A->B and B->A.
    let g = graph(
        vec![
            pinned_at(activity(), -164.6, 251.0),
            pinned_at(activity(), 30.9, 261.0),
        ],
        &[(0, 1), (1, 0)],
        &[1.0 + 3528.0f64.ln(), 1.0 + 1495.0f64.ln()],
        &[5.75, 4.1],
    );
    let out = reroute(&g, &process_map(RoutingStyle::Diagonal)).unwrap();
    let (ab, ba) = (route(&out, 0), route(&out, 1));
    let ccw = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    };
    for i in 0..ab.len() - 1 {
        for j in 0..ba.len() - 1 {
            let (p, q, r, s) = (ab[i], ab[i + 1], ba[j], ba[j + 1]);
            let proper = ccw(p, q, r) * ccw(p, q, s) < 0.0 && ccw(r, s, p) * ccw(r, s, q) < 0.0;
            assert!(!proper, "anti-parallel edges cross at segment {i}/{j}");
        }
    }
}

/// A target dropped above or below its source's column is entered from outside: no route crosses
/// a box, its own source and target included.
#[test]
fn reroute_target_dropped_by_source_column_clears_own_ends() {
    let chain = graph(
        (0..4).map(|_| activity()).collect(),
        &[(0, 1), (1, 2), (2, 3)],
        &[],
        &[],
    );
    let mut map_lr = LayoutOptions::process_map();
    map_lr.direction = Direction::LeftRight;
    for options in [layered_lr(), map_lr] {
        let first = layout(&chain, &options).unwrap();
        let source = center(&first, 1);
        for (dx, dy) in [
            (0.0, -120.0),
            (60.0, -120.0),
            (-100.0, -120.0),
            (0.0, 120.0),
        ] {
            let mut g = chain.clone();
            seed_all(&mut g, &first);
            g.nodes[2].position = Some(Point::new(source[0] + dx, source[1] + dy));
            let out = reroute(&g, &options).unwrap();
            for ei in 0..3 {
                let r = route(&out, ei);
                for w in r.windows(2) {
                    for ni in 0..4 {
                        assert!(
                            !seg_hits_box(w[0], w[1], center(&out, ni), 75.0, 29.0),
                            "drop ({dx}, {dy}): edge {ei} rakes node {ni}: {r:?}"
                        );
                    }
                }
            }
        }
    }
}

/// Nodes dropped closer than the node gap stay exactly where they were dropped, so the routes fit
/// the positions the caller shows.
#[test]
fn reroute_keeps_nodes_dropped_inside_the_node_gap() {
    let g0 = graph(
        (0..4).map(|_| activity()).collect(),
        &[(0, 1), (1, 2), (2, 3), (1, 3)],
        &[],
        &[],
    );
    let mut options = LayoutOptions::process_map();
    options.direction = Direction::LeftRight;
    let first = layout(&g0, &options).unwrap();
    let source = center(&first, 1);
    for dy in [-89.0, 89.0] {
        let mut g = g0.clone();
        seed_all(&mut g, &first);
        g.nodes[2].position = Some(Point::new(source[0], source[1] + dy));
        let out = reroute(&g, &options).unwrap();
        for (i, nd) in g.nodes.iter().enumerate() {
            let want = xy(nd.position.unwrap());
            let got = center(&out, i);
            assert!(
                (got[0] - want[0]).abs() < 1e-6 && (got[1] - want[1]).abs() < 1e-6,
                "drop {dy}: node {i} moved from {want:?} to {got:?}"
            );
        }
    }
}

/// An edge between nodes stacked across the flow, their boxes overlapping along it, runs straight
/// between the borders that face each other.
#[test]
fn reroute_joins_stacked_nodes_by_their_facing_borders() {
    let chain = graph(
        (0..4).map(|_| activity()).collect(),
        &[(0, 1), (1, 2), (2, 3)],
        &[],
        &[],
    );
    let mut map_lr = LayoutOptions::process_map();
    map_lr.direction = Direction::LeftRight;
    for options in [layered_lr(), map_lr] {
        let first = layout(&chain, &options).unwrap();
        let source = center(&first, 1);
        for (dx, dy) in [(0.0, -120.0), (60.0, -120.0), (0.0, 120.0), (-40.0, 150.0)] {
            let mut g = chain.clone();
            seed_all(&mut g, &first);
            g.nodes[2].position = Some(Point::new(source[0] + dx, source[1] + dy));
            let out = reroute(&g, &options).unwrap();
            let r = route(&out, 1);
            let up = dy < 0.0;
            assert_eq!(r.len(), 2, "drop ({dx}, {dy}): {r:?}");
            assert!(
                (r[0][0] - r[1][0]).abs() < 1e-6,
                "drop ({dx}, {dy}): not vertical {r:?}"
            );
            let (from, to) = (center(&out, 1), center(&out, 2));
            assert!(
                (r[0][1] - (from[1] + if up { -29.0 } else { 29.0 })).abs() < 1e-6,
                "{r:?}"
            );
            assert!(
                (r[1][1] - (to[1] + if up { 29.0 } else { -29.0 })).abs() < 1e-6,
                "{r:?}"
            );
        }
    }
}

/// Two pinned nodes closer than the node gap keep their places in a fresh layout, and a node
/// between them in the layer stays between them.
#[test]
fn pinned_nodes_closer_than_the_gap_keep_the_layer_order() {
    let at = |x: f64, y: f64, pinned: bool| Node {
        position: Some(Point::new(x, y)),
        pinned,
        ..node(100.0, 40.0, Shape::Box)
    };
    let g = graph(
        vec![
            at(0.0, 0.0, false),
            at(200.0, 0.0, true),
            at(200.0, 15.0, false),
            at(200.0, 30.0, true),
        ],
        &[(0, 1), (0, 2), (0, 3)],
        &[],
        &[],
    );
    let mut options = layered_lr();
    options.seed_mode = SeedMode::Structure;
    let out = layout(&g, &options).unwrap();
    let (a, b, c) = (center(&out, 1), center(&out, 2), center(&out, 3));
    // Pins hold across the layer; the layer axis is the layout's under `SeedMode::Structure`.
    assert_eq!((a[1], c[1]), (0.0, 30.0));
    assert!(a[1] < b[1] && b[1] < c[1], "{a:?} {b:?} {c:?}");
    assert!(out.nodes[1].order < out.nodes[2].order && out.nodes[2].order < out.nodes[3].order);
}

/// A straight join between stacked nodes ends on a place's circle, not on its bounding box.
#[test]
fn reroute_joins_a_place_on_its_circle() {
    let g = graph(
        vec![
            pinned_at(place(), 0.0, 0.0),
            pinned_at(transition(), 50.0, 120.0),
        ],
        &[(0, 1), (1, 0)],
        &[],
        &[],
    );
    let out = reroute(&g, &layered_lr()).unwrap();
    for ei in 0..2 {
        let r = route(&out, ei);
        let end = if ei == 0 { r[0] } else { r[r.len() - 1] };
        let d = (end[0].powi(2) + end[1].powi(2)).sqrt();
        assert!(
            (d - 26.0).abs() < 0.5,
            "edge {ei} ends {d:.1} from the place's centre: {r:?}"
        );
    }
}

/// A node whose edges may only use the facing borders keeps them after a reroute.
#[test]
fn reroute_keeps_facing_only_ports() {
    let a = Node {
        port_sides: Some(PortSides::FacingOnly),
        ..pinned_at(activity(), 0.0, 0.0)
    };
    let g = graph(
        vec![a, pinned_at(activity(), 0.0, 120.0)],
        &[(0, 1)],
        &[],
        &[],
    );
    let out = reroute(&g, &layered_lr()).unwrap();
    let r = route(&out, 0);
    assert!(
        (r[0][0] - 75.0).abs() < 1e-6,
        "leaves by its east border: {r:?}"
    );
}

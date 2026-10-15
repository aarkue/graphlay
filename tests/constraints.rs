//! Layer constraints: [`RankConstraint::Fixed`] lines nodes up across otherwise unrelated parts of
//! a graph.

use graphlay::{
    layout, Edge, Graph, GraphBuilder, LayoutError, LayoutOptions, Node, NodeId, RankConstraint,
    Size,
};

fn graph(n: usize, edges: &[(u32, u32)], fixed: &[(usize, i32)]) -> Graph {
    let mut b = GraphBuilder::new();
    for _ in 0..n {
        b.add_node(Node {
            size: Size::new(80.0, 40.0),
            ..Node::default()
        });
    }
    for &(s, t) in edges {
        b.add_edge(Edge::new(NodeId(s), NodeId(t)));
    }
    let mut g = b.build();
    for &(i, k) in fixed {
        g.nodes[i].rank = RankConstraint::Fixed(k);
    }
    g
}

fn ranks(g: &Graph) -> Vec<i32> {
    layout(g, &LayoutOptions::default())
        .expect("lays out")
        .nodes
        .iter()
        .map(|p| p.rank.expect("ranked"))
        .collect()
}

#[test]
fn nodes_with_one_fixed_value_share_a_layer() {
    // Two chains of different lengths: a-b-c-d and e-f. Free, `f` sits on layer 1. Fixed to the
    // same layer as `d`, it moves to layer 3.
    let edges = [(0, 1), (1, 2), (2, 3), (4, 5)];
    let free = ranks(&graph(6, &edges, &[]));
    assert_eq!(free[5], 1);
    let r = ranks(&graph(6, &edges, &[(3, 7), (5, 7)]));
    assert_eq!(r[3], r[5]);
    assert_eq!(r[3], 3);
}

#[test]
fn a_larger_value_lands_at_least_that_much_further() {
    let r = ranks(&graph(3, &[], &[(0, 0), (1, 2), (2, 5)]));
    assert!(r[1] - r[0] >= 2 && r[2] - r[1] >= 3, "{r:?}");
}

#[test]
fn a_fixed_node_without_edges_keeps_its_layer() {
    // a-b-c, and a lone d fixed with c.
    let r = ranks(&graph(4, &[(0, 1), (1, 2)], &[(2, 1), (3, 1)]));
    assert_eq!(r[3], r[2]);
}

#[test]
fn an_edge_against_the_fixed_order_is_drawn_backwards() {
    let g = graph(2, &[(0, 1)], &[(0, 4), (1, 1)]);
    let l = layout(&g, &LayoutOptions::default()).unwrap();
    assert!(l.nodes[0].rank > l.nodes[1].rank);
    assert_eq!(l.reversed.len(), 1);
}

#[test]
fn an_edge_inside_one_fixed_layer_is_an_error() {
    let g = graph(2, &[(0, 1)], &[(0, 2), (1, 2)]);
    assert!(matches!(
        layout(&g, &LayoutOptions::default()),
        Err(LayoutError::EdgeWithinFixedLayer { .. })
    ));
}

#[test]
fn fixed_layers_the_edges_contradict_are_dropped() {
    // `a` is fixed before `b`, but the path b -> x -> a says the opposite.
    let g = graph(3, &[(1, 2), (2, 0)], &[(0, 0), (1, 3)]);
    let r = ranks(&g);
    // Laid out as if free: b, x, a in order.
    assert!(r[1] < r[2] && r[2] < r[0], "{r:?}");
}

/// `Margins::along_neg` / `along_pos` keep the previous and next layers that much further away.
#[test]
fn along_margins_push_the_neighbouring_layers_away() {
    let gaps = |before: f64, after: f64| {
        let mut g = graph(3, &[(0, 1), (1, 2)], &[]);
        g.nodes[1].margin.along_neg = before;
        g.nodes[1].margin.along_pos = after;
        let l = layout(&g, &LayoutOptions::default()).expect("lays out");
        let x = |i: usize| l.nodes[i].center.x;
        (x(1) - x(0), x(2) - x(1))
    };
    let (a0, b0) = gaps(0.0, 0.0);
    let (a1, b1) = gaps(50.0, 0.0);
    let (a2, b2) = gaps(0.0, 30.0);
    assert!(
        (a1 - a0 - 50.0).abs() < 1e-6 && (b1 - b0).abs() < 1e-6,
        "{a0} {a1} {b0} {b1}"
    );
    assert!(
        (a2 - a0).abs() < 1e-6 && (b2 - b0 - 30.0).abs() < 1e-6,
        "{a0} {a2} {b0} {b2}"
    );
}

/// `Limits::max_dummies` stops a drawing whose long edges need more waypoints than allowed.
#[test]
fn max_dummies_is_enforced() {
    // A chain of five plus one edge from the first node to the last: three layer gaps to bridge.
    let g = graph(5, &[(0, 1), (1, 2), (2, 3), (3, 4), (0, 4)], &[]);
    let mut o = LayoutOptions::default();
    o.limits.max_dummies = Some(3);
    assert!(layout(&g, &o).is_ok());
    o.limits.max_dummies = Some(2);
    assert_eq!(
        layout(&g, &o).unwrap_err(),
        LayoutError::TooLarge {
            what: "routing waypoints",
            got: 3,
            limit: 2
        }
    );
}

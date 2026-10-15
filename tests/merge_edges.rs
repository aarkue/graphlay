//! Merged edges ([`Routing::merge_edges`]): a fan-out leaves its node on one trunk, and the trunk's
//! branch points come back as [`graphlay::Layout::junctions`].

use graphlay::{
    layout, Edge, Graph, GraphBuilder, Layout, LayoutOptions, MergeEdges, Node, NodeId, Size,
};

/// One node fanning out to three, which all fan back into one.
fn diamond() -> Graph {
    let mut b = GraphBuilder::new();
    for _ in 0..5 {
        b.add_node(Node {
            size: Size::new(80.0, 40.0),
            ..Node::default()
        });
    }
    for (s, t) in [(0, 1), (0, 2), (0, 3), (1, 4), (2, 4), (3, 4)] {
        b.add_edge(Edge::new(NodeId(s), NodeId(t)));
    }
    b.build()
}

fn run(merge: MergeEdges) -> Layout {
    let mut o = LayoutOptions::default();
    o.routing.merge_edges = merge;
    layout(&diamond(), &o).expect("lays out")
}

#[test]
fn off_by_default_and_without_junctions() {
    assert_eq!(
        LayoutOptions::default().routing.merge_edges,
        MergeEdges::None
    );
    assert!(run(MergeEdges::None).junctions.is_empty());
}

#[test]
fn a_fork_leaves_on_one_port() {
    let l = run(MergeEdges::Forks);
    let starts: Vec<_> = l.edges[..3].iter().map(|e| e.points[0]).collect();
    assert!(
        starts.windows(2).all(|w| w[0] == w[1]),
        "the three edges out of node 0 should leave from one point: {starts:?}"
    );
    assert!(!l.junctions.is_empty());
    // The join is untouched: three separate arrival points.
    let ends: Vec<_> = l.edges[3..]
        .iter()
        .map(|e| *e.points.last().expect("routed"))
        .collect();
    assert!(ends.windows(2).all(|w| w[0] != w[1]), "{ends:?}");
}

#[test]
fn both_merges_forks_and_joins_and_keeps_the_nodes_where_they_were() {
    let off = run(MergeEdges::None);
    let on = run(MergeEdges::Both);
    // Merging changes only the routes, so the nodes stay put.
    for (a, b) in off.nodes.iter().zip(&on.nodes) {
        assert_eq!(a.center, b.center);
    }
    let ends: Vec<_> = on.edges[3..]
        .iter()
        .map(|e| *e.points.last().expect("routed"))
        .collect();
    assert!(ends.windows(2).all(|w| w[0] == w[1]), "{ends:?}");
    // Every edge keeps a route.
    for e in &on.edges {
        assert!(e.points.len() >= 2);
    }
}

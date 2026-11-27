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
fn auto_by_default_and_without_junctions_on_roomy_borders() {
    assert_eq!(
        LayoutOptions::default().routing.merge_edges,
        MergeEdges::Auto
    );
    assert!(run(MergeEdges::Auto).junctions.is_empty());
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

/// A small sink whose incoming ends would sit closer than half the port gap gets them on one
/// trunk under `MergeEdges::Auto`, and keeps them apart under `MergeEdges::None`.
#[test]
fn a_crowded_sink_joins_its_edges_on_a_trunk() {
    let mut b = GraphBuilder::new();
    let start = b.add_node(Node {
        size: Size::new(80.0, 40.0),
        ..Node::default()
    });
    let sink = b.add_node(Node {
        size: Size::new(20.0, 20.0),
        ..Node::default()
    });
    for _ in 0..12 {
        let mid = b.add_node(Node {
            size: Size::new(80.0, 40.0),
            ..Node::default()
        });
        b.add_edge(Edge::new(start, mid));
        b.add_edge(Edge::new(mid, sink));
    }
    let g = b.build();
    let ends = |merge: MergeEdges| {
        let mut o = LayoutOptions::default();
        o.routing.merge_edges = merge;
        let l = layout(&g, &o).expect("lays out");
        let mut ends: Vec<(i64, i64)> = l
            .edges
            .iter()
            .zip(&g.edges)
            .filter(|(_, e)| e.target == sink)
            .map(|(r, _)| {
                let p = r.points.last().expect("routed");
                (p.x.round() as i64, p.y.round() as i64)
            })
            .collect();
        ends.sort_unstable();
        ends.dedup();
        (ends.len(), l.junctions.len())
    };
    let (auto, junctions) = ends(MergeEdges::Auto);
    let (none, none_junctions) = ends(MergeEdges::None);
    assert!(
        junctions > 0 && none_junctions == 0,
        "{junctions}, {none_junctions}"
    );
    assert!(auto < none, "{auto} ends under auto, {none} under none");
}

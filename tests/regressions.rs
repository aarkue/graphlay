//! Regression tests on small graphs.

use graphlay::{metrics, Direction, GraphBuilder, LayoutOptions, Node, NodeId, Shape, Size};

/// A Petri net with a 2-cycle (p1 -> b -> p1) and a duplicate arc (p1 -> b twice) draws with no
/// overlapping edges, so the reverse arc does not sit on a bundle lane.
#[test]
fn a_reverse_arc_does_not_land_on_a_bundle_lane() {
    let mut b = GraphBuilder::new();
    for _ in 0..3 {
        b.add_node(Node {
            size: Size::new(52.0, 52.0),
            shape: Shape::Ellipse,
            ..Node::default()
        });
    }
    for _ in 0..2 {
        b.add_node(Node {
            size: Size::new(120.0, 52.0),
            ..Node::default()
        });
    }
    for (s, t) in [(0, 3), (3, 1), (1, 4), (4, 1), (4, 2), (1, 4)] {
        b.connect(NodeId(s), NodeId(t));
    }
    let g = b.build();
    let o = LayoutOptions {
        direction: Direction::LeftRight,
        ..LayoutOptions::layered_default()
    };
    let l = graphlay::layout(&g, &o).unwrap();
    let m = metrics::measure(&g, &o, &l);
    assert_eq!(m.overlaps, 0);
    assert_eq!(m.node_edge_hits, 0);
}

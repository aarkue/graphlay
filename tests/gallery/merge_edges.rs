//! Merged edges
//!
//! `Routing::merge_edges` draws the edges leaving a node (forks) and the edges reaching a node
//! (joins) on one shared trunk that branches off a common riser. `Layout::junctions` lists the
//! branch points, drawn here as dots. A merge is kept only where it adds no crossing, no bend and
//! no node hit, and an arriving edge never shares a trunk with a leaving one.

use graphlay::{layout, GraphBuilder, LayoutOptions, MergeEdges, Node, Size};

use super::Panel;

pub fn panels() -> Vec<Panel> {
    let mut b = GraphBuilder::new();
    let names = [
        "Order received",
        "Check stock",
        "Check credit",
        "Check address",
        "Approve order",
        "Pick items",
        "Send invoice",
        "Notify customer",
        "Close order",
    ];
    let n: Vec<_> = names
        .iter()
        .map(|name| {
            b.add_node(Node {
                size: Size::new(120.0, 40.0),
                key: Some(name.to_string()),
                ..Node::default()
            })
        })
        .collect();
    for (s, t) in [(0, 1), (0, 2), (0, 3), (1, 4), (2, 4), (3, 4)] {
        b.connect(n[s], n[t]);
    }
    for (s, t) in [(4, 5), (4, 6), (4, 7), (5, 8), (6, 8), (7, 8)] {
        b.connect(n[s], n[t]);
    }
    let graph = b.build();

    let mut options = LayoutOptions::default();
    let plain = options;
    let separate = layout(&graph, &plain).expect("lays out");
    options.routing.merge_edges = MergeEdges::Both;
    let merged = layout(&graph, &options).expect("lays out");
    vec![
        Panel::new(
            "merge_edges: None (default)",
            graph.clone(),
            &plain,
            separate,
        ),
        Panel::new("merge_edges: Both", graph, &options, merged),
    ]
}

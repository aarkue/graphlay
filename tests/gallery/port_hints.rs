//! Port hints
//!
//! `Edge::source_port` and `Edge::target_port` choose the border an edge leaves or enters
//! (`PortHint::Side`), or an exact point along it (`PortHint::Offset`). Here the rework loop from
//! Approve back to Assess claim leaves and enters at the bottom, so it runs underneath the main
//! line. Both loops are marked `reverse_for_ranking`, which tells the layering they run against
//! the flow, so the main line reads left to right.

use graphlay::{layout, Edge, GraphBuilder, LayoutOptions, Node, PortHint, Side, Size};

use super::Panel;

pub fn panels() -> Vec<Panel> {
    let build = |hinted: bool| {
        let mut b = GraphBuilder::new();
        let n: Vec<_> = [
            "Receive claim",
            "Assess claim",
            "Approve",
            "Pay out",
            "Archive",
        ]
        .iter()
        .map(|name| {
            b.add_node(Node {
                size: Size::new(120.0, 40.0),
                key: Some(name.to_string()),
                ..Node::default()
            })
        })
        .collect();
        for (s, t) in [(0, 1), (1, 2), (2, 3), (3, 4)] {
            b.connect(n[s], n[t]);
        }
        // Send back for review: Approve -> Assess claim, underneath.
        let bottom = hinted.then_some(PortHint::Side(Side::Bottom));
        b.add_edge(Edge {
            source_port: bottom,
            target_port: bottom,
            reverse_for_ranking: true,
            ..Edge::new(n[2], n[1])
        });
        // Reopen: Pay out -> Receive claim.
        b.add_edge(Edge {
            reverse_for_ranking: true,
            ..Edge::new(n[3], n[0])
        });
        b.build()
    };
    let options = LayoutOptions::default();
    let (plain, hinted) = (build(false), build(true));
    let (a, b) = (
        layout(&plain, &options).expect("lays out"),
        layout(&hinted, &options).expect("lays out"),
    );
    vec![
        Panel::new("no hints", plain, &options, a),
        Panel::new("rework loop: Bottom -> Bottom", hinted, &options, b),
    ]
}

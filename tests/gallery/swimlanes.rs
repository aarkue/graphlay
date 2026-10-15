//! Swimlanes
//!
//! `Node::group` puts nodes into lanes. With `GroupLanes::Soft` (the default) groups become bands
//! only where that is cheap. With `GroupLanes::Strict` every group gets a strip of its own,
//! whatever it costs, and `Layout::lanes` returns the strips as rectangles that tile the drawing.
//! `Node::lane_key` orders the nodes inside a lane.

use graphlay::{layout, GraphBuilder, GroupId, GroupLanes, LayoutOptions, Node, Size};

use super::Panel;

/// Lane names, by group id.
pub const LANES: &[&str] = &["Customer", "Sales", "Warehouse", "Finance"];

pub fn panels() -> Vec<Panel> {
    let mut b = GraphBuilder::new();
    let steps = [
        ("Place order", 0),
        ("Receive order", 1),
        ("Check credit", 3),
        ("Confirm order", 1),
        ("Pick items", 2),
        ("Pack", 2),
        ("Ship", 2),
        ("Send invoice", 3),
        ("Receive goods", 0),
        ("Pay invoice", 0),
        ("Book payment", 3),
    ];
    let n: Vec<_> = steps
        .iter()
        .map(|&(name, lane)| {
            b.add_node(Node {
                size: Size::new(120.0, 40.0),
                key: Some(name.to_string()),
                group: Some(GroupId(lane)),
                ..Node::default()
            })
        })
        .collect();
    for (s, t) in [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 4),
        (4, 5),
        (5, 6),
        (3, 7),
        (6, 8),
        (7, 9),
        (8, 9),
        (9, 10),
    ] {
        b.connect(n[s], n[t]);
    }
    let graph = b.build();

    let mut options = LayoutOptions::default();
    let plain = options;
    let soft = layout(&graph, &plain).expect("lays out");
    options.order.group_lanes = GroupLanes::Strict;
    let strict = layout(&graph, &options).expect("lays out");
    vec![
        Panel::new("group_lanes: Soft (default)", graph.clone(), &plain, soft),
        Panel::new("group_lanes: Strict", graph, &options, strict).with_lanes(LANES),
    ]
}

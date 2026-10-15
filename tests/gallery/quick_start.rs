//! A first drawing
//!
//! Nodes are boxes with a size and a key (a stable name, used here as the label). Edges connect
//! them. `layout` assigns every node a centre and every edge a polyline, border to border.

use graphlay::{layout, GraphBuilder, LayoutOptions, Node, Size};

use super::Panel;

pub fn panels() -> Vec<Panel> {
    let mut b = GraphBuilder::new();
    let mut task = |name: &str| {
        b.add_node(Node {
            size: Size::new(120.0, 40.0),
            key: Some(name.to_string()),
            ..Node::default()
        })
    };
    let (register, check, approve, reject, notify) = (
        task("Register"),
        task("Check"),
        task("Approve"),
        task("Reject"),
        task("Notify"),
    );
    for (s, t) in [
        (register, check),
        (check, approve),
        (check, reject),
        (approve, notify),
        (reject, notify),
    ] {
        b.connect(s, t);
    }
    let graph = b.build();
    let options = LayoutOptions::default();
    let drawing = layout(&graph, &options).expect("lays out");
    vec![Panel::new("default options", graph, &options, drawing)]
}

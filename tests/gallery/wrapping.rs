//! Wrapping into rows
//!
//! A long process is much wider than a screen. `WrapOptions::viewport_aspect` breaks the drawing
//! into rows for a viewport of that width-to-height ratio, cutting where the fewest edges cross.

use graphlay::{layout, GraphBuilder, LayoutOptions, Node, Size};

use super::Panel;

pub fn panels() -> Vec<Panel> {
    let mut b = GraphBuilder::new();
    let steps = [
        "Lead",
        "Qualify",
        "Demo",
        "Offer",
        "Negotiate",
        "Contract",
        "Onboard",
        "Kick-off",
        "Deliver",
        "Invoice",
        "Renew",
    ];
    let n: Vec<_> = steps
        .iter()
        .map(|name| {
            b.add_node(Node {
                size: Size::new(100.0, 36.0),
                key: Some(name.to_string()),
                ..Node::default()
            })
        })
        .collect();
    for w in n.windows(2) {
        b.connect(w[0], w[1]);
    }
    // Two shortcuts and a loop back.
    b.connect(n[1], n[5]);
    b.connect(n[4], n[3]);
    b.connect(n[8], n[10]);
    let graph = b.build();

    let mut options = LayoutOptions::default();
    options.wrap.viewport_aspect = 16.0 / 9.0;
    let wrapped = layout(&graph, &options).expect("lays out");
    vec![Panel::new(
        "wrap.viewport_aspect = 16 / 9",
        graph,
        &options,
        wrapped,
    )]
}

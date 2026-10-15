//! Nested subprocesses
//!
//! A node whose `Node::parent` is set is drawn inside that parent, which becomes a box around its
//! children. Compounds nest to any depth and are laid out bottom-up. An edge that crosses a
//! compound's border is cut there and joined again exactly on the border.
//! `Spacing::compound_header` leaves room for a title.

use graphlay::{layout, Direction, GraphBuilder, LayoutOptions, Node, NodeId, Size};

use super::Panel;

pub fn panels() -> Vec<Panel> {
    let mut b = GraphBuilder::new();
    let mut node = |name: &str, parent: Option<NodeId>| {
        b.add_node(Node {
            size: Size::new(110.0, 36.0),
            key: Some(name.to_string()),
            parent,
            ..Node::default()
        })
    };
    let order_in = node("Order in", None);
    let fulfilment = node("Fulfilment", None);
    let check = node("Check stock", Some(fulfilment));
    let backorder = node("Backorder", Some(fulfilment));
    let packaging = node("Packaging", Some(fulfilment));
    let pick = node("Pick", Some(packaging));
    let pack = node("Pack", Some(packaging));
    let label = node("Label", Some(fulfilment));
    let billing = node("Billing", None);
    let invoice = node("Invoice", Some(billing));
    let collect = node("Collect payment", Some(billing));
    let done = node("Done", None);
    for (s, t) in [
        (order_in, check),
        (check, pick),
        (check, backorder),
        (backorder, pick),
        (pick, pack),
        (pack, label),
        (label, done),
        (order_in, invoice),
        (invoice, collect),
        (collect, done),
    ] {
        b.connect(s, t);
    }
    let graph = b.build();

    let mut options = LayoutOptions::default();
    options.spacing.compound_header = 16.0;
    let across_options = options;
    let across = layout(&graph, &across_options).expect("lays out");
    options.direction = Direction::TopBottom;
    let down = layout(&graph, &options).expect("lays out");
    vec![
        Panel::new(
            "Direction::LeftRight",
            graph.clone(),
            &across_options,
            across,
        ),
        Panel::new("Direction::TopBottom", graph, &options, down),
    ]
}

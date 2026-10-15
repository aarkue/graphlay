//! Presets, directions and routing styles
//!
//! A preset is a complete set of options for one kind of drawing, and a request patches the
//! fields it wants different. `layered_default` suits Petri nets and other diagrams whose
//! edges all matter alike. `process_map` is for directly-follows graphs, where thick edges
//! carry the flow. It runs top to bottom and keeps the main path straight. `hub_and_spoke`
//! packs a dense relationship graph around its hubs, and `hub_and_spoke_flow` does the same
//! for one with a direction. The routing style and the direction are fields like any other.

use graphlay::{layout, Direction, Edge, GraphBuilder, LayoutOptions, Node, RoutingStyle, Size};

use super::Panel;

/// An order process with a rework loop and a skip, with edge weights from case counts.
fn order_process() -> graphlay::Graph {
    let mut b = GraphBuilder::new();
    let mut step = |name: &str| {
        b.add_node(Node {
            size: Size::new(110.0, 40.0),
            key: Some(name.to_string()),
            ..Node::default()
        })
    };
    let (receive, check, reserve, pick, ship, invoice, close, reject) = (
        step("Receive"),
        step("Check"),
        step("Reserve"),
        step("Pick"),
        step("Ship"),
        step("Invoice"),
        step("Close"),
        step("Reject"),
    );
    for (s, t, count) in [
        (receive, check, 400.0),
        (check, reserve, 360.0),
        (check, reject, 40.0),
        (reserve, pick, 360.0),
        (pick, ship, 350.0),
        (ship, invoice, 350.0),
        (invoice, close, 350.0),
        (reject, close, 40.0),
        (reserve, invoice, 10.0),
        (pick, check, 25.0),
    ] {
        let mut e = Edge::new(s, t);
        e.weight = 1.0 + f64::ln(count);
        e.thickness = (1.0 + f64::ln(count) / 2.0).min(5.0);
        b.add_edge(e);
    }
    b.build()
}

/// Four object types around one central type, as a data model's type graph looks.
fn type_graph() -> graphlay::Graph {
    let mut b = GraphBuilder::new();
    let mut kind = |name: &str| {
        b.add_node(Node {
            size: Size::new(96.0, 40.0),
            key: Some(name.to_string()),
            ..Node::default()
        })
    };
    let order = kind("Order");
    let others: Vec<_> = ["Item", "Customer", "Invoice", "Package", "Payment", "Route"]
        .into_iter()
        .map(&mut kind)
        .collect();
    for (i, &other) in others.iter().enumerate() {
        b.connect(order, other);
        if i % 2 == 0 {
            b.connect(other, order);
        }
        if i + 1 < others.len() && i % 3 == 0 {
            b.connect(other, others[i + 1]);
        }
    }
    b.build()
}

pub fn panels() -> Vec<Panel> {
    let mut out = Vec::new();
    let process = order_process();
    let mut draw = |caption: &str, options: LayoutOptions| {
        let drawing = layout(&process, &options).expect("lays out");
        out.push(Panel::new(caption, process.clone(), &options, drawing));
    };
    draw(
        "layered, left to right: the default",
        LayoutOptions::layered_default(),
    );
    draw(
        "process map: top to bottom, orthogonal",
        LayoutOptions::process_map(),
    );
    let mut diagonal = LayoutOptions::process_map();
    diagonal.routing.style = RoutingStyle::Diagonal;
    draw("process map, diagonal routing", diagonal);
    let mut across = LayoutOptions::process_map();
    across.direction = Direction::LeftRight;
    draw("process map, left to right", across);

    let types = type_graph();
    for (caption, options) in [
        ("hub and spoke", LayoutOptions::hub_and_spoke()),
        (
            "hub and spoke with a flow",
            LayoutOptions::hub_and_spoke_flow(),
        ),
    ] {
        let drawing = layout(&types, &options).expect("lays out");
        out.push(Panel::new(caption, types.clone(), &options, drawing));
    }
    out
}

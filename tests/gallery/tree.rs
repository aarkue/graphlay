//! A tidy tree
//!
//! `tree::layout` draws a rooted tree or forest with every parent centred over its children,
//! in the Reingold-Tilford style. Its options are `TreeOptions`, which set the gaps between
//! siblings and subtrees, the direction, and whether an edge runs straight to the child or as an
//! elbow through a shared trunk.

use graphlay::tree::{self, TreeEdgeStyle, TreeOptions};
use graphlay::{Direction, GraphBuilder, LayoutOptions, Node, Size};

use super::Panel;

pub fn panels() -> Vec<Panel> {
    let mut b = GraphBuilder::new();
    let mut unit = |name: &str| {
        b.add_node(Node {
            size: Size::new(92.0, 36.0),
            key: Some(name.to_string()),
            ..Node::default()
        })
    };
    let company = unit("Company");
    let (sales, product, finance) = (unit("Sales"), unit("Product"), unit("Finance"));
    let teams = [
        (sales, "Inbound"),
        (sales, "Accounts"),
        (product, "Design"),
        (product, "Platform"),
        (product, "Mobile"),
        (finance, "Payroll"),
    ];
    let teams: Vec<_> = teams
        .into_iter()
        .map(|(parent, name)| (parent, unit(name)))
        .collect();
    for (parent, team) in teams {
        b.connect(parent, team);
    }
    for child in [sales, product, finance] {
        b.connect(company, child);
    }
    let graph = b.build();

    let elbow = TreeOptions {
        direction: Direction::TopBottom,
        edge_style: TreeEdgeStyle::Elbow,
        ..TreeOptions::default()
    };
    let straight = TreeOptions {
        edge_style: TreeEdgeStyle::Straight,
        ..elbow
    };
    let sideways = TreeOptions {
        direction: Direction::LeftRight,
        ..elbow
    };
    // The metrics check takes layered options. A tree has none, so the default stands in.
    let measured = LayoutOptions::layered_default();
    [
        ("elbow edges, top to bottom", elbow),
        ("straight edges", straight),
        ("left to right", sideways),
    ]
    .into_iter()
    .map(|(caption, options)| {
        let drawing = tree::layout(&graph, &options).expect("lays out");
        Panel::new(caption, graph.clone(), &measured, drawing)
    })
    .collect()
}

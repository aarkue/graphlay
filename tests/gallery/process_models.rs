//! Process models
//!
//! Process models are the main use case. The first three are written by hand: a Petri net of an
//! order process, an object-centric Petri net in which orders and their items move together, and
//! an object-centric directly-follows graph of orders, items and packages. Then a directly-follows
//! graph and a Petri net discovered from a real event log, the road traffic fines of an Italian
//! police force. Each is laid out with one preset, sketched with `sketch_scene`, and restyled by
//! editing the `Scene`.

use std::path::Path;

use graphlay::svg::{
    self, sketch_scene, Attrs, BuiltinMarker, Chip, Decoration, EdgeAnchor, EdgeLabel,
    GlyphCluster, GlyphRow, LegendItem, LineCap, MarkerRef, NodeContent, Overlay, Paint, Palette,
    PanelSection, Ring, SceneNode, SketchOptions, Stroke, Swatch, TextRun,
};
use graphlay::{
    fixture, layout, Edge, GraphBuilder, GroupId, Layout, LayoutOptions, Node, NodeId,
    RankConstraint, Shape, Size,
};

use super::Panel;

const INK: &str = "#1f2937";
const MUTED: &str = "#64748b";
/// Object types: name, colour, tint.
const TYPES: [(&str, &str, &str); 3] = [
    ("orders", "#2563eb", "#dbeafe"),
    ("items", "#059669", "#d1fae5"),
    ("packages", "#d97706", "#fef3c7"),
];

/// A count with thousands separators, as a process-mining tool prints it.
fn count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The room an edge label of `text` needs.
fn label_size(text: &str) -> Size {
    Size::new(6.5 * text.len() as f64 + 10.0, 14.0)
}

/// `text` in a chip, at the spot the layout reserved for edge `i`'s label.
fn chip(drawing: &Layout, i: usize, text: String, colour: &str) -> EdgeLabel {
    let at = drawing.edges[i]
        .label_anchor
        .map_or(EdgeAnchor::default(), |p| {
            EdgeAnchor::At(svg::Point::new(p.x, p.y))
        });
    EdgeLabel {
        text: TextRun {
            color: Some(colour.to_string()),
            ..TextRun::new(text).size(10.0).weight(600.0)
        },
        at,
        chip: Some(Chip::default()),
        ..EdgeLabel::default()
    }
}

/// The label the sketch put in a node, if it has one.
fn label(node: &mut SceneNode) -> Option<&mut TextRun> {
    node.content.iter_mut().find_map(|c| match c {
        NodeContent::Text(text) => Some(text),
        _ => None,
    })
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Place,
    Transition,
    Silent,
}

/// A Petri net under construction. Places are circles, transitions boxes sized to their label,
/// silent transitions narrow bars. Places and arcs may carry an object type, and an arc may be
/// variable, consuming or producing any number of objects at once.
#[derive(Default)]
struct Net {
    b: GraphBuilder,
    kinds: Vec<Kind>,
    labels: Vec<String>,
    types: Vec<Option<usize>>,
    arcs: Vec<(Option<usize>, bool)>,
}

impl Net {
    fn add(&mut self, kind: Kind, label: &str, size: Size, ty: Option<usize>) -> NodeId {
        self.kinds.push(kind);
        self.labels.push(label.to_string());
        self.types.push(ty);
        let shape = if kind == Kind::Place {
            Shape::Ellipse
        } else {
            Shape::Box
        };
        self.b.add_node(Node {
            size,
            shape,
            ..Node::default()
        })
    }
    fn place(&mut self, ty: Option<usize>) -> NodeId {
        self.add(Kind::Place, "", Size::new(30.0, 30.0), ty)
    }
    fn transition(&mut self, label: &str) -> NodeId {
        let width = (6.8 * label.len() as f64 + 28.0).clamp(92.0, 230.0);
        self.add(Kind::Transition, label, Size::new(width, 38.0), None)
    }
    fn silent(&mut self) -> NodeId {
        self.add(Kind::Silent, "", Size::new(10.0, 38.0), None)
    }
    fn arc(&mut self, from: NodeId, to: NodeId) {
        self.typed_arc(from, to, None, false);
    }
    fn typed_arc(&mut self, from: NodeId, to: NodeId, ty: Option<usize>, variable: bool) {
        self.b.connect(from, to);
        self.arcs.push((ty, variable));
    }

    /// Lays the net out left to right and draws it: tokens in the initial places, a double ring
    /// on the final ones, object types as colours and variable arcs as double lines.
    fn draw(self, caption: &str) -> Panel {
        let graph = self.b.build();
        let options = LayoutOptions::layered_default();
        let drawing = layout(&graph, &options).expect("lays out");
        let has_in = |i: usize| graph.edges.iter().any(|e| e.target.index() == i);
        let has_out = |i: usize| graph.edges.iter().any(|e| e.source.index() == i);
        let colour = |ty: Option<usize>| ty.map_or(INK, |t| TYPES[t].1);

        let sketch = SketchOptions {
            labels: self.labels.clone(),
            ..SketchOptions::default()
        };
        let mut scene = sketch_scene(&graph, &drawing, &sketch);
        for (i, node) in scene.nodes.iter_mut().enumerate() {
            match self.kinds[i] {
                Kind::Place => {
                    let c = colour(self.types[i]);
                    node.shape = svg::Shape::Circle;
                    node.fill = Some(self.types[i].map_or("#ffffff", |t| TYPES[t].2).to_string());
                    node.stroke = Some(Stroke::new(c, 1.5));
                    if !has_in(i) {
                        let token = GlyphCluster {
                            radius: 5.5,
                            color: Some(c.to_string()),
                            ..GlyphCluster::default()
                        };
                        node.content.push(NodeContent::Glyphs(GlyphRow {
                            clusters: vec![token],
                            ..GlyphRow::default()
                        }));
                    }
                    if !has_out(i) {
                        let ring = Ring {
                            color: c.to_string(),
                            width: 1.5,
                            inset: 3.5,
                        };
                        node.content.push(NodeContent::Rings { rings: vec![ring] });
                    }
                }
                Kind::Transition => {
                    node.shape = svg::Shape::Rect { corner_radius: 2.0 };
                    node.stroke = Some(Stroke::new(INK, 1.5));
                    if let Some(text) = label(node) {
                        text.color = Some(INK.to_string());
                        text.size = Some(11.5);
                    }
                }
                Kind::Silent => {
                    node.shape = svg::Shape::Rect { corner_radius: 0.0 };
                    node.fill = Some(INK.to_string());
                    node.stroke = Some(Stroke::new(INK, 1.6));
                }
            }
        }
        for (edge, &(ty, variable)) in scene.edges.iter_mut().zip(&self.arcs) {
            let c = colour(ty);
            edge.corner_radius = 6.0;
            if variable {
                // A double line: a wide band in the type's colour under a white core.
                edge.decorations.push(Decoration::Band {
                    points: edge.points.clone(),
                    stroke: Stroke {
                        linecap: LineCap::Round,
                        ..Stroke::new(c, 6.5)
                    },
                    corner_radius: 6.0,
                    attrs: Attrs::default(),
                });
                edge.stroke = Stroke::new("#ffffff", 2.5);
                edge.marker_end = MarkerRef::Builtin {
                    marker: BuiltinMarker::Arrow,
                    color: Some(c.to_string()),
                    size: None,
                };
            } else {
                edge.stroke = Stroke::new(c, 1.6);
            }
        }
        if self.types.iter().any(Option::is_some) {
            let items = TYPES
                .iter()
                .enumerate()
                .filter(|(t, _)| self.types.contains(&Some(*t)))
                .map(|(_, (name, c, _))| LegendItem {
                    swatch: Swatch::Color {
                        color: c.to_string(),
                    },
                    text: name.to_string(),
                    ..LegendItem::default()
                })
                .collect();
            scene.overlays.push(Overlay::panel(svg::Panel {
                origin: svg::Point::new(drawing.bounds.max.x + 24.0, drawing.bounds.min.y),
                sections: vec![PanelSection {
                    title: Some("Object types".to_string()),
                    items,
                    ..PanelSection::default()
                }],
                ..svg::Panel::default()
            }));
        }
        Panel::new(caption, graph, &options, drawing).scene(scene, Palette::default())
    }
}

/// An order process: receive the order, check stock and credit in parallel, then ship or
/// reject, and send reminders until the payment arrives.
fn order_to_cash() -> Panel {
    let mut n = Net::default();
    let start = n.place(None);
    let receive = n.transition("Receive order");
    let p1 = n.place(None);
    let split = n.silent();
    let (stock_in, credit_in) = (n.place(None), n.place(None));
    let check_stock = n.transition("Check stock");
    let check_credit = n.transition("Check credit");
    let (stock_ok, credit_ok) = (n.place(None), n.place(None));
    let approve = n.transition("Approve order");
    let reject = n.transition("Reject order");
    let decided = n.place(None);
    let ship = n.transition("Ship goods");
    let shipped = n.place(None);
    let invoice = n.transition("Send invoice");
    let open = n.place(None);
    let remind = n.transition("Send reminder");
    let pay = n.transition("Receive payment");
    let done = n.place(None);
    for (a, b) in [
        (start, receive),
        (receive, p1),
        (p1, split),
        (split, stock_in),
        (split, credit_in),
        (stock_in, check_stock),
        (credit_in, check_credit),
        (check_stock, stock_ok),
        (check_credit, credit_ok),
        (stock_ok, approve),
        (credit_ok, approve),
        (stock_ok, reject),
        (credit_ok, reject),
        (approve, decided),
        (decided, ship),
        (ship, shipped),
        (shipped, invoice),
        (invoice, open),
        (open, remind),
        (remind, open),
        (open, pay),
        (pay, done),
        (reject, done),
    ] {
        n.arc(a, b);
    }
    n.draw("a Petri net: order to cash")
}

/// An object-centric Petri net: an order and its items are placed together, the items are
/// picked one by one, and the order ships with all of them.
fn object_centric_net() -> Panel {
    let (o, i) = (Some(0), Some(1));
    let mut n = Net::default();
    let (o_start, i_start) = (n.place(o), n.place(i));
    let place = n.transition("Place order");
    let (o_placed, i_placed) = (n.place(o), n.place(i));
    let confirm = n.transition("Confirm order");
    let cancel = n.transition("Cancel order");
    let o_confirmed = n.place(o);
    let pick = n.transition("Pick item");
    let i_picked = n.place(i);
    let ship = n.transition("Ship order");
    let (o_end, i_end) = (n.place(o), n.place(i));
    n.typed_arc(o_start, place, o, false);
    n.typed_arc(i_start, place, i, true);
    n.typed_arc(place, o_placed, o, false);
    n.typed_arc(place, i_placed, i, true);
    n.typed_arc(o_placed, confirm, o, false);
    n.typed_arc(confirm, o_confirmed, o, false);
    n.typed_arc(o_placed, cancel, o, false);
    n.typed_arc(cancel, o_end, o, false);
    n.typed_arc(i_placed, pick, i, false);
    n.typed_arc(pick, i_picked, i, false);
    n.typed_arc(o_confirmed, ship, o, false);
    n.typed_arc(i_picked, ship, i, true);
    n.typed_arc(ship, o_end, o, false);
    n.typed_arc(ship, i_end, i, true);
    n.draw("an object-centric Petri net")
}

/// An object-centric directly-follows graph of an online shop: what happens to orders, to their
/// items and to the packages they ship in, each object type in its own colour, with how often
/// each step occurred.
fn object_centric_dfg() -> Panel {
    const ACTIVITIES: [(&str, usize, u64); 10] = [
        ("place order", 0, 2000),
        ("confirm order", 0, 1950),
        ("payment reminder", 0, 550),
        ("pay order", 0, 1930),
        ("item out of stock", 1, 300),
        ("pick item", 1, 7390),
        ("create package", 2, 2400),
        ("send package", 2, 2400),
        ("failed delivery", 2, 170),
        ("package delivered", 2, 2390),
    ];
    let mut b = GraphBuilder::new();
    let activity: Vec<NodeId> = ACTIVITIES
        .iter()
        .map(|(name, ty, _)| {
            b.add_node(Node {
                size: Size::new((7.0 * name.len() as f64 + 30.0).max(120.0), 46.0),
                group: Some(GroupId(*ty as u32)),
                ..Node::default()
            })
        })
        .collect();
    let mut ends = |rank: RankConstraint, size: Size| -> Vec<NodeId> {
        (0..TYPES.len())
            .map(|ty| {
                b.add_node(Node {
                    size,
                    rank,
                    group: Some(GroupId(ty as u32)),
                    ..Node::default()
                })
            })
            .collect()
    };
    let start = ends(RankConstraint::First, Size::new(92.0, 30.0));
    let end = ends(RankConstraint::Last, Size::new(24.0, 24.0));
    let a = |name: &str| activity[ACTIVITIES.iter().position(|x| x.0 == name).expect("known")];
    let flows: Vec<(NodeId, NodeId, usize, u64)> = vec![
        (start[0], a("place order"), 0, 2000),
        (a("place order"), a("confirm order"), 0, 1950),
        (a("confirm order"), a("pay order"), 0, 1520),
        (a("confirm order"), a("payment reminder"), 0, 430),
        (a("payment reminder"), a("payment reminder"), 0, 120),
        (a("payment reminder"), a("pay order"), 0, 410),
        (a("pay order"), end[0], 0, 1930),
        (start[1], a("place order"), 1, 7600),
        (a("place order"), a("confirm order"), 1, 7400),
        (a("confirm order"), a("pick item"), 1, 7100),
        (a("confirm order"), a("item out of stock"), 1, 300),
        (a("item out of stock"), a("pick item"), 1, 290),
        (a("pick item"), a("create package"), 1, 7300),
        (a("create package"), end[1], 1, 7300),
        (start[2], a("create package"), 2, 2400),
        (a("create package"), a("send package"), 2, 2400),
        (a("send package"), a("package delivered"), 2, 2230),
        (a("send package"), a("failed delivery"), 2, 170),
        (a("failed delivery"), a("package delivered"), 2, 160),
        (a("package delivered"), end[2], 2, 2390),
    ];
    for &(s, t, ty, n) in &flows {
        let mut e = Edge::new(s, t);
        e.group = Some(GroupId(ty as u32));
        e.weight = 1.0 + (n as f64).ln();
        e.thickness = 1.4 + 3.0 * (n as f64).ln() / 7600f64.ln();
        e.label_size = Some(label_size(&count(n)));
        b.add_edge(e);
    }
    let graph = b.build();
    let options = LayoutOptions::process_map();
    let drawing = layout(&graph, &options).expect("lays out");

    let mut scene = sketch_scene(&graph, &drawing, &SketchOptions::default());
    for (i, node) in scene.nodes.iter_mut().enumerate() {
        if let Some(&(name, _, n)) = ACTIVITIES.get(i) {
            node.stroke = Some(Stroke::new(INK, 1.6));
            node.content = vec![
                NodeContent::Text(TextRun {
                    color: Some(INK.to_string()),
                    ..TextRun::new(name).at(0.0, -7.0).weight(600.0)
                }),
                NodeContent::Text(TextRun {
                    color: Some(MUTED.to_string()),
                    ..TextRun::new(count(n)).at(0.0, 9.0).size(10.5).weight(500.0)
                }),
            ];
        } else if let Some(&(name, c, _)) = TYPES.get(i - ACTIVITIES.len()) {
            node.shape = svg::Shape::Rect {
                corner_radius: 15.0,
            };
            node.fill = Some(c.to_string());
            node.stroke = Some(Stroke::new(c, 1.6));
            node.content = vec![NodeContent::Text(TextRun {
                color: Some("#ffffff".to_string()),
                ..TextRun::new(name).weight(600.0)
            })];
        } else {
            let c = TYPES[i - ACTIVITIES.len() - TYPES.len()].1;
            node.shape = svg::Shape::Circle;
            node.stroke = Some(Stroke::new(c, 4.0));
        }
    }
    for (i, edge) in scene.edges.iter_mut().enumerate() {
        let (_, _, ty, n) = flows[i];
        let c = TYPES[ty].1;
        edge.stroke.paint = Paint::color(c);
        edge.labels.push(chip(&drawing, i, count(n), c));
    }
    Panel::new(
        "an object-centric directly-follows graph",
        graph,
        &options,
        drawing,
    )
    .scene(scene, Palette::default())
}

/// The road traffic fines log as a directly-follows graph, its most frequent quarter of the
/// steps: edge width and the count on each edge say how many fines took that step.
fn fines_dfg() -> Panel {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/dfg/dfg-rtfm-top25pct.json");
    let f = fixture::load_file(&path).expect("the fixture loads");
    let (mut graph, options) = f.to_graph();
    let counts: Vec<u64> = f
        .edges
        .iter()
        .map(|e| e.label.as_deref().and_then(|t| t.parse().ok()).unwrap_or(0))
        .collect();
    for (i, n) in graph.nodes.iter_mut().enumerate() {
        if f.nodes[i].label.is_empty() {
            n.size = Size::new(24.0, 24.0);
        }
    }
    let most = counts.iter().copied().max().unwrap_or(1).max(2) as f64;
    for (e, &n) in graph.edges.iter_mut().zip(&counts) {
        e.thickness = 1.2 + 3.6 * (n.max(1) as f64).ln() / most.ln();
        e.label_size = Some(label_size(&count(n)));
    }
    let drawing = layout(&graph, &options).expect("lays out");

    let labels = f.nodes.iter().map(|n| n.label.clone()).collect();
    let sketch = SketchOptions {
        labels,
        ..SketchOptions::default()
    };
    let mut scene = sketch_scene(&graph, &drawing, &sketch);
    for (i, node) in scene.nodes.iter_mut().enumerate() {
        match f.nodes[i].pin.as_deref() {
            Some("first") => {
                node.shape = svg::Shape::Circle;
                node.fill = Some("#16a34a".to_string());
                node.stroke = Some(Stroke::new("#16a34a", 1.6));
            }
            Some("last") => {
                node.shape = svg::Shape::Circle;
                node.stroke = Some(Stroke::new(INK, 4.0));
            }
            _ => {
                node.fill = Some("#f8fafc".to_string());
                node.stroke = Some(Stroke::new("#334155", 1.6));
                if let Some(text) = label(node) {
                    text.color = Some(INK.to_string());
                    text.weight = Some(600.0);
                }
            }
        }
    }
    for (i, edge) in scene.edges.iter_mut().enumerate() {
        let share = (counts[i].max(1) as f64).ln() / most.ln();
        let c = if share > 0.75 { "#1e293b" } else { "#64748b" };
        edge.stroke.paint = Paint::color(c);
        edge.corner_radius = 6.0;
        edge.labels.push(chip(&drawing, i, count(counts[i]), c));
    }
    Panel::new(
        "a directly-follows graph of road traffic fines",
        graph,
        &options,
        drawing,
    )
    .scene(scene, Palette::default())
}

/// The Petri net the inductive miner discovers from the same log.
fn fines_net() -> Panel {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/petri/pn-rtfm-imf02-reference.json");
    let f = fixture::load_file(&path).expect("the fixture loads");
    let mut n = Net::default();
    let ids: Vec<NodeId> = f
        .nodes
        .iter()
        .map(|node| {
            if node.shape.eq_ignore_ascii_case("ellipse") {
                n.place(None)
            } else if node.label.is_empty() {
                n.silent()
            } else {
                n.transition(&node.label)
            }
        })
        .collect();
    for e in &f.edges {
        n.arc(ids[e.from], ids[e.to]);
    }
    n.draw("the Petri net discovered from the same log")
}

pub fn panels() -> Vec<Panel> {
    vec![
        order_to_cash(),
        object_centric_net(),
        object_centric_dfg(),
        fines_dfg(),
        fines_net(),
    ]
}

//! Drawing styles
//!
//! A layout is positions and polylines, and the renderer decides how they look. Every page of
//! this gallery draws with `svg::sketch`, the plain look, and `sketch_scene` hands the same
//! drawing over as a `Scene` to restyle: shapes, fills, token rows, icons, rings, edge colours
//! and widths, gradients, bands, markers, labels, a legend, and a palette for whatever names no
//! colour.

use std::path::Path;

use graphlay::svg::{
    self, sketch_scene, BuiltinMarker, Chip, Decoration, EdgeLabel, GlyphRow, Gradient,
    GradientStop, LegendItem, LineCap, MarkerRef, NodeContent, Overlay, Paint, Palette, Point,
    Ring, SketchOptions, Stroke, Swatch, TextRun, SYMBOL_PLAY_TRIANGLE, SYMBOL_STOP_SQUARE,
    SYMBOL_SWATCH_DOT,
};
use graphlay::{
    fixture, layout, Direction, Edge, Graph, GraphBuilder, Layout, LayoutOptions, LoopSide, Node,
    Shape, Size,
};

use super::Panel;

const BLUE: &str = "#2563eb";
const INK: &str = "#1f2937";

/// `text` on a chip, at the spot the layout reserved for edge `i`'s label.
fn chip(drawing: &Layout, i: usize, text: impl Into<String>, colour: &str) -> EdgeLabel {
    let text = TextRun::new(text).size(10.0).weight(600.0).color(colour);
    EdgeLabel::new(text, drawing.edges[i].label_anchor).chip()
}

/// A legend just right of the drawing.
fn legend(drawing: &Layout, title: &str, items: impl IntoIterator<Item = LegendItem>) -> Overlay {
    let origin = Point::new(drawing.bounds.max.x + 24.0, drawing.bounds.min.y);
    Overlay::panel(svg::Panel::legend(origin, title, items))
}

/// Edge `i` runs along `main` when it joins two of its nodes in order.
fn on_path(g: &Graph, i: usize, main: &[usize]) -> bool {
    let e = &g.edges[i];
    main.windows(2)
        .any(|w| e.source.index() == w[0] && e.target.index() == w[1])
}

/// A Petri net: places and transitions alternate, and the first place holds the token.
fn petri_net() -> Panel {
    let mut b = GraphBuilder::new();
    let mut place = |name: &str| {
        b.add_node(Node {
            size: Size::new(34.0, 34.0),
            shape: Shape::Ellipse,
            key: Some(name.to_string()),
            ..Node::default()
        })
    };
    let (p0, p1, p2, p3, p4) = (
        place("p0"),
        place("p1"),
        place("p2"),
        place("p3"),
        place("p4"),
    );
    let mut transition = |name: &str, width: f64| {
        b.add_node(Node {
            size: Size::new(width, 40.0),
            key: Some(name.to_string()),
            ..Node::default()
        })
    };
    let register = transition("Register", 100.0);
    let check = transition("Check", 100.0);
    let approve = transition("Approve", 100.0);
    let reject = transition("Reject", 100.0);
    let skip = transition("", 14.0);
    let notify = transition("Notify", 100.0);
    for (s, t) in [
        (p0, register),
        (register, p1),
        (p1, check),
        (check, p2),
        (p2, approve),
        (p2, reject),
        (approve, p3),
        (reject, p3),
        (p3, skip),
        (p3, notify),
        (skip, p4),
        (notify, p4),
    ] {
        b.connect(s, t);
    }
    let net = b.build();
    let options = LayoutOptions::layered_default();
    let drawing = layout(&net, &options).expect("lays out");

    let mut scene = sketch_scene(&net, &drawing, &SketchOptions::default());
    for (i, node) in scene.nodes.iter_mut().enumerate() {
        if net.nodes[i].shape == Shape::Ellipse {
            node.shape = svg::Shape::Circle;
            node.content.clear();
            if i == 0 {
                node.content
                    .push(NodeContent::Glyphs(GlyphRow::dots(1, 5.5, INK)));
            }
        } else {
            node.fill = Some(INK.to_string());
            node.stroke = Some(Stroke::new(INK, 1.6));
            if let Some(text) = node.label_mut() {
                text.color = Some("#ffffff".to_string());
            }
        }
    }
    let place = Swatch::Symbol {
        id: SYMBOL_SWATCH_DOT.to_string(),
        size: svg::Size::new(10.0, 10.0),
        color: None,
    };
    let items = [
        LegendItem::new(place, "place"),
        LegendItem::new(Swatch::color(INK), "transition"),
    ];
    scene.overlays.push(legend(&drawing, "Petri net", items));
    Panel::new("a Petri net", net, &options, drawing).scene(scene, Palette::default())
}

/// An approval process between a start and an end event, with case counts on the edges.
fn happy_path() -> Panel {
    let mut b = GraphBuilder::new();
    let mut event = |name: &str| {
        b.add_node(Node {
            size: Size::new(36.0, 36.0),
            shape: Shape::Ellipse,
            key: Some(name.to_string()),
            ..Node::default()
        })
    };
    let (start, end) = (event("start"), event("end"));
    let mut task = |name: &str| {
        b.add_node(Node {
            size: Size::new(104.0, 40.0),
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
    let steps = [
        (start, register, 412),
        (register, check, 412),
        (check, approve, 380),
        (check, reject, 32),
        (approve, notify, 380),
        (reject, notify, 32),
        (notify, end, 412),
    ];
    for (s, t, _) in steps {
        let mut e = Edge::new(s, t);
        e.label_size = Some(Size::new(26.0, 14.0));
        b.add_edge(e);
    }
    let g = b.build();
    let options = LayoutOptions::layered_default();
    let drawing = layout(&g, &options).expect("lays out");
    let main = [0usize, 2, 3, 4, 6, 1];

    let mut scene = sketch_scene(&g, &drawing, &SketchOptions::default());
    for (i, node) in scene.nodes.iter_mut().enumerate() {
        if g.nodes[i].shape == Shape::Ellipse {
            let (colour, icon, size) = if i == start.index() {
                (BLUE, SYMBOL_PLAY_TRIANGLE, svg::Size::new(10.0, 11.6))
            } else {
                (INK, SYMBOL_STOP_SQUARE, svg::Size::new(12.6, 12.6))
            };
            node.shape = svg::Shape::Circle;
            node.fill = Some(colour.to_string());
            node.stroke = Some(Stroke::new(colour, 1.6));
            node.content = vec![NodeContent::symbol(icon, size, "#ffffff")];
        } else if main.contains(&i) {
            node.fill = Some("#dbeafe".to_string());
            node.stroke = Some(Stroke::new(BLUE, 1.75));
            if let Some(text) = node.label_mut() {
                text.color = Some("#1e3a8a".to_string());
                text.weight = Some(600.0);
            }
        } else {
            node.fill = Some("#f8fafc".to_string());
            node.stroke = Some(Stroke::new("#cbd5e1", 1.6));
            if let Some(text) = node.label_mut() {
                text.color = Some("#64748b".to_string());
            }
        }
    }
    for (i, edge) in scene.edges.iter_mut().enumerate() {
        let on = on_path(&g, i, &main);
        edge.stroke = Stroke::new(
            if on { BLUE } else { "#cbd5e1" },
            if on { 3.0 } else { 1.6 },
        );
        if on {
            // A soft band under the path draws it as one stroke of colour.
            let band = Stroke {
                linecap: LineCap::Round,
                opacity: Some(0.45),
                ..Stroke::new("#bfdbfe", 14.0)
            };
            edge.decorations.push(Decoration::band(edge, band));
        }
        let text = if on { "#1e3a8a" } else { "#64748b" };
        edge.labels
            .push(chip(&drawing, i, steps[i].2.to_string(), text));
    }
    Panel::new("the happy path", g, &options, drawing).scene(scene, Palette::default())
}

/// A directly-follows graph of a repair process, with the case count of each step and each
/// activity shown as colour: the busier, the deeper the blue.
fn heat_map() -> Panel {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/dfg/dfg-repairexample-all.json");
    let f = fixture::load_file(&path).expect("the fixture loads");
    let (mut g, mut options) = f.to_graph();
    options.spacing.layer_gap = 84.0;
    for (i, n) in g.nodes.iter_mut().enumerate() {
        if f.nodes[i].label.is_empty() {
            n.size = Size::new(22.0, 22.0);
        }
    }
    for e in &mut g.edges {
        e.label_size = Some(Size::new(34.0, 14.0));
    }
    let drawing = layout(&g, &options).expect("lays out");
    // The fixture stores `1 + ln(count)` as the weight.
    let count = |i: usize| (f.edges[i].weight - 1.0).exp().round() as u64;
    let mut through = vec![0u64; g.nodes.len()];
    for (i, e) in g.edges.iter().enumerate() {
        through[e.target.index()] += count(i);
    }
    let busiest = through.iter().copied().max().unwrap_or(1).max(1);
    let most = (0..g.edges.len()).map(count).max().unwrap_or(1).max(1) as f64;

    // Fill and text colour, from the quietest activity to the busiest.
    const HEAT: [(&str, &str); 6] = [
        ("#eff6ff", "#1e3a8a"),
        ("#dbeafe", "#1e3a8a"),
        ("#bfdbfe", "#1e3a8a"),
        ("#93c5fd", "#1e3a8a"),
        ("#3b82f6", "#ffffff"),
        ("#1d4ed8", "#ffffff"),
    ];
    let labels = f.nodes.iter().map(|n| n.label.clone()).collect();
    let sketch = SketchOptions {
        labels,
        ..SketchOptions::default()
    };
    let mut scene = sketch_scene(&g, &drawing, &sketch);
    for (i, node) in scene.nodes.iter_mut().enumerate() {
        if f.nodes[i].label.is_empty() {
            // The start and end markers of the log.
            node.shape = svg::Shape::Circle;
            node.fill = Some(INK.to_string());
            node.stroke = Some(Stroke::new(INK, 1.6));
            continue;
        }
        let bin = ((through[i] as f64 / busiest as f64) * 5.0).round() as usize;
        let (fill, text) = HEAT[bin.min(5)];
        node.fill = Some(fill.to_string());
        node.stroke = Some(Stroke::new("#1e40af", 1.25));
        if let Some(label) = node.label_mut() {
            label.color = Some(text.to_string());
            label.weight = Some(600.0);
        }
    }
    for (i, edge) in scene.edges.iter_mut().enumerate() {
        let c = count(i);
        let share = (c as f64).ln().max(0.0) / most.ln();
        let colour = if share > 0.66 {
            "#1e3a8a"
        } else if share > 0.33 {
            "#475569"
        } else {
            "#94a3b8"
        };
        edge.stroke = Stroke::new(colour, 1.2 + 3.3 * share);
        edge.labels.push(chip(&drawing, i, c.to_string(), colour));
    }
    let shades = [(5, "every case"), (3, "most cases"), (0, "a few")]
        .map(|(bin, text)| LegendItem::new(Swatch::color(HEAT[bin].0), text));
    scene.overlays.push(legend(&drawing, "Cases", shades));
    Panel::new("frequencies as heat", g, &options, drawing).scene(scene, Palette::default())
}

/// A finite automaton: circles for states, a double ring for the accepting ones, self-loops
/// for the transitions that stay, and the symbol read on every edge.
fn automaton() -> Panel {
    let mut b = GraphBuilder::new();
    let mut state = |name: &str| {
        b.add_node(Node {
            size: Size::new(44.0, 44.0),
            shape: Shape::Ellipse,
            key: Some(name.to_string()),
            ..Node::default()
        })
    };
    let q: Vec<_> = (0..5).map(|i| state(&format!("q{i}"))).collect();
    // The arrow into the initial state comes from a point-sized node nothing draws.
    let start = b.add_node(Node {
        size: Size::new(2.0, 2.0),
        ..Node::default()
    });
    let transitions = [
        (start, q[0], ""),
        (q[0], q[1], "a"),
        (q[0], q[0], "b"),
        (q[1], q[1], "a"),
        (q[1], q[2], "b"),
        (q[2], q[3], "a"),
        (q[2], q[0], "b"),
        (q[3], q[4], "b"),
        (q[3], q[3], "a"),
        (q[4], q[4], "a, b"),
    ];
    for (s, t, symbol) in transitions {
        let mut e = Edge::new(s, t);
        if !symbol.is_empty() {
            e.label_size = Some(Size::new(8.0 * symbol.len() as f64 + 8.0, 14.0));
        }
        b.add_edge(e);
    }
    let g = b.build();
    let mut options = LayoutOptions::layered_default();
    options.self_loops.side = LoopSide::CrossNegative;
    let drawing = layout(&g, &options).expect("lays out");
    let accepting = [q[2].index(), q[4].index()];

    let mut scene = sketch_scene(&g, &drawing, &SketchOptions::default());
    for (i, node) in scene.nodes.iter_mut().enumerate() {
        if i == start.index() {
            node.fill = Some("none".to_string());
            node.stroke = Some(Stroke::new("none", 1.6));
            continue;
        }
        node.shape = svg::Shape::Circle;
        node.stroke = Some(Stroke::new(INK, 1.75));
        if accepting.contains(&i) {
            let ring = Ring {
                color: INK.to_string(),
                width: 1.5,
                inset: 4.0,
            };
            node.content.push(NodeContent::Rings { rings: vec![ring] });
        }
    }
    for (i, edge) in scene.edges.iter_mut().enumerate() {
        edge.stroke = Stroke::new(INK, 1.6);
        let symbol = transitions[i].2;
        if !symbol.is_empty() {
            edge.labels.push(chip(&drawing, i, symbol, INK));
        }
    }
    Panel::new("a finite automaton", g, &options, drawing).scene(scene, Palette::default())
}

/// A service map on a dark background: clients, services and stores in their own colours,
/// edge width by request rate, a gradient where a call crosses tiers, and latency chips.
fn service_map() -> Panel {
    const TIER: [&str; 3] = ["#38bdf8", "#a78bfa", "#34d399"];
    let services = [
        ("Web app", 0),
        ("Mobile API", 0),
        ("Auth", 1),
        ("Orders", 1),
        ("Catalog", 1),
        ("Payments", 1),
        ("Notify", 1),
        ("Users DB", 2),
        ("Orders DB", 2),
        ("Cache", 2),
        ("Queue", 2),
    ];
    let mut b = GraphBuilder::new();
    let ids: Vec<_> = services
        .iter()
        .map(|(name, tier)| {
            b.add_node(Node {
                size: Size::new(if *tier == 2 { 96.0 } else { 104.0 }, 40.0),
                shape: if *tier == 2 {
                    Shape::Ellipse
                } else {
                    Shape::Box
                },
                key: Some(name.to_string()),
                ..Node::default()
            })
        })
        .collect();
    let calls = [
        (0, 2, 120.0, "8 ms"),
        (0, 3, 300.0, "24 ms"),
        (0, 4, 900.0, "5 ms"),
        (1, 2, 200.0, "9 ms"),
        (1, 3, 150.0, "26 ms"),
        (3, 5, 140.0, "41 ms"),
        (3, 4, 300.0, "4 ms"),
        (3, 6, 100.0, "2 ms"),
        (5, 6, 90.0, "3 ms"),
        (2, 7, 320.0, "1 ms"),
        (3, 8, 450.0, "3 ms"),
        (4, 9, 1200.0, "<1 ms"),
        (4, 8, 60.0, "6 ms"),
        (6, 10, 190.0, "1 ms"),
        (5, 8, 140.0, "2 ms"),
    ];
    for (s, t, _, _) in calls {
        let mut e = Edge::new(ids[s], ids[t]);
        e.label_size = Some(Size::new(38.0, 14.0));
        b.add_edge(e);
    }
    let g = b.build();
    let mut options = LayoutOptions::layered_default();
    options.direction = Direction::LeftRight;
    options.spacing.layer_gap = 120.0;
    options.spacing.node_gap = 40.0;
    let drawing = layout(&g, &options).expect("lays out");

    let mut scene = sketch_scene(&g, &drawing, &SketchOptions::default());
    for (i, node) in scene.nodes.iter_mut().enumerate() {
        node.fill = Some("#1e293b".to_string());
        node.stroke = Some(Stroke::new(TIER[services[i].1], 1.75));
        if let Some(text) = node.label_mut() {
            text.color = Some("#e2e8f0".to_string());
            text.weight = Some(600.0);
        }
    }
    for (i, edge) in scene.edges.iter_mut().enumerate() {
        let (s, t, rate, latency): (usize, usize, f64, &str) = calls[i];
        let (from, to) = (TIER[services[s].1], TIER[services[t].1]);
        edge.stroke = Stroke::new(from, 1.2 + 2.6 * rate.ln() / 1200f64.ln());
        if from != to {
            let stop = |offset, colour: &str| GradientStop {
                offset,
                color: colour.to_string(),
            };
            edge.stroke.paint = Paint::Gradient(Gradient {
                from: edge.points[0],
                to: edge.points[edge.points.len() - 1],
                stops: vec![stop(0.0, from), stop(1.0, to)],
            });
            edge.marker_end = MarkerRef::builtin(BuiltinMarker::Arrow).colored(to);
        }
        edge.labels.push(EdgeLabel {
            chip: Some(Chip {
                fill: "#0f172a".to_string(),
                ..Chip::default()
            }),
            ..chip(&drawing, i, latency, "#cbd5e1")
        });
    }
    let tiers = ["clients", "services", "stores"]
        .into_iter()
        .zip(TIER)
        .map(|(text, colour)| LegendItem::new(Swatch::color(colour), text));
    scene.overlays.push(legend(&drawing, "Tiers", tiers));
    scene.background = Some("#0b1220".to_string());
    // The legend's text takes the palette's text colour.
    let palette = Palette {
        node_text: "#e2e8f0".to_string(),
        ..Palette::default()
    };
    Panel::new("a service map", g, &options, drawing).scene(scene, palette)
}

pub fn panels() -> Vec<Panel> {
    vec![
        petri_net(),
        happy_path(),
        heat_map(),
        automaton(),
        service_map(),
    ]
}

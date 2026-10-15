//! Port hints (`Edge::source_port` / `Edge::target_port`): a hinted end attaches to the border it
//! names, in output terms and in any drawing direction, and at the given position for an offset
//! hint. Router preferences, the cycle breaker and reversed ranking do not override it.

use graphlay::{
    layout, reroute, Direction, Edge, EdgeId, EdgeRoute, Graph, GraphBuilder, Layout, LayoutError,
    LayoutOptions, Node, NodeId, Placement, Point, PortHint, PortSides, RouteKind, RoutingStyle,
    Shape, Side, Size,
};

const SIDES: [Side; 4] = [Side::Right, Side::Left, Side::Top, Side::Bottom];

/// The share of a border's half-extent a port may use (`ports::PORT_SPAN`). An offset hint's `t`
/// is clamped to it.
const PORT_SPAN: f64 = 0.95;

fn options(direction: Direction) -> LayoutOptions {
    LayoutOptions {
        direction,
        ..LayoutOptions::layered_default()
    }
}

fn node(w: f64, h: f64) -> Node {
    Node {
        size: Size::new(w, h),
        ..Node::default()
    }
}

/// A small flow with neighbours on every border of the middle node: `0 -> 1 -> 2`, a fork from
/// `0` round `1` to `2`, and two neighbours of `1` in its own layer.
fn flow() -> GraphBuilder {
    let mut b = GraphBuilder::new();
    for _ in 0..6 {
        b.add_node(node(120.0, 60.0));
    }
    let n = |i: u32| NodeId(i);
    b.connect(n(0), n(1)); // edge 0: the one the tests hint
    b.connect(n(1), n(2));
    b.connect(n(0), n(3));
    b.connect(n(3), n(2));
    b.connect(n(0), n(4));
    b.connect(n(4), n(5));
    b
}

/// The point on `side` of the node placed at `pl`, `t` of the way along it (output terms: `-1`
/// is the left end of a top/bottom border, the top end of a left/right one).
fn point_on(pl: &Placement, side: Side, t: f64) -> Point {
    let (hw, hh) = (pl.size.width / 2.0, pl.size.height / 2.0);
    let c = pl.center;
    match side {
        Side::Right => Point::new(c.x + hw, c.y + t * hh),
        Side::Left => Point::new(c.x - hw, c.y + t * hh),
        Side::Top => Point::new(c.x + t * hw, c.y - hh),
        Side::Bottom => Point::new(c.x + t * hw, c.y + hh),
    }
}

/// Where `p` is along `side`, normalised by the border's half-extent, and how far off the border
/// line it is.
fn along(pl: &Placement, side: Side, p: Point) -> (f64, f64) {
    let (hw, hh) = (pl.size.width / 2.0, pl.size.height / 2.0);
    let c = pl.center;
    match side {
        Side::Right => ((p.y - c.y) / hh, (p.x - (c.x + hw)).abs()),
        Side::Left => ((p.y - c.y) / hh, (p.x - (c.x - hw)).abs()),
        Side::Top => ((p.x - c.x) / hw, (p.y - (c.y - hh)).abs()),
        Side::Bottom => ((p.x - c.x) / hw, (p.y - (c.y + hh)).abs()),
    }
}

/// The two ends of a route: `(point, the next point in)` at the source and at the target.
fn ends(r: &EdgeRoute) -> [(Point, Point); 2] {
    let p = &r.points;
    let n = p.len();
    [(p[0], p[1]), (p[n - 1], p[n - 2])]
}

/// Whether the end `p` (with `q` the next point of the route) honours `hint` on a box node. It
/// must lie on the border, leave it outwards, sit at the offset for an offset hint, and be
/// reported on that side.
fn check_end(
    l: &Layout,
    edge: usize,
    at_source: bool,
    hint: PortHint,
    orthogonal: bool,
) -> Result<(), String> {
    let g_end = if at_source { "source" } else { "target" };
    let r = &l.edges[edge];
    let [(ps, qs), (pt, qt)] = ends(r);
    let (p, q) = if at_source { (ps, qs) } else { (pt, qt) };
    let port = if at_source {
        r.source_port
    } else {
        r.target_port
    };
    let side = hint.side();
    if port.map(|p| p.side) != Some(side) {
        return Err(format!(
            "edge {edge} {g_end}: reported {:?}, hinted {side:?}",
            port.map(|p| p.side)
        ));
    }
    let node = l
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, pl)| {
            let (t, off) = along(pl, side, p);
            off <= 0.5 && t.abs() <= 1.0 + 1e-9
        })
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    if node.is_empty() {
        return Err(format!(
            "edge {edge} {g_end}: {p:?} is on no node's {side:?} border"
        ));
    }
    let pl = &l.nodes[node[0]];
    let (t, _) = along(pl, side, p);
    if let Some(want) = hint.offset() {
        let want = want.clamp(-PORT_SPAN, PORT_SPAN);
        let half = if side.horizontal() {
            pl.size.height
        } else {
            pl.size.width
        } / 2.0;
        if ((t - want) * half).abs() > 0.5 {
            return Err(format!(
                "edge {edge} {g_end}: at {t:.4} of the {side:?} border, hinted {want:.4}"
            ));
        }
    }
    if orthogonal {
        let (dx, dy) = (q.x - p.x, q.y - p.y);
        let out = match side {
            Side::Right => dx > 0.0 && dy.abs() < 0.5,
            Side::Left => dx < 0.0 && dy.abs() < 0.5,
            Side::Bottom => dy > 0.0 && dx.abs() < 0.5,
            Side::Top => dy < 0.0 && dx.abs() < 0.5,
        };
        if !out {
            return Err(format!(
                "edge {edge} {g_end}: leaves its {side:?} border towards {q:?} from {p:?}"
            ));
        }
    }
    Ok(())
}

/// Whether a segment of `r` passes through the inside of the box at `pl`.
fn cuts(r: &EdgeRoute, pl: &Placement) -> bool {
    let (hw, hh) = (pl.size.width / 2.0 - 1.0, pl.size.height / 2.0 - 1.0);
    let c = pl.center;
    r.points.windows(2).any(|w| {
        (0..=64).any(|k| {
            let f = k as f64 / 64.0;
            let x = w[0].x + (w[1].x - w[0].x) * f;
            let y = w[0].y + (w[1].y - w[0].y) * f;
            (x - c.x).abs() < hw && (y - c.y).abs() < hh
        })
    })
}

/// Every side, at either end, in both directions: the end lands on the hinted border, leaves it
/// outwards, is reported there, and the route does not run back through the node it leaves.
#[test]
fn every_side_is_honoured_at_either_end_in_both_directions() {
    for direction in [Direction::LeftRight, Direction::TopBottom] {
        for at_source in [true, false] {
            for side in SIDES {
                let mut b = flow();
                let e = b.edge_mut(EdgeId(0)).unwrap();
                let hint = PortHint::Side(side);
                if at_source {
                    e.source_port = Some(hint);
                } else {
                    e.target_port = Some(hint);
                }
                let g = b.build();
                let l = layout(&g, &options(direction)).unwrap();
                check_end(&l, 0, at_source, hint, true)
                    .unwrap_or_else(|m| panic!("{direction:?}: {m}"));
                for n in [0usize, 1] {
                    assert!(
                        !cuts(&l.edges[0], &l.nodes[n]),
                        "{direction:?} {side:?} at {}: edge 0 runs through node {n}: {:?}",
                        if at_source { "source" } else { "target" },
                        l.edges[0].points
                    );
                }
            }
        }
    }
}

/// Both ends hinted at once, every pair of sides.
#[test]
fn both_ends_hinted_together() {
    for direction in [Direction::LeftRight, Direction::TopBottom] {
        for s in SIDES {
            for t in SIDES {
                let mut b = flow();
                let e = b.edge_mut(EdgeId(0)).unwrap();
                e.source_port = Some(PortHint::Side(s));
                e.target_port = Some(PortHint::Side(t));
                let g = b.build();
                let l = layout(&g, &options(direction)).unwrap();
                for (end, hint) in [(true, s), (false, t)] {
                    check_end(&l, 0, end, PortHint::Side(hint), true)
                        .unwrap_or_else(|m| panic!("{direction:?} {s:?}->{t:?}: {m}"));
                }
            }
        }
    }
}

/// An offset hint puts the end where it says along the border (`-1` is the left or top end),
/// clamped only to the share of the border a port may use.
#[test]
fn offsets_are_exact_and_in_output_terms() {
    for direction in [Direction::LeftRight, Direction::TopBottom] {
        for side in SIDES {
            for t in [-1.0, -0.6, 0.0, 0.35, 1.0] {
                let mut b = flow();
                let hint = PortHint::Offset { side, t };
                let e = b.edge_mut(EdgeId(0)).unwrap();
                e.source_port = Some(hint);
                e.target_port = Some(hint);
                let g = b.build();
                let l = layout(&g, &options(direction)).unwrap();
                for end in [true, false] {
                    check_end(&l, 0, end, hint, true)
                        .unwrap_or_else(|m| panic!("{direction:?} t={t}: {m}"));
                }
            }
        }
    }
    // In a top-to-bottom drawing, `-1` on the top border is its left end.
    let mut b = flow();
    b.edge_mut(EdgeId(0)).unwrap().target_port = Some(PortHint::Offset {
        side: Side::Top,
        t: -0.5,
    });
    let g = b.build();
    let l = layout(&g, &options(Direction::TopBottom)).unwrap();
    let end = *l.edges[0].points.last().unwrap();
    let want = point_on(&l.nodes[1], Side::Top, -0.5);
    assert!(
        (end.x - want.x).abs() <= 0.5 && (end.y - want.y).abs() <= 0.5,
        "{end:?} vs {want:?}"
    );
}

/// Several hinted ends on one border keep their offsets, and the free ends there are spread
/// around them.
#[test]
fn several_offsets_on_one_border_keep_their_places() {
    let mut b = GraphBuilder::new();
    let hub = b.add_node(node(80.0, 160.0));
    let leaves: Vec<NodeId> = (0..5).map(|_| b.add_node(node(80.0, 40.0))).collect();
    let ts = [-0.8, 0.1, 0.7];
    for (i, &leaf) in leaves.iter().enumerate() {
        let e = b.connect(hub, leaf);
        if let Some(&t) = ts.get(i) {
            b.edge_mut(e).unwrap().source_port = Some(PortHint::Offset {
                side: Side::Right,
                t,
            });
        }
    }
    let g = b.build();
    let l = layout(&g, &options(Direction::LeftRight)).unwrap();
    for (i, &t) in ts.iter().enumerate() {
        check_end(
            &l,
            i,
            true,
            PortHint::Offset {
                side: Side::Right,
                t,
            },
            true,
        )
        .unwrap();
    }
    let starts: Vec<Point> = l.edges.iter().map(|e| e.points[0]).collect();
    for (i, p) in starts.iter().enumerate() {
        for q in &starts[i + 1..] {
            assert!(
                (p.x - q.x).hypot(p.y - q.y) > 1.0,
                "two ports share a point: {starts:?}"
            );
        }
    }
}

/// A hint beats the drawing's port-side preference and the node's own.
#[test]
fn a_hint_beats_every_port_side_preference() {
    for sides in [PortSides::FacingOnly, PortSides::FacingPreferred] {
        let mut b = flow();
        b.node_mut(NodeId(1)).unwrap().port_sides = Some(PortSides::FacingOnly);
        b.edge_mut(EdgeId(0)).unwrap().target_port = Some(PortHint::Side(Side::Bottom));
        let g = b.build();
        let mut o = options(Direction::LeftRight);
        o.routing.port_sides = sides;
        let l = layout(&g, &o).unwrap();
        check_end(&l, 0, false, PortHint::Side(Side::Bottom), true).unwrap();
    }
}

/// The hints on a back edge that the cycle breaker turns round stay with the caller's source and
/// target.
#[test]
fn a_hint_on_an_edge_the_cycle_breaker_reverses_stays_with_its_node() {
    for direction in [Direction::LeftRight, Direction::TopBottom] {
        let mut b = flow();
        // 2 -> 0 closes a cycle with 0 -> 1 -> 2. The breaker turns it round.
        let back = b.add_edge(Edge {
            source_port: Some(PortHint::Offset {
                side: Side::Top,
                t: 0.5,
            }),
            target_port: Some(PortHint::Side(Side::Bottom)),
            ..Edge::new(NodeId(2), NodeId(0))
        });
        let g = b.build();
        let l = layout(&g, &options(direction)).unwrap();
        assert!(l.reversed.contains(&back), "the back edge was not turned");
        let r = &l.edges[back.index()];
        // The route runs source to target, and each end is on its own node's hinted border.
        let [(ps, _), (pt, _)] = ends(r);
        let (s_t, s_off) = along(&l.nodes[2], Side::Top, ps);
        assert!(s_off <= 0.5 && (s_t - 0.5).abs() * 60.0 <= 0.5, "{ps:?}");
        let (_, t_off) = along(&l.nodes[0], Side::Bottom, pt);
        assert!(t_off <= 0.5, "{pt:?} is not on node 0's bottom border");
        check_end(
            &l,
            back.index(),
            true,
            PortHint::Offset {
                side: Side::Top,
                t: 0.5,
            },
            true,
        )
        .unwrap();
        check_end(&l, back.index(), false, PortHint::Side(Side::Bottom), true).unwrap();
    }
}

/// An edge the caller ranks backwards (`reverse_for_ranking`), alone and on a back edge the cycle
/// breaker also turns (the two turns cancel): the hints still belong to the caller's nodes.
#[test]
fn reverse_for_ranking_keeps_the_hints_on_the_callers_nodes() {
    for direction in [Direction::LeftRight, Direction::TopBottom] {
        for close_a_cycle in [false, true] {
            let mut b = flow();
            let (s, t) = if close_a_cycle {
                (NodeId(2), NodeId(0))
            } else {
                (NodeId(5), NodeId(4))
            };
            let src = PortHint::Offset {
                side: Side::Left,
                t: -0.4,
            };
            let tgt = PortHint::Offset {
                side: Side::Bottom,
                t: 0.6,
            };
            let e = b.add_edge(Edge {
                source_port: Some(src),
                target_port: Some(tgt),
                reverse_for_ranking: true,
                ..Edge::new(s, t)
            });
            let g = b.build();
            let l = layout(&g, &options(direction)).unwrap();
            let r = &l.edges[e.index()];
            let [(ps, _), (pt, _)] = ends(r);
            let (st, so) = along(&l.nodes[s.index()], Side::Left, ps);
            let (tt, to) = along(&l.nodes[t.index()], Side::Bottom, pt);
            assert!(
                so <= 0.5 && to <= 0.5,
                "{direction:?} cycle={close_a_cycle}: {ps:?} / {pt:?} not on their nodes"
            );
            let half = |n: NodeId, horizontal: bool| {
                let sz = l.nodes[n.index()].size;
                if horizontal {
                    sz.height / 2.0
                } else {
                    sz.width / 2.0
                }
            };
            assert!(((st + 0.4) * half(s, true)).abs() <= 0.5, "{st}");
            assert!(((tt - 0.6) * half(t, false)).abs() <= 0.5, "{tt}");
            check_end(&l, e.index(), true, src, true).unwrap();
            check_end(&l, e.index(), false, tgt, true).unwrap();
        }
    }
}

/// `reroute` honours the hints over the caller's positions too, including on an edge that the
/// dragged positions turn backwards.
#[test]
fn reroute_honours_the_hints() {
    for direction in [Direction::LeftRight, Direction::TopBottom] {
        let mut b = flow();
        let hint_s = PortHint::Side(Side::Top);
        let hint_t = PortHint::Offset {
            side: Side::Right,
            t: 0.25,
        };
        {
            let e = b.edge_mut(EdgeId(0)).unwrap();
            e.source_port = Some(hint_s);
            e.target_port = Some(hint_t);
        }
        let g = b.build();
        let o = options(direction);
        let first = layout(&g, &o).unwrap();
        let mut seeded = g.clone();
        for (n, p) in seeded.nodes.iter_mut().zip(&first.nodes) {
            n.position = Some(p.center);
        }
        // Dragging node 1 behind node 0 along the flow makes edge 0 run backwards.
        let c0 = first.nodes[0].center;
        let back = match direction {
            Direction::LeftRight => Point::new(c0.x - 400.0, c0.y),
            Direction::TopBottom => Point::new(c0.x, c0.y - 400.0),
        };
        for drag in [None, Some(back)] {
            let mut g2 = seeded.clone();
            if let Some(p) = drag {
                g2.nodes[1].position = Some(p);
                g2.nodes[1].pinned = true;
            }
            let l = reroute(&g2, &o).unwrap();
            check_end(&l, 0, true, hint_s, true)
                .unwrap_or_else(|m| panic!("{direction:?} drag={drag:?}: {m}"));
            check_end(&l, 0, false, hint_t, true)
                .unwrap_or_else(|m| panic!("{direction:?} drag={drag:?}: {m}"));
        }
    }
}

/// Two parallel edges, one hinted: the hinted one is routed on its own and keeps its hint. The
/// other is not drawn as a lane of a bundle with it.
#[test]
fn a_hinted_edge_is_not_bundled_with_its_parallels() {
    let mut b = GraphBuilder::new();
    let a = b.add_node(node(120.0, 120.0));
    let c = b.add_node(node(120.0, 120.0));
    b.connect(a, c);
    b.connect(a, c);
    b.connect(a, c);
    let hint = PortHint::Offset {
        side: Side::Bottom,
        t: -0.3,
    };
    b.edge_mut(EdgeId(1)).unwrap().source_port = Some(hint);
    let g = b.build();
    let mut o = options(Direction::LeftRight);
    o.routing.bundle_parallel = true;
    let l = layout(&g, &o).unwrap();
    assert!(
        !matches!(l.edges[1].kind, RouteKind::Bundled { .. }),
        "{:?}",
        l.edges[1].kind
    );
    // The other two still form a bundle.
    for i in [0, 2] {
        assert!(
            matches!(l.edges[i].kind, RouteKind::Bundled { .. }),
            "edge {i}: {:?}",
            l.edges[i].kind
        );
    }
    check_end(&l, 1, true, hint, true).unwrap();
    // Two parallel edges that both carry a hint are not bundled.
    let mut g2 = g.clone();
    g2.edges.truncate(2);
    g2.edges[0].target_port = Some(PortHint::Side(Side::Top));
    let l = layout(&g2, &o).unwrap();
    for e in &l.edges {
        assert!(!matches!(e.kind, RouteKind::Bundled { .. }), "{:?}", e.kind);
    }
    check_end(&l, 0, false, PortHint::Side(Side::Top), true).unwrap();
    check_end(&l, 1, true, hint, true).unwrap();
}

/// Ellipses: the end is on the outline, on the hinted half, and reported on the hinted side even
/// where the dominant offset of the point would name the neighbouring one.
#[test]
fn an_ellipse_reports_its_hinted_side() {
    for style in [RoutingStyle::Orthogonal, RoutingStyle::Diagonal] {
        let mut b = GraphBuilder::new();
        let a = b.add_node(Node {
            shape: Shape::Ellipse,
            ..node(60.0, 60.0)
        });
        let c = b.add_node(Node {
            shape: Shape::Ellipse,
            ..node(60.0, 60.0)
        });
        let e = b.connect(a, c);
        b.edge_mut(e).unwrap().source_port = Some(PortHint::Offset {
            side: Side::Top,
            t: 0.9,
        });
        let g = b.build();
        let mut o = options(Direction::LeftRight);
        o.routing.style = style;
        let l = layout(&g, &o).unwrap();
        let r = &l.edges[0];
        let p = r.points[0];
        let pl = &l.nodes[0];
        assert_eq!(r.source_port.map(|p| p.side), Some(Side::Top), "{style:?}");
        assert!(p.y < pl.center.y, "{style:?}: {p:?} is not on the top half");
        assert!(
            (p.x - (pl.center.x + 0.9 * 30.0)).abs() <= 0.5,
            "{style:?}: {p:?} is not at the offset"
        );
    }
}

/// The diagonal style puts a hinted end on its border too, at its offset where it has one.
#[test]
fn the_diagonal_style_honours_the_side() {
    for direction in [Direction::LeftRight, Direction::TopBottom] {
        for side in SIDES {
            let mut b = flow();
            let e = b.edge_mut(EdgeId(0)).unwrap();
            e.source_port = Some(PortHint::Side(side));
            e.target_port = Some(PortHint::Offset { side, t: 0.3 });
            let g = b.build();
            let mut o = options(direction);
            o.routing.style = RoutingStyle::Diagonal;
            let l = layout(&g, &o).unwrap();
            check_end(&l, 0, true, PortHint::Side(side), false)
                .unwrap_or_else(|m| panic!("{direction:?}: {m}"));
            check_end(&l, 0, false, PortHint::Offset { side, t: 0.3 }, false)
                .unwrap_or_else(|m| panic!("{direction:?}: {m}"));
        }
    }
}

/// A self-loop honours a side hint: its bump leaves and re-enters the hinted border.
#[test]
fn a_self_loop_takes_its_hinted_side() {
    for direction in [Direction::LeftRight, Direction::TopBottom] {
        for side in SIDES {
            let mut b = flow();
            let lp = b.add_edge(Edge {
                source_port: Some(PortHint::Side(side)),
                ..Edge::new(NodeId(1), NodeId(1))
            });
            let g = b.build();
            let l = layout(&g, &options(direction)).unwrap();
            let r = &l.edges[lp.index()];
            assert_eq!(r.kind, RouteKind::SelfLoop);
            let pl = &l.nodes[1];
            for p in [r.points[0], *r.points.last().unwrap()] {
                let (t, off) = along(pl, side, p);
                assert!(
                    off <= 0.5 && t.abs() <= 1.0,
                    "{direction:?} {side:?}: loop end {p:?} is off the border"
                );
            }
            assert_eq!(r.source_port.map(|p| p.side), Some(side));
            assert_eq!(r.target_port.map(|p| p.side), Some(side));
        }
    }
}

/// An edgeless node's loops are placed with it outside the layered drawing, and take their
/// hinted side there too.
#[test]
fn an_orphan_s_self_loop_takes_its_hinted_side() {
    for side in SIDES {
        let mut b = flow();
        let orphan = b.add_node(node(80.0, 40.0));
        let lp = b.add_edge(Edge {
            target_port: Some(PortHint::Side(side)),
            ..Edge::new(orphan, orphan)
        });
        let g = b.build();
        let l = layout(&g, &options(Direction::TopBottom)).unwrap();
        let r = &l.edges[lp.index()];
        let pl = &l.nodes[orphan.index()];
        for p in [r.points[0], *r.points.last().unwrap()] {
            let (t, off) = along(pl, side, p);
            assert!(off <= 0.5 && t.abs() <= 1.0, "{side:?}: {p:?}");
        }
    }
}

/// The loops of one node all bump out of one border, so hints that disagree are not honoured:
/// the loops are drawn where they would be without them, and report where they are.
#[test]
fn disagreeing_loop_hints_are_ignored() {
    let mut b = flow();
    let plain = layout(&b.clone().build(), &options(Direction::LeftRight)).unwrap();
    for side in [Side::Top, Side::Left] {
        b.add_edge(Edge {
            source_port: Some(PortHint::Side(side)),
            ..Edge::new(NodeId(1), NodeId(1))
        });
    }
    let g = b.build();
    let l = layout(&g, &options(Direction::LeftRight)).unwrap();
    assert_eq!(l.nodes.len(), plain.nodes.len());
    let mut unhinted = g.clone();
    for e in &mut unhinted.edges {
        e.source_port = None;
    }
    assert_eq!(
        l,
        layout(&unhinted, &options(Direction::LeftRight)).unwrap()
    );
}

/// The JSON form round-trips a hint, and validation names the edge and end with a bad offset.
#[test]
fn bad_offsets_are_rejected_with_the_edge_named() {
    let mut g: Graph = flow().build();
    for (t, ok) in [
        (-1.0, true),
        (1.0, true),
        (0.0, true),
        (1.000_001, false),
        (-3.0, false),
        (f64::NAN, false),
        (f64::NEG_INFINITY, false),
    ] {
        g.edges[3].target_port = Some(PortHint::Offset { side: Side::Top, t });
        let got = layout(&g, &LayoutOptions::default());
        if ok {
            assert!(got.is_ok(), "t = {t}: {got:?}");
        } else {
            assert_eq!(
                got.unwrap_err(),
                LayoutError::InvalidPortOffset {
                    edge: EdgeId(3),
                    source: false
                },
                "t = {t}"
            );
        }
    }
    g.edges[3].target_port = None;
    g.edges[1].source_port = Some(PortHint::Offset {
        side: Side::Left,
        t: 2.0,
    });
    let err = layout(&g, &LayoutOptions::default()).unwrap_err();
    assert_eq!(
        err,
        LayoutError::InvalidPortOffset {
            edge: EdgeId(1),
            source: true
        }
    );
    assert_eq!(
        err.to_string(),
        "edge 1 has a source port offset outside -1..=1"
    );
    // `reroute` validates the same way.
    for n in &mut g.nodes {
        n.position = Some(Point::new(0.0, 0.0));
    }
    assert!(matches!(
        reroute(&g, &LayoutOptions::default()),
        Err(LayoutError::InvalidPortOffset { .. })
    ));
}

/// Hints change nothing for the edges that carry none, and nothing at all when no edge does.
#[test]
fn no_hints_means_no_change() {
    let g = flow().build();
    let o = options(Direction::LeftRight);
    let plain = layout(&g, &o).unwrap();
    let mut with_none = g.clone();
    for e in &mut with_none.edges {
        e.source_port = None;
        e.target_port = None;
    }
    assert_eq!(layout(&with_none, &o).unwrap(), plain);
}

/// With two hinted ends side by side on one border, separating runs leaves every segment
/// axis-aligned.
#[test]
fn separating_runs_never_slants_a_hinted_route() {
    let mut b = GraphBuilder::new();
    b.add_node(Node {
        size: Size::new(394.0, 632.0),
        rank: graphlay::RankConstraint::Free,
        ..Node::default()
    });
    b.add_node(Node {
        size: Size::new(60.0, 30.0),
        rank: graphlay::RankConstraint::Free,
        ..Node::default()
    });
    b.add_node(Node {
        size: Size::new(195.0, 326.0),
        rank: graphlay::RankConstraint::Free,
        ..Node::default()
    });
    b.add_node(Node {
        size: Size::new(2.0, 2.0),
        rank: graphlay::RankConstraint::Last,
        ..Node::default()
    });
    b.add_node(Node {
        size: Size::new(2.0, 2.0),
        rank: graphlay::RankConstraint::Last,
        ..Node::default()
    });
    b.add_edge(Edge {
        source_port: Some(PortHint::Offset {
            side: Side::Bottom,
            t: -0.18974358974358974,
        }),
        target_port: Some(PortHint::Offset {
            side: Side::Top,
            t: -0.07614213197969544,
        }),
        ..Edge::new(NodeId(2), NodeId(0))
    });
    b.add_edge(Edge {
        source_port: Some(PortHint::Offset {
            side: Side::Bottom,
            t: -0.6820512820512821,
        }),
        target_port: None,
        ..Edge::new(NodeId(2), NodeId(1))
    });
    b.add_edge(Edge {
        source_port: Some(PortHint::Offset {
            side: Side::Bottom,
            t: -0.07614213197969544,
        }),
        target_port: Some(PortHint::Offset {
            side: Side::Top,
            t: -0.18974358974358974,
        }),
        ..Edge::new(NodeId(0), NodeId(2))
    });
    b.add_edge(Edge {
        source_port: Some(PortHint::Offset {
            side: Side::Bottom,
            t: -0.5634517766497462,
        }),
        target_port: None,
        ..Edge::new(NodeId(0), NodeId(1))
    });
    b.add_edge(Edge {
        source_port: Some(PortHint::Offset {
            side: Side::Bottom,
            t: 0.30256410256410254,
        }),
        target_port: Some(PortHint::Offset {
            side: Side::Top,
            t: 0.0,
        }),
        ..Edge::new(NodeId(2), NodeId(3))
    });
    b.add_edge(Edge {
        source_port: Some(PortHint::Offset {
            side: Side::Bottom,
            t: 0.16751269035532995,
        }),
        target_port: Some(PortHint::Offset {
            side: Side::Top,
            t: 0.0,
        }),
        ..Edge::new(NodeId(0), NodeId(4))
    });
    b.add_edge(Edge {
        source_port: Some(PortHint::Offset {
            side: Side::Bottom,
            t: 0.6548223350253807,
        }),
        target_port: None,
        ..Edge::new(NodeId(0), NodeId(1))
    });
    b.add_edge(Edge {
        source_port: Some(PortHint::Offset {
            side: Side::Bottom,
            t: 0.7948717948717948,
        }),
        target_port: None,
        ..Edge::new(NodeId(2), NodeId(1))
    });
    b.add_edge(Edge {
        source_port: None,
        target_port: Some(PortHint::Offset {
            side: Side::Top,
            t: -0.3197969543147208,
        }),
        ..Edge::new(NodeId(1), NodeId(0))
    });
    b.add_edge(Edge {
        source_port: None,
        target_port: Some(PortHint::Offset {
            side: Side::Top,
            t: 0.30256410256410254,
        }),
        ..Edge::new(NodeId(0), NodeId(2))
    });
    let o = LayoutOptions {
        direction: Direction::TopBottom,
        ..LayoutOptions::default()
    };
    let l = layout(&b.build(), &o).expect("lays out");
    for (i, e) in l.edges.iter().enumerate() {
        for w in e.points.windows(2) {
            assert!(
                // Sub-pixel rounding between a port and its gutter column is not a slant.
                (w[0].x - w[1].x).abs() < 0.75 || (w[0].y - w[1].y).abs() < 0.75,
                "edge {i}: slanted segment {:?} -> {:?}",
                w[0],
                w[1]
            );
        }
    }
}

/// `PortInfo::offset` reports where along its border an end sits: for an offset hint, `t` times
/// the border's half length, on every side and in both directions.
#[test]
fn port_info_reports_the_offset_along_the_border() {
    for direction in [Direction::LeftRight, Direction::TopBottom] {
        for side in [Side::Left, Side::Right, Side::Top, Side::Bottom] {
            let mut g = flow().build();
            g.edges[0].source_port = Some(PortHint::Offset { side, t: 0.5 });
            let l = layout(&g, &options(direction)).expect("lays out");
            let pl = &l.nodes[0];
            let half = match side {
                Side::Left | Side::Right => pl.size.height / 2.0,
                Side::Top | Side::Bottom => pl.size.width / 2.0,
            };
            let port = l.edges[0].source_port.expect("routed");
            assert_eq!(port.side, side, "{direction:?}");
            assert!(
                (port.offset - 0.5 * half).abs() < 0.5,
                "{direction:?} {side:?}: offset {} for half border {half}",
                port.offset
            );
        }
    }
}

/// An offset hint sets where the edge attaches, so placement puts the far end level with that
/// point and the edge runs straight.
#[test]
fn placement_aligns_the_far_end_with_an_offset_hinted_port() {
    for direction in [Direction::LeftRight, Direction::TopBottom] {
        let mut b = GraphBuilder::new();
        let a = b.add_node(node(120.0, 60.0));
        let c = b.add_node(node(120.0, 60.0));
        b.connect(a, c);
        let mut g = b.build();
        let side = if direction == Direction::LeftRight {
            Side::Right
        } else {
            Side::Bottom
        };
        g.edges[0].source_port = Some(PortHint::Offset { side, t: 0.5 });
        let l = layout(&g, &options(direction)).expect("lays out");
        let route = &l.edges[0];
        assert_eq!(route.points.len(), 2, "{direction:?}: {:?}", route.points);
        let port = route.source_port.expect("routed");
        assert!(port.offset > 5.0, "{direction:?}: offset {}", port.offset);
        let (pa, pc) = (&l.nodes[0], &l.nodes[1]);
        let (want, got) = if direction == Direction::LeftRight {
            (pa.center.y + port.offset, pc.center.y)
        } else {
            (pa.center.x + port.offset, pc.center.x)
        };
        assert!(
            (want - got).abs() < 0.5,
            "{direction:?}: far end at {got}, hinted point at {want}"
        );
    }
}

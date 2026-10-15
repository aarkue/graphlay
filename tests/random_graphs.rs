//! Random graphs under many option combinations: the layout contract holds on shapes no fixture
//! covers. A failure names the case number, which reproduces it.
//!
//! Overlapping edge segments are outside the contract, since a few of these graphs draw two edges
//! along one stretch. Set `GRAPHLAY_RANDOM_OVERLAPS=1` to fail on them too.

use graphlay::metrics;
use graphlay::{
    CycleStrategy, Direction, Edge, Graph, GraphBuilder, GroupId, Layout, LayoutOptions, Node,
    NodeId, Placement, Point, PortHint, PortSides, RoutingStyle, SeedMode, Shape, Side, Size,
};

/// A xorshift64* generator, so every case is reproducible.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
    fn between(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * (self.below(1000) as f64 / 1000.0)
    }
}

fn random_graph(rng: &mut Rng) -> Graph {
    let n = 1 + rng.below(36) as usize;
    let groups = rng.below(4);
    let mut b = GraphBuilder::new();
    for _ in 0..n {
        let ellipse = rng.chance(30);
        let w = rng.between(16.0, 160.0);
        let h = if ellipse && rng.chance(70) {
            w
        } else {
            rng.between(16.0, 80.0)
        };
        b.add_node(Node {
            size: Size::new(w, h),
            shape: if ellipse { Shape::Ellipse } else { Shape::Box },
            group: (groups > 1).then(|| GroupId(rng.below(groups) as u32)),
            ..Node::default()
        });
    }
    // Mostly forward edges over a hidden order, so there is a flow to find, plus the rest.
    let m = rng.below((n as u64) * 2 + 1) as usize;
    for _ in 0..m {
        let a = rng.below(n as u64) as u32;
        let c = rng.below(n as u64) as u32;
        let (s, t) = if rng.chance(75) {
            (a.min(c), a.max(c))
        } else {
            (a, c)
        };
        let mut e = Edge::new(NodeId(s), NodeId(t));
        e.weight = rng.between(0.2, 20.0);
        e.thickness = rng.between(1.0, 6.0);
        if rng.chance(20) {
            e.label_size = Some(Size::new(rng.between(10.0, 70.0), rng.between(8.0, 18.0)));
        }
        b.add_edge(e);
    }
    b.build()
}

fn options_for(case: u64, rng: &mut Rng) -> LayoutOptions {
    let mut o = match case % 3 {
        0 => LayoutOptions::default(),
        1 => LayoutOptions::process_map(),
        _ => LayoutOptions::hub_and_spoke(),
    };
    if rng.chance(50) {
        o.direction = Direction::LeftRight;
    }
    if rng.chance(30) {
        o.routing.style = RoutingStyle::Diagonal;
    }
    o.routing.port_sides = match rng.below(4) {
        0 => PortSides::Any,
        1 => PortSides::FacingOnly,
        2 => PortSides::FacingPreferred,
        _ => PortSides::LongEdgesAny,
    };
    if rng.chance(25) {
        o.wrap.viewport_aspect = rng.between(0.5, 3.0);
    }
    if rng.chance(40) {
        o.cycles.strategy = CycleStrategy::FollowFlow;
    }
    o
}

fn check(
    case: u64,
    what: &str,
    graph: &Graph,
    o: &LayoutOptions,
    l: &Layout,
    out: &mut Vec<String>,
) {
    let mut fail = |msg: String| out.push(format!("case {case} ({what}): {msg}"));
    if l.nodes.len() != graph.nodes.len() || l.edges.len() != graph.edges.len() {
        fail(format!(
            "{} nodes / {} edges laid out for {} / {}",
            l.nodes.len(),
            l.edges.len(),
            graph.nodes.len(),
            graph.edges.len()
        ));
        return;
    }
    let finite = l
        .nodes
        .iter()
        .all(|p| p.center.x.is_finite() && p.center.y.is_finite())
        && l.edges
            .iter()
            .flat_map(|e| &e.points)
            .all(|p| p.x.is_finite() && p.y.is_finite());
    if !finite {
        fail("a coordinate is not finite".into());
        return;
    }
    for (i, (e, r)) in graph.edges.iter().zip(&l.edges).enumerate() {
        if e.source != e.target && r.points.len() < 2 {
            fail(format!("edge {i} has {} points", r.points.len()));
        }
    }
    let m = metrics::measure(graph, o, l);
    if m.node_overlaps != 0 {
        fail(format!("{} overlapping node pairs", m.node_overlaps));
    }
    if std::env::var_os("GRAPHLAY_RANDOM_OVERLAPS").is_some() && m.overlaps != 0 {
        fail(format!("{} overlapping edge segments", m.overlaps));
    }
    if m.endpoint_off_outline != 0 {
        fail(format!(
            "{} route ends off their node's outline (worst {:.2}px)",
            m.endpoint_off_outline,
            m.max_endpoint_outline_gap.unwrap_or(0.0)
        ));
    }
}

fn run_case(case: u64, out: &mut Vec<String>) {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15 ^ (case.wrapping_mul(0x1000_0000_01b3) + 1));
    let graph = random_graph(&mut rng);
    let o = options_for(case, &mut rng);
    let first = match graphlay::layout(&graph, &o) {
        Ok(l) => l,
        Err(e) => return out.push(format!("case {case}: layout failed: {e}")),
    };
    check(case, "layout", &graph, &o, &first, out);
    let again = graphlay::layout(&graph, &o).expect("it succeeded once");
    if serde_json::to_string(&first).ok() != serde_json::to_string(&again).ok() {
        out.push(format!("case {case}: two runs differ"));
    }
    let mut seeded = graph.clone();
    for (n, p) in seeded.nodes.iter_mut().zip(&first.nodes) {
        n.position = Some(p.center);
    }
    for (mode, name) in [
        (SeedMode::Placement, "seeded"),
        (SeedMode::Structure, "structure"),
    ] {
        let mut so = o;
        so.seed_mode = mode;
        match graphlay::layout(&seeded, &so) {
            Ok(l) => check(case, name, &graph, &so, &l, out),
            Err(e) => out.push(format!("case {case} ({name}): failed: {e}")),
        }
    }
    if let Err(e) = graphlay::reroute(&seeded, &o) {
        out.push(format!("case {case} (reroute): failed: {e}"));
    }
}

#[test]
fn random_graphs_keep_the_layout_contract() {
    let cases: u64 = std::env::var("GRAPHLAY_RANDOM_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(if cfg!(debug_assertions) { 150 } else { 600 });
    let mut failures = Vec::new();
    for case in 0..cases {
        let mut mine = Vec::new();
        let caught =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_case(case, &mut mine)));
        if caught.is_err() {
            mine.push(format!("case {case}: panicked"));
        }
        failures.extend(mine);
    }
    assert!(
        failures.is_empty(),
        "{} problem(s) over {cases} random cases:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

/// With a key on every node, shuffling the nodes and edges moves nothing.
#[test]
fn keyed_graphs_draw_the_same_in_any_order() {
    let cases: u64 = if cfg!(debug_assertions) { 60 } else { 300 };
    let mut failures = Vec::new();
    for case in 0..cases {
        let mut rng = Rng(0x51_7cc1_b727_220a ^ case.wrapping_mul(0x9e37_79b9));
        let mut graph = random_graph(&mut rng);
        let o = options_for(case, &mut rng);
        for (i, n) in graph.nodes.iter_mut().enumerate() {
            n.key = Some(format!("n{i:03}"));
        }
        // A random permutation of the nodes, and of the edges, with the edges' ends renumbered.
        let n = graph.nodes.len();
        let mut perm: Vec<usize> = (0..n).collect();
        for i in (1..n).rev() {
            perm.swap(i, rng.below(i as u64 + 1) as usize);
        }
        let mut new_of = vec![0u32; n];
        for (k, &old) in perm.iter().enumerate() {
            new_of[old] = k as u32;
        }
        let mut eperm: Vec<usize> = (0..graph.edges.len()).collect();
        for i in (1..eperm.len()).rev() {
            eperm.swap(i, rng.below(i as u64 + 1) as usize);
        }
        let shuffled = Graph {
            nodes: perm.iter().map(|&i| graph.nodes[i].clone()).collect(),
            edges: eperm
                .iter()
                .map(|&i| {
                    let mut e = graph.edges[i].clone();
                    e.source = NodeId(new_of[e.source.0 as usize]);
                    e.target = NodeId(new_of[e.target.0 as usize]);
                    e
                })
                .collect(),
        };
        let (Ok(a), Ok(b)) = (
            graphlay::layout(&graph, &o),
            graphlay::layout(&shuffled, &o),
        ) else {
            failures.push(format!("case {case}: layout failed"));
            continue;
        };
        let moved = (0..n)
            .filter(|&i| a.nodes[i].center != b.nodes[new_of[i] as usize].center)
            .count();
        // Parallel edges with identical attributes may trade routes. Compare the set of routes.
        let mut ra: Vec<String> = a.edges.iter().map(|e| format!("{:?}", e.points)).collect();
        let mut rb: Vec<String> = b.edges.iter().map(|e| format!("{:?}", e.points)).collect();
        ra.sort();
        rb.sort();
        if moved > 0 || ra != rb {
            failures.push(format!(
                "case {case}: {moved} nodes moved, routes equal: {}",
                ra == rb
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "order changed the drawing:\n  {}",
        failures.join("\n  ")
    );
}

/// Hints about a third of the edge ends of `graph` (a random border, half of them at a random
/// offset) and ranks a few edges against their arrow.
fn add_random_hints(graph: &mut Graph, rng: &mut Rng) {
    const SIDES: [Side; 4] = [Side::Right, Side::Left, Side::Top, Side::Bottom];
    for e in &mut graph.edges {
        if rng.chance(15) {
            e.reverse_for_ranking = true;
        }
        for end in [&mut e.source_port, &mut e.target_port] {
            if rng.chance(35) {
                let side = SIDES[rng.below(4) as usize];
                *end = Some(if rng.chance(50) {
                    PortHint::Side(side)
                } else {
                    PortHint::Offset {
                        side,
                        t: rng.between(-1.0, 1.0),
                    }
                });
            }
        }
    }
}

/// Why the hinted end `p` (with `q` the next point of its route) does not honour `hint` on the
/// node at `at`, or `None` when it does. The end must lie on the hinted border within the port
/// span, at the offset for an offset hint, and in orthogonal routing leave the border straight.
fn hint_broken(
    p: Point,
    q: Point,
    at: &Placement,
    shape: Shape,
    hint: PortHint,
    orthogonal: bool,
) -> Option<String> {
    let (hw, hh) = (at.size.width / 2.0, at.size.height / 2.0);
    let (dx, dy) = (p.x - at.center.x, p.y - at.center.y);
    let side = hint.side();
    // Distance out of the border (positive outside), position along it, the half-extent along
    // it, and the offset from the centre towards the border.
    let (out, along, half, normal) = match side {
        Side::Right => (dx - hw, dy, hh, dx),
        Side::Left => (-dx - hw, dy, hh, -dx),
        Side::Bottom => (dy - hh, dx, hw, dy),
        Side::Top => (-dy - hh, dx, hw, -dy),
    };
    let on = match shape {
        Shape::Box => out.abs() <= 0.5,
        // `endpoint_off_outline` checks the outline. This checks the hinted half.
        Shape::Ellipse => normal > 0.0,
    };
    if !on || along.abs() > 0.95 * half + 0.5 {
        return Some(format!(
            "{p:?} is not on the {side:?} border of the node at {:?}",
            at.center
        ));
    }
    if let Some(t) = hint.offset() {
        let want = t.clamp(-0.95, 0.95) * half;
        if (along - want).abs() > 0.5 {
            return Some(format!(
                "{p:?} is {along:.3} along the {side:?} border, hinted {want:.3}"
            ));
        }
    }
    if orthogonal {
        let (ex, ey) = (q.x - p.x, q.y - p.y);
        let straight_out = match side {
            Side::Right => ex > 0.0 && ey.abs() < 0.5,
            Side::Left => ex < 0.0 && ey.abs() < 0.5,
            Side::Bottom => ey > 0.0 && ex.abs() < 0.5,
            Side::Top => ey < 0.0 && ex.abs() < 0.5,
        };
        if !straight_out {
            return Some(format!(
                "leaves its {side:?} border from {p:?} towards {q:?}"
            ));
        }
    }
    None
}

/// Every hinted end of every non-loop edge of `l`, checked by [`hint_broken`], and reported on
/// its hinted side.
fn check_hints(case: u64, what: &str, graph: &Graph, l: &Layout, orthogonal: bool) -> Vec<String> {
    let mut out = Vec::new();
    for (i, (e, r)) in graph.edges.iter().zip(&l.edges).enumerate() {
        if e.source == e.target || r.points.len() < 2 {
            continue;
        }
        let n = r.points.len();
        for (hint, node, p, q, port, end) in [
            (
                e.source_port,
                e.source,
                r.points[0],
                r.points[1],
                r.source_port,
                "source",
            ),
            (
                e.target_port,
                e.target,
                r.points[n - 1],
                r.points[n - 2],
                r.target_port,
                "target",
            ),
        ] {
            let Some(hint) = hint else { continue };
            let (at, shape) = (&l.nodes[node.index()], graph.nodes[node.index()].shape);
            if let Some(why) = hint_broken(p, q, at, shape, hint, orthogonal) {
                out.push(format!("case {case} ({what}): edge {i} {end}: {why}"));
            }
            if port.map(|p| p.side) != Some(hint.side()) {
                out.push(format!(
                    "case {case} ({what}): edge {i} {end}: reported {:?}, hinted {:?}",
                    port.map(|p| p.side),
                    hint.side()
                ));
            }
        }
    }
    out
}

/// Random port hints hold in fresh, seeded, re-routed and diagonal layouts. Wrapped drawings skip
/// the end-direction check, since a run crossing two cuts comes out slanted with or without a hint.
#[test]
fn random_port_hints_are_honoured() {
    let cases: u64 = std::env::var("GRAPHLAY_RANDOM_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(if cfg!(debug_assertions) { 120 } else { 500 });
    let mut failures = Vec::new();
    for case in 0..cases {
        let mut mine = Vec::new();
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut rng = Rng(0x2545_f491_4f6c_dd1d ^ (case.wrapping_mul(0x9e37_79b9) + 7));
            let mut graph = random_graph(&mut rng);
            add_random_hints(&mut graph, &mut rng);
            let mut o = options_for(case, &mut rng);
            let diagonal = o.routing.style == RoutingStyle::Diagonal;
            o.routing.style = RoutingStyle::Orthogonal;
            let straight = o.wrap.viewport_aspect <= 0.0;
            let first = match graphlay::layout(&graph, &o) {
                Ok(l) => l,
                Err(e) => return mine.push(format!("case {case}: layout failed: {e}")),
            };
            check(case, "hinted", &graph, &o, &first, &mut mine);
            mine.extend(check_hints(case, "hinted", &graph, &first, straight));
            let mut seeded = graph.clone();
            for (n, p) in seeded.nodes.iter_mut().zip(&first.nodes) {
                n.position = Some(p.center);
            }
            match graphlay::layout(&seeded, &o) {
                Ok(l) => mine.extend(check_hints(case, "hinted seeded", &graph, &l, straight)),
                Err(e) => mine.push(format!("case {case} (hinted seeded): failed: {e}")),
            }
            match graphlay::reroute(&seeded, &o) {
                Ok(l) => mine.extend(check_hints(case, "hinted reroute", &graph, &l, straight)),
                Err(e) => mine.push(format!("case {case} (hinted reroute): failed: {e}")),
            }
            if diagonal {
                o.routing.style = RoutingStyle::Diagonal;
                match graphlay::layout(&graph, &o) {
                    Ok(l) => {
                        check(case, "hinted diagonal", &graph, &o, &l, &mut mine);
                        mine.extend(check_hints(case, "hinted diagonal", &graph, &l, false));
                    }
                    Err(e) => mine.push(format!("case {case} (hinted diagonal): failed: {e}")),
                }
            }
        }));
        if caught.is_err() {
            mine.push(format!("case {case}: panicked"));
        }
        failures.extend(mine);
    }
    assert!(
        failures.is_empty(),
        "{} problem(s) over {cases} random hinted cases:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

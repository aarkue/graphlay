//! Every fixture under `fixtures/` lays out cleanly: it succeeds, keeps every node, routes every
//! edge that is not a self-loop, and never stacks two nodes.

use std::collections::BTreeSet;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

use graphlay::fixture::{self, Fixture};
use graphlay::metrics;
use graphlay::{tree, Layout, RouteKind};

/// Fixtures known to lay out with overlapping nodes. An entry here is a regression.
const KNOWN_NODE_OVERLAPS: &[&str] = &[];

/// Fixtures above this many nodes are stress fixtures, skipped in a debug build to keep the test
/// fast. Set `GRAPHLAY_STRESS=1` or run `cargo test --release` to include them.
const STRESS_NODE_LIMIT: usize = 500;

/// Whether `f` is skipped as a stress fixture in this build.
fn is_skipped_stress(f: &Fixture) -> bool {
    f.nodes.len() > STRESS_NODE_LIMIT
        && cfg!(debug_assertions)
        && std::env::var_os("GRAPHLAY_STRESS").is_none()
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn load() -> Vec<Fixture> {
    fixture::load_dir(&fixtures_dir()).unwrap_or_else(|e| panic!("loading fixtures: {e}"))
}

/// Lay out `f`, turning a panic into an `Err` so one bad fixture names itself instead of aborting
/// the whole run.
fn layout(f: &Fixture) -> Result<Layout, String> {
    let (graph, options) = f.to_graph();
    let tree_options = f.options.tree.then(|| f.to_tree_options());
    let run = || match &tree_options {
        Some(t) => tree::layout(&graph, t),
        None => graphlay::layout(&graph, &options),
    };
    match catch_unwind(AssertUnwindSafe(run)) {
        Ok(result) => result.map_err(|e| e.to_string()),
        Err(_) => Err("panicked".to_string()),
    }
}

#[test]
fn fixtures_load_and_validate() {
    let fixtures = load();
    assert!(
        !fixtures.is_empty(),
        "no fixtures found under {}",
        fixtures_dir().display()
    );
    let mut names = BTreeSet::new();
    for f in &fixtures {
        f.validate().unwrap_or_else(|e| panic!("{e}"));
        assert!(
            names.insert(f.name.clone()),
            "duplicate fixture name {}",
            f.name
        );
    }
}

#[test]
fn every_fixture_lays_out() {
    let mut failures = Vec::new();
    let mut still_overlapping = BTreeSet::new();

    let mut skipped = Vec::new();
    for f in load() {
        if is_skipped_stress(&f) {
            skipped.push(f.name.clone());
            continue;
        }
        let (graph, options) = f.to_graph();
        let laid_out = f.laid_out_edges().len();
        let layout = match layout(&f) {
            Ok(l) => l,
            Err(e) => {
                failures.push(format!("{}: {e}", f.name));
                continue;
            }
        };
        if layout.nodes.len() != f.nodes.len() {
            failures.push(format!(
                "{}: {} placements for {} nodes",
                f.name,
                layout.nodes.len(),
                f.nodes.len()
            ));
        }
        // Every edge gets a slot, self-loops included. Every edge that is not a self-loop gets a
        // polyline in it.
        if layout.edges.len() != f.edges.len() {
            failures.push(format!(
                "{}: {} routes for {} edges",
                f.name,
                layout.edges.len(),
                f.edges.len()
            ));
        }
        let routed = layout
            .edges
            .iter()
            .filter(|e| e.kind != RouteKind::SelfLoop && e.points.len() >= 2)
            .count();
        if routed != laid_out {
            failures.push(format!(
                "{}: {routed} routed polylines for {laid_out} non-self-loop edges",
                f.name
            ));
        }
        let m = metrics::measure(&graph, &options, &layout);
        // Both endpoints of a routed edge lie on their node's outline, within half a pixel.
        if m.endpoint_off_outline != 0 {
            failures.push(format!(
                "{}: {} route endpoint(s) more than 0.5px off their node outline (worst {:.2}px)",
                f.name,
                m.endpoint_off_outline,
                m.max_endpoint_outline_gap.unwrap_or(0.0)
            ));
        }
        if m.node_overlaps != 0 {
            if KNOWN_NODE_OVERLAPS.contains(&f.name.as_str()) {
                still_overlapping.insert(f.name.clone());
            } else {
                failures.push(format!(
                    "{}: {} overlapping node pairs (node_overlaps must be 0)",
                    f.name, m.node_overlaps
                ));
            }
        }
    }

    for known in KNOWN_NODE_OVERLAPS {
        if !still_overlapping.contains(*known) {
            failures.push(format!(
                "{known}: no longer overlaps (or is missing) - drop it from KNOWN_NODE_OVERLAPS"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "fixtures failed:\n  {}",
        failures.join("\n  ")
    );
    if !skipped.is_empty() {
        println!(
            "skipped {} stress fixture(s) (> {STRESS_NODE_LIMIT} nodes, debug build): {}\n\
             set GRAPHLAY_STRESS=1 or use --release to include them",
            skipped.len(),
            skipped.join(", ")
        );
    }
}

/// Every quality metric stays in its defined range on every fixture, and `metrics::compare` of a
/// drawing against itself reports no movement, reproducibly.
#[test]
fn section_two_metrics_hold_their_definitions_on_every_fixture() {
    let mut failures = Vec::new();
    for f in load() {
        if is_skipped_stress(&f) {
            continue;
        }
        let (graph, options) = f.to_graph();
        let Ok(laid_out) = layout(&f) else { continue };
        let m = metrics::measure(&graph, &options, &laid_out);
        let name = &f.name;
        let mut check = |ok: bool, what: &str| {
            if !ok {
                failures.push(format!("{name}: {what}"));
            }
        };

        check(
            m.label_overlaps == m.label_label_overlaps + m.label_node_overlaps + m.label_edge_hits,
            "label_overlaps is not the sum of its three parts",
        );
        check(
            m.label_count <= m.edge_count,
            "more reserved labels than edges",
        );
        check(
            m.label_count > 0 || (m.label_overlaps == 0 && m.min_label_clearance.is_none()),
            "label collisions or a clearance without a single reserved label",
        );
        check(
            m.weighted_crossings >= 0.0 && m.thickness_weighted_crossings >= 0.0,
            "negative weighted crossings",
        );
        check(
            (m.crossings == 0) == (m.crossing_angle_min.is_none()),
            "crossing angles disagree with the crossing count",
        );
        check(
            m.crossings_below_30deg <= m.crossings,
            "more shallow crossings than crossings",
        );
        if let (Some(lo), Some(mean)) = (m.crossing_angle_min, m.crossing_angle_mean) {
            check(
                (0.0..=90.0).contains(&lo) && (lo - 1e-9..=90.0).contains(&mean),
                "crossing angle out of 0..=90 or a mean below the minimum",
            );
        }
        check(
            m.main_path_len <= m.node_count,
            "main path longer than the graph",
        );
        check(
            (0.0..=1.0).contains(&m.main_path_aligned_fraction),
            "main_path_aligned_fraction outside 0..=1",
        );
        check(m.main_path_deviation >= 0.0, "negative main_path_deviation");
        check(
            (0.0..=1.0).contains(&m.flow_consistency)
                && (0.0..=1.0).contains(&m.flow_consistency_weighted),
            "flow consistency outside 0..=1",
        );
        check(m.parallel_run_length >= 0.0, "negative parallel_run_length");
        check(
            (0.0..=1.0).contains(&m.lane_purity),
            "lane_purity outside 0..=1",
        );
        check(m.whitespace_balance >= 0.0, "negative whitespace_balance");
        check(
            m.path_turn_mean >= 0.0 && m.path_turn_max <= 180.0 + 1e-9,
            "turn angles outside 0..=180",
        );
        // Each step accounts for two of the counted bends.
        check(
            m.short_steps * 2 <= m.bends,
            "more short steps than bends allow",
        );
        check(
            m.staircase_steps * 2 <= m.bends,
            "more staircase steps than bends allow",
        );
        // Each routed edge has two ends. Self-loop ends are not counted.
        check(
            m.perpendicular_ends <= 2 * (m.edge_count - m.unrouted_edges),
            "more perpendicular ends than routed edges have ends",
        );
        // A drawing without groups scores a lane purity of 1.0.
        let grouped = f.nodes.iter().filter(|n| n.category.is_some()).count();
        if grouped == 0 {
            check(
                m.lane_purity == 1.0 && m.lane_intruders == 0,
                "a drawing with no groups was judged on lanes",
            );
        }

        // Compared with itself, nothing moves and the seeded sample is reproducible.
        let common: Vec<(graphlay::NodeId, graphlay::NodeId)> = (0..graph.nodes.len() as u32)
            .map(|i| (graphlay::NodeId(i), graphlay::NodeId(i)))
            .collect();
        let same = metrics::compare(&laid_out, &laid_out, &common);
        check(
            same.hausdorff == 0.0
                && same.relative_euclidean == 0.0
                && same.orthogonal_ordering == 0.0
                && same.edge_shape == 0.0,
            "a drawing compared with itself reports movement",
        );
        check(
            same == metrics::compare(&laid_out, &laid_out, &common),
            "metrics::compare is not deterministic",
        );
    }
    assert!(
        failures.is_empty(),
        "metrics broke their own definitions:\n  {}",
        failures.join("\n  ")
    );
}

/// In the sepsis net, IV Liquid has no room to move between its skip and LacticAcid. The places of
/// its branch then line up with it, and the skip takes the bend.
#[test]
fn a_boxed_in_branch_transition_gets_its_places_on_its_line() {
    let f = fixture::load_file(&fixtures_dir().join("petri/pn-sepsis-imf.json")).expect("loads");
    let (g, o) = f.to_graph();
    let l = graphlay::layout(&g, &o).expect("lays out");
    let liquid = f
        .nodes
        .iter()
        .position(|n| n.label == "IV Liquid")
        .expect("node");
    let ends: Vec<usize> = g
        .edges
        .iter()
        .filter_map(|e| {
            let (s, t) = (e.source.0 as usize, e.target.0 as usize);
            (t == liquid).then_some(s).or((s == liquid).then_some(t))
        })
        .collect();
    assert_eq!(ends.len(), 2);
    let y = |i: usize| l.nodes[i].center.y;
    for p in ends {
        assert!((y(p) - y(liquid)).abs() < 0.5, "{} vs {}", y(p), y(liquid));
    }
}

/// With `PortSides::Any`, a long edge can enter its target from the top or bottom, after a run
/// along the target's layer. Straightening its waypoints must not make that run cross a node.
#[test]
fn a_perpendicular_approach_stays_clear_of_its_layer() {
    let f = fixture::load_file(&fixtures_dir().join("synthetic/cycles-300.json")).expect("loads");
    let (g, mut o) = f.to_graph();
    o.routing.port_sides = graphlay::PortSides::Any;
    let l = graphlay::layout(&g, &o).expect("lays out");
    assert_eq!(metrics::measure(&g, &o, &l).node_edge_hits, 0);
}

/// No edge runs along the border of a node it passes, e.g., the end of a silent transition.
#[test]
fn a_flattened_run_keeps_off_the_borders_it_passes() {
    let f =
        fixture::load_file(&fixtures_dir().join("petri/pn-sepsis-discovered.json")).expect("loads");
    let (g, o) = f.to_graph();
    let l = graphlay::layout(&g, &o).expect("lays out");
    for (e, route) in g.edges.iter().zip(&l.edges) {
        let half = e.thickness / 2.0;
        for w in route.points.windows(2) {
            let (p, q) = (w[0], w[1]);
            let vertical = (p.x - q.x).abs() < 0.01;
            if !vertical && (p.y - q.y).abs() >= 0.01 {
                continue;
            }
            for (i, n) in l.nodes.iter().enumerate() {
                if i == e.source.0 as usize || i == e.target.0 as usize {
                    continue;
                }
                let (hw, hh) = (n.size.width / 2.0, n.size.height / 2.0);
                let (x0, x1, y0, y1) = (
                    n.center.x - hw,
                    n.center.x + hw,
                    n.center.y - hh,
                    n.center.y + hh,
                );
                // Across the run, its position `at` and the node's `lo..hi`. Along it, the run's
                // `a..b` and the node's `s..t`.
                let (at, a, b, lo, hi, s, t) = if vertical {
                    (p.x, p.y.min(q.y), p.y.max(q.y), x0, x1, y0, y1)
                } else {
                    (p.y, p.x.min(q.x), p.x.max(q.x), y0, y1, x0, x1)
                };
                let gap = (lo - at).max(at - hi) - half;
                let shared = b.min(t) - a.max(s);
                assert!(
                    gap < -half || gap >= 1.0 || shared <= 1.0,
                    "edge {:?} runs {gap:.1} px from node {i}'s border",
                    (e.source, e.target)
                );
            }
        }
    }
}

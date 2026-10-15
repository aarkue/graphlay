//! Pins the whole `Layout` of every fixture, coordinates rounded to 0.001 px, so it catches changes
//! that move things without changing a metric. Smaller fixtures also run under option variants.
//!
//! A change meant to alter drawings rewrites `tests/golden.json`:
//!
//! ```text
//! GRAPHLAY_BLESS=1 cargo test --release -p graphlay --features fixtures --test golden
//! ```
//!
//! Stress fixtures only run in release builds, so bless with `--release`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use graphlay::fixture::{self, Fixture};
use graphlay::{tree, CycleStrategy, Graph, Layout, LayoutOptions, SeedMode};

/// Fixtures above this many nodes are only fingerprinted in release builds.
const STRESS_NODE_LIMIT: usize = 500;
/// Fixtures up to this many nodes are also fingerprinted under the option variants.
const VARIANT_NODE_LIMIT: usize = 80;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden.json")
}

fn seeded(graph: &Graph, from: &Layout) -> Graph {
    let mut g = graph.clone();
    for (n, p) in g.nodes.iter_mut().zip(&from.nodes) {
        n.position = Some(p.center);
    }
    g
}

/// The `(key, fingerprint)` pairs one fixture contributes.
fn fingerprints(f: &Fixture) -> Vec<(String, String)> {
    let (graph, options) = f.to_graph();
    let run = |g: &Graph, o: &LayoutOptions| match graphlay::layout(g, o) {
        Ok(l) => l.fingerprint(),
        Err(e) => format!("error: {e}"),
    };
    if f.options.tree {
        let out = match tree::layout(&graph, &f.to_tree_options()) {
            Ok(l) => l.fingerprint(),
            Err(e) => format!("error: {e}"),
        };
        return vec![(f.name.clone(), out)];
    }
    let mut out = Vec::new();
    let base = graphlay::layout(&graph, &options);
    out.push((
        f.name.clone(),
        match &base {
            Ok(l) => l.fingerprint(),
            Err(e) => format!("error: {e}"),
        },
    ));
    if f.nodes.len() > VARIANT_NODE_LIMIT {
        return out;
    }
    let mut wrap = options;
    wrap.wrap.viewport_aspect = 16.0 / 9.0;
    out.push((format!("{}#wrap16:9", f.name), run(&graph, &wrap)));
    let mut flow = options;
    flow.cycles.strategy = CycleStrategy::FollowFlow;
    out.push((format!("{}#follow_flow", f.name), run(&graph, &flow)));
    if let Ok(before) = &base {
        let g = seeded(&graph, before);
        let mut keep = options;
        keep.seed_mode = SeedMode::Structure;
        out.push((format!("{}#structure", f.name), run(&g, &keep)));
        out.push((format!("{}#placement", f.name), run(&g, &options)));
        let rerouted = match graphlay::reroute(&g, &options) {
            Ok(l) => l.fingerprint(),
            Err(e) => format!("error: {e}"),
        };
        out.push((format!("{}#reroute", f.name), rerouted));
    }
    out
}

#[test]
fn every_fixture_draws_exactly_as_pinned() {
    let fixtures = fixture::load_dir(&fixtures_dir()).unwrap_or_else(|e| panic!("{e}"));
    let check_stress = !cfg!(debug_assertions);
    let bless = std::env::var_os("GRAPHLAY_BLESS").is_some();
    let pinned: BTreeMap<String, String> = std::fs::read_to_string(golden_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    let mut now = BTreeMap::new();
    let mut skipped = Vec::new();
    for f in &fixtures {
        if f.nodes.len() > STRESS_NODE_LIMIT && !check_stress {
            skipped.push(f.name.clone());
            continue;
        }
        now.extend(fingerprints(f));
    }

    if bless {
        assert!(
            check_stress,
            "bless in a release build, or the stress fixtures drop out of golden.json"
        );
        let text = serde_json::to_string_pretty(&now).expect("a map serialises");
        std::fs::write(golden_path(), text + "\n").expect("writing golden.json");
        println!("blessed {} fingerprints", now.len());
        return;
    }

    let mut changed = Vec::new();
    for (key, got) in &now {
        match pinned.get(key) {
            Some(want) if want == got => {}
            Some(_) => changed.push(format!("{key}: changed")),
            None => changed.push(format!("{key}: not pinned")),
        }
    }
    for key in pinned.keys() {
        let base = key.split('#').next().unwrap_or(key);
        if !now.contains_key(key) && !skipped.iter().any(|s| s == base) {
            changed.push(format!("{key}: pinned but no longer produced"));
        }
    }
    assert!(
        changed.is_empty(),
        "{} of {} drawings differ from tests/golden.json:\n  {}\n\
         If the change is intended, re-bless (see this file's header) and review the diff.",
        changed.len(),
        now.len(),
        changed.join("\n  ")
    );
}

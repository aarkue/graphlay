//! Deterministic synthetic layout problems, for benchmarking layout changes on shapes that stress
//! the engine in known ways (long chains, dense layers, hubs, cycles, parallel edges, trees).
//!
//! [`synthetic_fixtures`] returns the same graphs on every run and platform. Available with the
//! `fixtures` cargo feature.

use std::collections::BTreeSet;

use crate::fixture::{Fixture, FixtureEdge, FixtureNode, FixtureOptions};

/// How far back [`random_dag`] may reach for a node's connecting parent.
const PARENT_WINDOW: usize = 12;

/// Default node box in px, the size of a typical activity box in a process-model viewer.
const NODE_W: f64 = 120.0;
const NODE_H: f64 = 52.0;

/// A linear congruential generator (Knuth's MMIX constants), to avoid a random-number dependency.
#[derive(Debug, Clone)]
pub struct Lcg {
    state: u64,
}

impl Lcg {
    /// Start a generator from `seed`.
    pub fn new(seed: u64) -> Self {
        Lcg {
            state: seed.wrapping_mul(2_862_933_555_777_941_757).wrapping_add(1),
        }
    }

    /// Next raw 32-bit value (the high bits of the state, which are the well-mixed ones).
    pub fn next_u32(&mut self) -> u32 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.state >> 32) as u32
    }

    /// Uniform value in `0..n` (`0` when `n == 0`).
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            self.next_u32() as usize % n
        }
    }
}

/// A plain 120x52 box node labelled `N<i>`.
fn box_node(i: usize) -> FixtureNode {
    sized_node(i, NODE_W, NODE_H)
}

/// A box node with explicit extents.
fn sized_node(i: usize, width: f64, height: f64) -> FixtureNode {
    FixtureNode {
        id: format!("n{i}"),
        label: format!("N{i}"),
        width,
        height,
        shape: "box".to_string(),
        pin: None,
        category: None,
    }
}

/// A plain edge of weight 1 and stroke width 2.
fn edge(from: usize, to: usize) -> FixtureEdge {
    FixtureEdge {
        from,
        to,
        weight: 1.0,
        thickness: 2.0,
        label: None,
        label_size: None,
        category: None,
        reversed: false,
    }
}

/// Assemble a fixture of kind `synthetic`.
fn fixture(
    name: &str,
    source: &str,
    direction: &str,
    options: FixtureOptions,
    nodes: Vec<FixtureNode>,
    edges: Vec<FixtureEdge>,
) -> Fixture {
    Fixture {
        name: name.to_string(),
        kind: "synthetic".to_string(),
        source: source.to_string(),
        direction: direction.to_string(),
        options,
        nodes,
        edges,
    }
}

/// A path of `n` nodes.
fn chain(name: &str, n: usize, direction: &str) -> Fixture {
    let nodes = (0..n).map(box_node).collect();
    let edges = (0..n.saturating_sub(1)).map(|i| edge(i, i + 1)).collect();
    fixture(
        name,
        &format!("synthetic: chain of {n} nodes"),
        direction,
        FixtureOptions::default(),
        nodes,
        edges,
    )
}

/// `blocks` diamonds chained end to end, each splitting into `width` parallel branches. Tests
/// symmetric placement and branch ordering.
fn diamond_blocks(name: &str, blocks: usize, width: usize) -> Fixture {
    let mut nodes = vec![box_node(0)];
    let mut edges = Vec::new();
    let mut source = 0usize;
    for _ in 0..blocks {
        let first_branch = nodes.len();
        for k in 0..width {
            nodes.push(box_node(first_branch + k));
        }
        let join = nodes.len();
        nodes.push(box_node(join));
        for k in 0..width {
            edges.push(edge(source, first_branch + k));
            edges.push(edge(first_branch + k, join));
        }
        source = join;
    }
    fixture(
        name,
        &format!("synthetic: {blocks} chained diamonds, {width} parallel branches each"),
        "TB",
        FixtureOptions::default(),
        nodes,
        edges,
    )
}

/// A random graph on `n` nodes with about `1.5 * n` edges.
///
/// Every node after the first gets a parent at most [`PARENT_WINDOW`] positions back, which keeps
/// the graph connected. The other edges are random forward skips, `back_percent`% of them reversed.
fn random_dag(name: &str, n: usize, back_percent: usize, direction: &str, seed: u64) -> Fixture {
    random_graph(
        name,
        &format!("synthetic: random graph, n={n}, e~1.5n, {back_percent}% back edges, seed {seed}"),
        n,
        (n as f64 * 1.5).round() as usize,
        back_percent,
        direction,
        seed,
    )
}

/// [`random_dag`] with an explicit edge budget, for densities other than `1.5n`.
fn random_graph(
    name: &str,
    source: &str,
    n: usize,
    target: usize,
    back_percent: usize,
    direction: &str,
    seed: u64,
) -> Fixture {
    let mut rng = Lcg::new(seed);
    let nodes: Vec<FixtureNode> = (0..n).map(box_node).collect();
    let mut edges: Vec<FixtureEdge> = Vec::new();
    for child in 1..n {
        let back = 1 + rng.below(child.min(PARENT_WINDOW));
        edges.push(edge(child - back, child));
    }
    let (mut extras, mut backs) = (0usize, 0usize);
    let mut guard = 0;
    while edges.len() < target && guard < target * 20 {
        guard += 1;
        let a = rng.below(n.saturating_sub(1));
        let reach = (n - 1 - a).min(6);
        if reach == 0 {
            continue;
        }
        let b = a + 1 + rng.below(reach);
        // Spread the back edges evenly over the extras (Bresenham-style) so even the smallest
        // graph gets some cycles.
        extras += 1;
        if backs * 100 < extras * back_percent {
            backs += 1;
            edges.push(edge(b, a));
        } else {
            edges.push(edge(a, b));
        }
    }
    fixture(
        name,
        source,
        direction,
        FixtureOptions::default(),
        nodes,
        edges,
    )
}

/// `layers` ranks of `per_layer` nodes with `target` forward edges spread over the rank gaps.
///
/// Every node past the first rank gets one parent in the previous rank, then random forward edges
/// fill the budget. Stresses crossing minimisation without the edge count of [`layered_grid`].
fn layered_random(
    name: &str,
    layers: usize,
    per_layer: usize,
    target: usize,
    seed: u64,
) -> Fixture {
    let mut rng = Lcg::new(seed);
    let n = layers * per_layer;
    let nodes: Vec<FixtureNode> = (0..n).map(box_node).collect();
    let mut edges: Vec<FixtureEdge> = Vec::new();
    let mut seen: BTreeSet<(usize, usize)> = BTreeSet::new();
    for l in 1..layers {
        for k in 0..per_layer {
            let to = l * per_layer + k;
            let from = (l - 1) * per_layer + rng.below(per_layer);
            if seen.insert((from, to)) {
                edges.push(edge(from, to));
            }
        }
    }
    let mut guard = 0;
    while edges.len() < target && guard < target * 20 {
        guard += 1;
        let l = rng.below(layers.saturating_sub(1));
        let from = l * per_layer + rng.below(per_layer);
        let to = (l + 1) * per_layer + rng.below(per_layer);
        if seen.insert((from, to)) {
            edges.push(edge(from, to));
        }
    }
    fixture(
        name,
        &format!(
            "synthetic: {layers} sparse layers of {per_layer}, e={}, seed {seed}",
            edges.len()
        ),
        "TB",
        FixtureOptions::default(),
        nodes,
        edges,
    )
}

/// One wide centre node with `leaves` neighbours, half feeding in and half fed from it. Uses the
/// `compact` option, which is meant for this shape.
fn hub_and_spoke(name: &str, leaves: usize) -> Fixture {
    let mut nodes = vec![sized_node(0, 160.0, 60.0)];
    let mut edges = Vec::new();
    for i in 0..leaves {
        let leaf = i + 1;
        nodes.push(box_node(leaf));
        if i % 2 == 0 {
            edges.push(edge(leaf, 0));
        } else {
            edges.push(edge(0, leaf));
        }
    }
    fixture(
        name,
        &format!("synthetic: hub with {leaves} spokes (half in, half out)"),
        "TB",
        FixtureOptions {
            compact: true,
            ..FixtureOptions::default()
        },
        nodes,
        edges,
    )
}

/// `layers` ranks of `per_layer` nodes, every node connected to every node of the next rank. A
/// worst case for crossing reduction.
fn layered_grid(name: &str, layers: usize, per_layer: usize) -> Fixture {
    let nodes = (0..layers * per_layer).map(box_node).collect();
    let mut edges = Vec::new();
    for l in 0..layers.saturating_sub(1) {
        for a in 0..per_layer {
            for b in 0..per_layer {
                edges.push(edge(l * per_layer + a, (l + 1) * per_layer + b));
            }
        }
    }
    fixture(
        name,
        &format!(
            "synthetic: {layers} layers of {per_layer}, fully connected between adjacent layers"
        ),
        "TB",
        FixtureOptions::default(),
        nodes,
        edges,
    )
}

/// A chain where every step carries `lanes` parallel edges and every node carries a self-loop.
fn parallel_and_self_loops(name: &str, n: usize, lanes: usize) -> Fixture {
    let nodes = (0..n).map(box_node).collect();
    let mut edges = Vec::new();
    for i in 0..n.saturating_sub(1) {
        for _ in 0..lanes {
            edges.push(edge(i, i + 1));
        }
    }
    for i in 0..n {
        edges.push(edge(i, i));
    }
    fixture(
        name,
        &format!("synthetic: chain of {n} with {lanes} parallel edges per step plus a self-loop on every node"),
        "TB",
        FixtureOptions::default(),
        nodes,
        edges,
    )
}

/// One source fanning out to `sinks` leaves, two ranks deep.
fn wide_shallow(name: &str, sinks: usize) -> Fixture {
    let mut nodes = vec![box_node(0)];
    let mut edges = Vec::new();
    for i in 0..sinks {
        nodes.push(box_node(i + 1));
        edges.push(edge(0, i + 1));
    }
    fixture(
        name,
        &format!("synthetic: one source fanning out to {sinks} sinks"),
        "TB",
        FixtureOptions::default(),
        nodes,
        edges,
    )
}

/// A rooted tree of `n` nodes, at most `max_children` per node, laid out in tidy-tree mode.
fn tree(name: &str, n: usize, max_children: usize, seed: u64) -> Fixture {
    let mut rng = Lcg::new(seed);
    let nodes: Vec<FixtureNode> = (0..n).map(box_node).collect();
    let mut children = vec![0usize; n];
    let mut edges = Vec::new();
    for child in 1..n {
        // Prefer recent parents so the tree grows deep instead of into one huge fan.
        let mut parent = None;
        for _ in 0..16 {
            let candidate = rng.below(child);
            if children[candidate] < max_children {
                parent = Some(candidate);
                break;
            }
        }
        let parent = parent.unwrap_or_else(|| {
            (0..child)
                .find(|&i| children[i] < max_children)
                .unwrap_or(child - 1)
        });
        children[parent] += 1;
        edges.push(edge(parent, child));
    }
    fixture(
        name,
        &format!("synthetic: rooted tree, n={n}, <= {max_children} children per node, seed {seed}"),
        "TB",
        FixtureOptions {
            tree: true,
            ..FixtureOptions::default()
        },
        nodes,
        edges,
    )
}

/// The full synthetic suite, in a fixed order. Every call returns identical graphs.
pub fn synthetic_fixtures() -> Vec<Fixture> {
    vec![
        chain("chain-10", 10, "TB"),
        chain("chain-10-lr", 10, "LR"),
        diamond_blocks("diamond-blocks", 4, 3),
        random_dag("dag-20", 20, 10, "TB", 20),
        random_dag("dag-50", 50, 10, "TB", 50),
        random_dag("dag-50-lr", 50, 10, "LR", 50),
        random_dag("dag-100", 100, 10, "TB", 100),
        random_dag("dag-200", 200, 10, "TB", 200),
        random_dag("dag-400", 400, 10, "TB", 400),
        random_dag("dag-1000", 1000, 10, "TB", 1000),
        random_dag("dag-2000", 2000, 10, "TB", 2000),
        layered_random("layered-wide-200x8", 8, 25, 600, 200),
        random_graph(
            "cycles-300",
            "synthetic: random graph, n=300, e=500, 30% back edges, seed 300",
            300,
            500,
            30,
            "TB",
            300,
        ),
        hub_and_spoke("hub-spoke-40", 40),
        layered_grid("grid-6x6", 6, 6),
        parallel_and_self_loops("parallel-selfloops", 8, 3),
        wide_shallow("wide-shallow-30", 30),
        tree("tree-60", 60, 3, 60),
    ]
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn all_fixtures_are_valid_and_named_uniquely() {
        let fixtures = synthetic_fixtures();
        let mut names = BTreeSet::new();
        for f in &fixtures {
            f.validate().unwrap_or_else(|e| panic!("{e}"));
            assert!(names.insert(f.name.clone()), "duplicate name {}", f.name);
            assert_eq!(f.kind, "synthetic");
            assert!(!f.nodes.is_empty());
        }
        assert_eq!(fixtures.len(), 18);
    }

    #[test]
    fn generation_is_deterministic() {
        assert_eq!(synthetic_fixtures(), synthetic_fixtures());
    }

    #[test]
    fn shapes_are_what_they_claim() {
        let by_name = |name: &str| {
            synthetic_fixtures()
                .into_iter()
                .find(|f| f.name == name)
                .expect("fixture exists")
        };
        let chain = by_name("chain-10");
        assert_eq!(chain.nodes.len(), 10);
        assert_eq!(chain.edges.len(), 9);
        assert_eq!(by_name("chain-10-lr").direction, "LR");

        let grid = by_name("grid-6x6");
        assert_eq!(grid.nodes.len(), 36);
        assert_eq!(grid.edges.len(), 5 * 36);

        let hub = by_name("hub-spoke-40");
        assert_eq!(hub.nodes.len(), 41);
        assert_eq!(hub.edges.len(), 40);
        assert!(hub.options.compact);

        let loops = by_name("parallel-selfloops");
        assert_eq!(loops.self_loop_edges().len(), 8);
        assert_eq!(loops.laid_out_edges().len(), 7 * 3);

        let wide = by_name("wide-shallow-30");
        assert_eq!(wide.nodes.len(), 31);
        assert_eq!(wide.edges.len(), 30);

        let tree = by_name("tree-60");
        assert!(tree.options.tree);
        assert_eq!(tree.nodes.len(), 60);
        assert_eq!(tree.edges.len(), 59, "a tree on n nodes has n-1 edges");
    }

    /// The wide layered fixture is exactly 8 ranks of 25, every rank reachable, every edge a
    /// single-rank forward step, and no duplicated pair.
    #[test]
    fn layered_wide_is_eight_sparse_ranks_of_twenty_five() {
        let f = synthetic_fixtures()
            .into_iter()
            .find(|f| f.name == "layered-wide-200x8")
            .expect("fixture exists");
        assert_eq!(f.nodes.len(), 200);
        assert_eq!(f.edges.len(), 600);
        let rank = |i: usize| i / 25;
        assert!(
            f.edges.iter().all(|e| rank(e.to) == rank(e.from) + 1),
            "every edge steps exactly one rank forward"
        );
        let pairs: BTreeSet<(usize, usize)> = f.edges.iter().map(|e| (e.from, e.to)).collect();
        assert_eq!(pairs.len(), f.edges.len(), "no duplicated edge");
        let with_pred: BTreeSet<usize> = f.edges.iter().map(|e| e.to).collect();
        assert_eq!(with_pred.len(), 175, "every node above rank 0 has a parent");
    }

    /// The cycle-heavy fixture keeps its requested density and gets roughly the requested share of
    /// back edges.
    #[test]
    fn cycles_300_is_dense_and_cyclic() {
        let f = synthetic_fixtures()
            .into_iter()
            .find(|f| f.name == "cycles-300")
            .expect("fixture exists");
        assert_eq!(f.nodes.len(), 300);
        assert_eq!(f.edges.len(), 500);
        let backs = f.edges.iter().filter(|e| e.to < e.from).count();
        let extras = f.edges.len() - 299;
        assert!(
            backs * 100 >= extras * 29 && backs * 100 <= extras * 31,
            "{backs} back edges out of {extras} extras is not ~30%"
        );
        assert!(f.edges.iter().all(|e| e.from != e.to));
    }

    /// The random graphs hit their size target and contain both forward and back edges.
    #[test]
    fn random_dags_have_the_requested_density_and_cycles() {
        for (name, n) in [
            ("dag-20", 20usize),
            ("dag-50", 50),
            ("dag-100", 100),
            ("dag-200", 200),
            ("dag-400", 400),
            ("dag-1000", 1000),
            ("dag-2000", 2000),
        ] {
            let f = synthetic_fixtures()
                .into_iter()
                .find(|f| f.name == name)
                .expect("fixture exists");
            assert_eq!(f.nodes.len(), n);
            let want = (n as f64 * 1.5).round() as usize;
            assert_eq!(f.edges.len(), want, "{name} edge count");
            assert!(
                f.edges.iter().any(|e| e.to < e.from),
                "{name} must contain back edges"
            );
            assert!(f.edges.iter().all(|e| e.from != e.to));
        }
    }
}

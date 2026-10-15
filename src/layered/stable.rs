//! Keeps a drawing's layers and order across a relayout ([`crate::options::SeedMode::Structure`]).
//!
//! Fresh layering can move nodes to other layers after a few edges are removed, so here layers
//! come from the seeds (overlapping boxes on the layer axis share a layer, as in
//! [`super::reroute`]) and each layer is ordered by seeded position.

use super::state::LayeredState;
use crate::geometry::stats::barycentre;
use crate::geometry::{EPS, SLACK};
use crate::options::LayoutOptions;
use crate::sort::SortDyn;

/// Tiny lean of a guessed waypoint towards its edge's near end, per layer of span. It only orders
/// two edges into one node by their sources.
const SOURCE_LEAN: f64 = 1e-3;

/// Ranks nodes from their seeds and orients every arc from the lower layer to the higher one.
/// Returns `false` and changes nothing if a real node has no seed or an arc joins two nodes of
/// one layer.
pub fn seeded_layers(st: &mut LayeredState) -> bool {
    let n = st.n_real;
    if n == 0 || st.nodes.iter().take(n).any(|nd| nd.seed.is_none()) {
        return false;
    }
    let along: Vec<f64> = st
        .nodes
        .iter()
        .take(n)
        .map(|nd| nd.seed.map_or(0.0, |s| s.0))
        .collect();
    let mut by_layer: Vec<usize> = (0..n).collect();
    by_layer.sort_unstable_by_dyn(|&a, &b| along[a].total_cmp(&along[b]).then(a.cmp(&b)));
    let mut rank = vec![0i32; n];
    let mut cur = 0i32;
    for w in 1..n {
        let (a, b) = (by_layer[w - 1], by_layer[w]);
        if along[b] - along[a] >= 0.5 * (st.nodes[a].w_along + st.nodes[b].w_along) {
            cur += 1;
        }
        rank[b] = cur;
    }
    if st
        .arcs
        .iter()
        .any(|a| a.from != a.to && rank[a.from] == rank[a.to])
    {
        return false;
    }
    for (nd, &r) in st.nodes.iter_mut().zip(&rank) {
        nd.rank = r;
    }
    for arc in st.arcs.iter_mut() {
        if arc.from != arc.to && rank[arc.from] > rank[arc.to] {
            std::mem::swap(&mut arc.from, &mut arc.to);
            arc.reversed = !arc.reversed;
        }
    }
    true
}

/// Orders every layer by [`seeded_cross`]. Each waypoint goes into the nearest gap between real
/// nodes that has room at the seeded positions, so it never pushes real nodes off their seeds.
pub fn seeded_order(st: &mut LayeredState, o: &LayoutOptions) {
    let key = seeded_cross(st);
    let by_key = |a: &usize, b: &usize| key[*a].total_cmp(&key[*b]).then(a.cmp(b));
    let layers = std::mem::take(&mut st.layers);
    let mut ordered: Vec<Vec<usize>> = Vec::with_capacity(layers.len());
    for layer in layers {
        let (mut real, mut way): (Vec<usize>, Vec<usize>) =
            layer.into_iter().partition(|&v| !st.nodes[v].is_dummy());
        real.sort_by_dyn(by_key);
        way.sort_by_dyn(by_key);
        // `slots[s]` holds the waypoints between `real[s - 1]` and `real[s]`. The end slots are
        // open-ended.
        let mut slots: Vec<Vec<usize>> = vec![Vec::new(); real.len() + 1];
        let fits = |slot: &[usize], s: usize, v: usize| -> bool {
            if s == 0 || s == real.len() {
                return true;
            }
            let (a, b) = (real[s - 1], real[s]);
            let mut chain = Vec::with_capacity(slot.len() + 3);
            chain.push(a);
            chain.extend_from_slice(slot);
            chain.push(v);
            chain.push(b);
            let need: f64 = chain.windows(2).map(|w| st.sep(w[0], w[1], o)).sum();
            need <= key[b] - key[a] + SLACK
        };
        for v in way {
            let home = real.partition_point(|&r| key[r] <= key[v]);
            let s = (0..=real.len())
                .flat_map(|d| [home.checked_sub(d), Some(home + d)])
                .flatten()
                .find(|&s| s <= real.len() && fits(&slots[s], s, v))
                .unwrap_or(home);
            let at = slots[s].partition_point(|&w| by_key(&w, &v).is_lt());
            slots[s].insert(at, v);
        }
        let mut out = Vec::with_capacity(real.len() + slots.iter().map(Vec::len).sum::<usize>());
        for (s, slot) in slots.into_iter().enumerate() {
            out.extend(slot);
            if let Some(&r) = real.get(s) {
                out.push(r);
            }
        }
        ordered.push(out);
    }
    settle_waypoints(st, &mut ordered);
    st.layers = ordered;
    st.reindex();
}

/// Barycentre sweeps that reorder waypoints only within their run between two real nodes. This
/// moves no real node, and keeping the best ordering seen means it never adds a crossing.
fn settle_waypoints(st: &LayeredState, layers: &mut [Vec<usize>]) {
    const SWEEPS: usize = 8;
    let n = st.len();
    let (pred, succ) = st.adjacency();
    let mut pos = vec![0usize; n];
    let index = |layers: &[Vec<usize>], pos: &mut [usize]| {
        for layer in layers {
            for (i, &v) in layer.iter().enumerate() {
                pos[v] = i;
            }
        }
    };
    index(layers, &mut pos);
    let mut best = super::order::count_crossings(layers, &pred, &pos);
    let mut best_layers = layers.to_vec();
    for sweep in 0..SWEEPS {
        if best == 0 {
            break;
        }
        let down = sweep % 2 == 0;
        let nb = if down { &pred } else { &succ };
        let order: Vec<usize> = if down {
            (1..layers.len()).collect()
        } else {
            (0..layers.len().saturating_sub(1)).rev().collect()
        };
        for l in order {
            let layer = &mut layers[l];
            let mut i = 0;
            while i < layer.len() {
                if !st.nodes[layer[i]].is_dummy() {
                    i += 1;
                    continue;
                }
                let start = i;
                while i < layer.len() && st.nodes[layer[i]].is_dummy() {
                    i += 1;
                }
                let run = &mut layer[start..i];
                let bary = |v: usize| barycentre(nb, v, &pos);
                run.sort_by_dyn(|&a, &b| bary(a).total_cmp(&bary(b)).then(pos[a].cmp(&pos[b])));
            }
            for (k, &v) in layer.iter().enumerate() {
                pos[v] = k;
            }
        }
        let c = super::order::count_crossings(layers, &pred, &pos);
        if c < best {
            best = c;
            best_layers = layers.to_vec();
        }
    }
    layers.clone_from_slice(&best_layers);
}

/// Cross-axis target per node: a real node's seed, or for a waypoint the lane of its edge's far
/// end, so the edge needs a single riser.
pub fn seeded_cross(st: &LayeredState) -> Vec<f64> {
    let len = st.len();
    let mut succ = vec![usize::MAX; len];
    let mut pred = vec![usize::MAX; len];
    for a in &st.arcs {
        if a.from != a.to {
            if st.nodes[a.from].is_dummy() {
                succ[a.from] = a.to;
            }
            if st.nodes[a.to].is_dummy() {
                pred[a.to] = a.from;
            }
        }
    }
    let real_end = |mut v: usize, step: &[usize]| {
        let mut guard = 0;
        while st.nodes[v].is_dummy() && step[v] != usize::MAX && guard < len {
            v = step[v];
            guard += 1;
        }
        v
    };
    (0..len)
        .map(|v| {
            let nd = &st.nodes[v];
            if !nd.is_dummy() {
                return nd.seed.map_or(nd.cross, |s| s.1);
            }
            let (lo, hi) = (real_end(v, &pred), real_end(v, &succ));
            let (a, b) = (&st.nodes[lo], &st.nodes[hi]);
            let (ca, cb) = (
                a.seed.map_or(a.cross, |s| s.1),
                b.seed.map_or(b.cross, |s| s.1),
            );
            let span = (b.rank - a.rank) as f64;
            if span.abs() < EPS {
                cb
            } else {
                cb + (ca - cb) * SOURCE_LEAN * (b.rank - nd.rank) as f64 / span
            }
        })
        .collect()
}

/// Widens layer gaps so each layer lands back on its seeded position. A gap is never narrowed,
/// so nothing overlaps.
pub fn hold_layers(st: &mut LayeredState) -> Result<(), crate::error::LayoutError> {
    let n_layers = st.layers.len();
    if n_layers < 2 {
        return Ok(());
    }
    let off: Vec<Option<f64>> = st
        .layers
        .iter()
        .map(|layer| {
            let (sum, k) = layer
                .iter()
                .filter_map(|&v| st.nodes[v].seed.map(|s| s.0 - st.nodes[v].along))
                .fold((0.0, 0usize), |(s, k), d| (s + d, k + 1));
            (k > 0).then(|| sum / k as f64)
        })
        .collect();
    let Some(base) = off.iter().flatten().next().copied() else {
        return Ok(());
    };
    let mut gap = std::mem::take(&mut st.gap_after);
    gap.resize(n_layers, 0.0);
    let mut shift = 0.0f64;
    for l in 1..n_layers {
        if let Some(o) = off[l] {
            let want = o - base;
            if want > shift {
                gap[l - 1] += want - shift;
                shift = want;
            }
        }
    }
    let result = super::coord::assign_layer_positions(st, &gap);
    st.gap_after = gap;
    result
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use crate::{layout, Edge, Graph, GraphBuilder, Layout, LayoutOptions, NodeId, SeedMode, Size};

    /// 0 -> 1 -> 2 -> 3, 0 -> 3, 0 -> 4 -> 3, 1 -> 5.
    fn graph() -> Graph {
        let mut b = GraphBuilder::new();
        for _ in 0..6 {
            b.add_box(Size::new(80.0, 40.0));
        }
        for (s, t) in [(0, 1), (1, 2), (2, 3), (0, 3), (0, 4), (4, 3), (1, 5)] {
            b.add_edge(Edge::new(NodeId(s), NodeId(t)));
        }
        b.build()
    }

    fn seeded(mut g: Graph, from: &Layout) -> Graph {
        for (n, p) in g.nodes.iter_mut().zip(&from.nodes) {
            n.position = Some(p.center);
        }
        g
    }

    fn keep() -> LayoutOptions {
        LayoutOptions {
            seed_mode: SeedMode::Structure,
            ..LayoutOptions::default()
        }
    }

    #[test]
    fn laying_a_drawing_out_again_over_its_own_positions_reproduces_it() {
        let g = graph();
        let before = layout(&g, &LayoutOptions::default()).unwrap();
        let again = layout(&seeded(g, &before), &keep()).unwrap();
        for (a, b) in before.nodes.iter().zip(&again.nodes) {
            assert!(
                (a.center.x - b.center.x).abs() < 0.5 && (a.center.y - b.center.y).abs() < 0.5,
                "{:?} moved to {:?}",
                a.center,
                b.center
            );
        }
    }

    #[test]
    fn a_filtered_graph_keeps_its_layers_and_order() {
        let g = graph();
        let before = layout(&g, &LayoutOptions::default()).unwrap();
        let mut filtered = seeded(g, &before);
        // Drop the long edge and the fork's side branch.
        filtered
            .edges
            .retain(|e| !matches!((e.source.0, e.target.0), (0, 3) | (0, 4) | (4, 3)));
        let after = layout(&filtered, &keep()).unwrap();
        // Node 4 lost both its edges, so it has no layer.
        assert_eq!(after.nodes[4].rank, None);
        for (a, b) in before
            .nodes
            .iter()
            .zip(&after.nodes)
            .filter(|(_, b)| b.rank.is_some())
        {
            assert_eq!(a.rank, b.rank, "a node changed layer");
        }
        let n = before.nodes.len();
        for i in 0..n {
            for j in 0..n {
                let (a, b) = (&before.nodes, &after.nodes);
                let side = |l: &[crate::Placement], k: usize| {
                    (l[i].center.x - l[j].center.x).abs() > 1.0 && k == 0
                        || (l[i].center.y - l[j].center.y).abs() > 1.0 && k == 1
                };
                for k in 0..2 {
                    if side(a, k) && side(b, k) {
                        let d = |l: &[crate::Placement]| {
                            if k == 0 {
                                l[i].center.x < l[j].center.x
                            } else {
                                l[i].center.y < l[j].center.y
                            }
                        };
                        assert_eq!(d(a), d(b), "nodes {i} and {j} swapped");
                    }
                }
            }
        }
    }

    #[test]
    fn an_edge_within_one_seeded_layer_falls_back_to_a_fresh_layout() {
        let mut b = GraphBuilder::new();
        let x = b.add_box(Size::new(80.0, 40.0));
        let y = b.add_box(Size::new(80.0, 40.0));
        b.add_edge(Edge::new(x, y));
        let mut g = b.build();
        g.nodes[0].position = Some(crate::Point::new(0.0, 0.0));
        g.nodes[1].position = Some(crate::Point::new(0.0, 200.0));
        let fresh = layout(&g, &LayoutOptions::default()).unwrap();
        let kept = layout(&g, &keep()).unwrap();
        assert_ne!(kept.nodes[0].rank, kept.nodes[1].rank);
        assert_eq!(
            fresh.nodes.iter().map(|n| n.rank).collect::<Vec<_>>(),
            kept.nodes.iter().map(|n| n.rank).collect::<Vec<_>>()
        );
    }
}

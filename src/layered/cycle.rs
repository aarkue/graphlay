//! Cycle breaking. A light set of arcs is reversed here and turned back when routes are emitted.
//!
//! Every strategy produces a linear node order and reverses each arc running backwards in it, so
//! the result is acyclic by construction.

use std::cmp::Ordering;
use std::collections::BTreeSet;

use super::state::LayeredState;
use crate::error::LayoutError;
use crate::geometry::EPS;
use crate::sort::SortDyn;

/// Every variant except `FollowFlow` ends with [`improve_order`]. `GreedyFas` and `Dfs` are only
/// used in tests.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CycleBreaker {
    /// Weighted greedy Eades-Lin-Smyth sequencing, `O((V + E) log V)`.
    #[allow(dead_code)]
    GreedyFas,
    /// Depth-first reverse postorder, `O(V + E)`. Blind to weight, it reverses whichever arc of a
    /// cycle the search happens to close.
    #[allow(dead_code)]
    Dfs,
    /// Both of the above on the graph and its reverse, keeping the order with the least backward
    /// weight ([`crate::options::CycleStrategy::MinimumReversal`]).
    Best,
    /// Components in topological order, nodes by distance from the component's entry
    /// ([`scc_flow_order`]). Only arcs inside a cycle are reversed, each pointing back to the
    /// entry, whatever its weight ([`crate::options::CycleStrategy::FollowFlow`]).
    FollowFlow,
}

/// Caps the improvement sweeps instead of waiting for a fixed point, since later sweeps move few
/// nodes.
const IMPROVE_ROUNDS: usize = 4;

#[cfg(test)]
pub const DEFAULT_BREAKER: CycleBreaker = CycleBreaker::Best;

#[cfg(test)]
pub fn break_cycles(st: &mut LayeredState) -> Result<(), LayoutError> {
    break_cycles_with(st, DEFAULT_BREAKER)
}

pub fn breaker_for(strategy: crate::options::CycleStrategy) -> CycleBreaker {
    match strategy {
        crate::options::CycleStrategy::MinimumReversal => CycleBreaker::Best,
        crate::options::CycleStrategy::FollowFlow => CycleBreaker::FollowFlow,
    }
}

/// Self-loops are left alone. They are extracted before routing and cannot be part of a cycle.
pub fn break_cycles_with(st: &mut LayeredState, how: CycleBreaker) -> Result<(), LayoutError> {
    if st.is_empty() {
        return Ok(());
    }
    let g = Adjacency::of(st);
    let seq = node_order(&g, how);
    let mut rank = vec![0usize; g.n];
    for (i, &v) in seq.iter().enumerate() {
        rank[v] = i;
    }
    for arc in st.arcs.iter_mut() {
        if arc.from != arc.to && rank[arc.from] > rank[arc.to] {
            std::mem::swap(&mut arc.from, &mut arc.to);
            arc.reversed = !arc.reversed;
        }
    }
    Ok(())
}

fn node_order(g: &Adjacency, how: CycleBreaker) -> Vec<usize> {
    match how {
        CycleBreaker::GreedyFas => improved(g, greedy_fas_order(g)),
        CycleBreaker::Dfs => improved(g, dfs_order(g)),
        CycleBreaker::FollowFlow => scc_flow_order(g, greedy_fas_order(g)),
        CycleBreaker::Best => {
            // Improvement only finds a local optimum, so several seeds are tried. Seeds from the
            // reversed graph start from the other end and reach different optima.
            let t = g.transposed();
            let mut rev_greedy = greedy_fas_order(&t);
            rev_greedy.reverse();
            let mut rev_dfs = dfs_order(&t);
            rev_dfs.reverse();
            // Unit weights often tie, so among least-weight orders prefer the one whose reversed
            // arcs point back toward the start. It stays a tiebreak because charging the other cut
            // extra draws the heavier side of a two-way pair backwards.
            let depth = depth_from_sources(g);
            let score = |seq: &[usize]| (backward_weight(g, seq), unnatural_weight(g, seq, &depth));
            let mut best = improved(g, greedy_fas_order(g));
            let mut best_s = score(&best);
            for seed in [dfs_order(g), rev_greedy, rev_dfs, dfs_order_from_sources(g)] {
                let cand = improved(g, seed);
                let s = score(&cand);
                if s.0 < best_s.0 - EPS || (s.0 <= best_s.0 + EPS && s.1 < best_s.1 - EPS) {
                    best_s = s;
                    best = cand;
                }
            }
            best
        }
    }
}

/// Iterative Tarjan. Components are numbered in reverse topological order of the condensation.
fn sccs(g: &Adjacency) -> (Vec<usize>, usize) {
    let n = g.n;
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on = vec![false; n];
    let mut comp = vec![usize::MAX; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut next = 0usize;
    let mut n_comp = 0usize;
    for root in 0..n {
        if index[root] != usize::MAX {
            continue;
        }
        let mut work: Vec<(usize, usize)> = vec![(root, 0)];
        index[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root);
        on[root] = true;
        while let Some(&mut (v, ref mut i)) = work.last_mut() {
            if *i < g.out[v].len() {
                let u = g.out[v][*i].0;
                *i += 1;
                if index[u] == usize::MAX {
                    index[u] = next;
                    low[u] = next;
                    next += 1;
                    stack.push(u);
                    on[u] = true;
                    work.push((u, 0));
                } else if on[u] {
                    low[v] = low[v].min(index[u]);
                }
            } else {
                work.pop();
                if let Some(&(p, _)) = work.last() {
                    low[p] = low[p].min(low[v]);
                }
                if low[v] == index[v] {
                    while let Some(w) = stack.pop() {
                        on[w] = false;
                        comp[w] = n_comp;
                        if w == v {
                            break;
                        }
                    }
                    n_comp += 1;
                }
            }
        }
    }
    (comp, n_comp)
}

/// Components in topological order, and within each the nodes by BFS distance from where the flow
/// enters it, or from its first node in `fallback` if nothing enters. Only arcs inside a cycle can
/// run backwards, each back toward the cycle's entry.
fn scc_flow_order(g: &Adjacency, fallback: Vec<usize>) -> Vec<usize> {
    let (comp, n_comp) = sccs(g);
    let mut pos = vec![0usize; g.n];
    for (i, &v) in fallback.iter().enumerate() {
        pos[v] = i;
    }
    // Tarjan numbers components in reverse topological order.
    let topo = |c: usize| n_comp - 1 - c;
    let mut depth = vec![usize::MAX; g.n];
    let mut queue = std::collections::VecDeque::new();
    let mut entered = vec![false; n_comp];
    for v in 0..g.n {
        if g.inc[v].iter().any(|&(u, _)| comp[u] != comp[v]) {
            depth[v] = 0;
            entered[comp[v]] = true;
        }
    }
    for (c, _) in entered.iter().enumerate().filter(|(_, &e)| !e) {
        if let Some(&v) = fallback.iter().find(|&&v| comp[v] == c) {
            depth[v] = 0;
        }
    }
    queue.extend((0..g.n).filter(|&v| depth[v] == 0));
    while let Some(v) = queue.pop_front() {
        for &(u, _) in &g.out[v] {
            if comp[u] == comp[v] && depth[u] == usize::MAX {
                depth[u] = depth[v] + 1;
                queue.push_back(u);
            }
        }
    }
    let mut seq: Vec<usize> = (0..g.n).collect();
    seq.sort_by_key_dyn(|&v| (topo(comp[v]), depth[v], pos[v]));
    seq
}

fn improved(g: &Adjacency, mut seq: Vec<usize>) -> Vec<usize> {
    improve_order(g, &mut seq);
    seq
}

fn backward_weight(g: &Adjacency, seq: &[usize]) -> f64 {
    let mut rank = vec![0usize; g.n];
    for (i, &v) in seq.iter().enumerate() {
        rank[v] = i;
    }
    let mut total = 0.0;
    for (v, arcs) in g.out.iter().enumerate() {
        for &(u, w) in arcs {
            if rank[v] > rank[u] {
                total += w;
            }
        }
    }
    total
}

/// BFS distance from the sources over forward arcs, `usize::MAX` where no source reaches.
fn depth_from_sources(g: &Adjacency) -> Vec<usize> {
    let mut depth = vec![usize::MAX; g.n];
    let mut queue = std::collections::VecDeque::new();
    for v in (0..g.n).filter(|&v| g.inc[v].is_empty() && !g.out[v].is_empty()) {
        depth[v] = 0;
        queue.push_back(v);
    }
    while let Some(v) = queue.pop_front() {
        for &(u, _) in &g.out[v] {
            if depth[u] == usize::MAX {
                depth[u] = depth[v] + 1;
                queue.push_back(u);
            }
        }
    }
    depth
}

/// Weight of reversed arcs that run from a node nearer the sources to one further away, which
/// puts a loop's body in front of its entry. Arcs with an unreached end do not count.
fn unnatural_weight(g: &Adjacency, seq: &[usize], depth: &[usize]) -> f64 {
    let mut rank = vec![0usize; g.n];
    for (i, &v) in seq.iter().enumerate() {
        rank[v] = i;
    }
    let mut total = 0.0;
    for (v, arcs) in g.out.iter().enumerate() {
        for &(u, w) in arcs {
            if rank[v] > rank[u]
                && depth[v] != usize::MAX
                && depth[u] != usize::MAX
                && depth[u] > depth[v]
            {
                total += w;
            }
        }
    }
    total
}

/// Self-loops dropped. Parallel arcs stay separate, so their counts decide when a node is a sink.
struct Adjacency {
    n: usize,
    out: Vec<Vec<(usize, f64)>>,
    inc: Vec<Vec<(usize, f64)>>,
}

impl Adjacency {
    fn of(st: &LayeredState) -> Self {
        let n = st.len();
        let mut out: Vec<Vec<(usize, f64)>> = vec![vec![]; n];
        let mut inc: Vec<Vec<(usize, f64)>> = vec![vec![]; n];
        for arc in &st.arcs {
            if arc.from == arc.to {
                continue;
            }
            let w = st.weight.get(arc.edge).copied().unwrap_or(1.0);
            out[arc.from].push((arc.to, w));
            inc[arc.to].push((arc.from, w));
        }
        Adjacency { n, out, inc }
    }

    fn transposed(&self) -> Adjacency {
        Adjacency {
            n: self.n,
            out: self.inc.clone(),
            inc: self.out.clone(),
        }
    }
}

/// `total_cmp` ordering, so the peeling order is the same on every platform.
#[derive(Clone, Copy)]
struct Key(f64);

impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        self.0.total_cmp(&other.0) == Ordering::Equal
    }
}
impl Eq for Key {}
impl Ord for Key {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}
impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

struct Peel<'a> {
    g: &'a Adjacency,
    /// Counted, not weighed, so a zero-weight arc still keeps its endpoint out of a queue.
    out_cnt: Vec<usize>,
    in_cnt: Vec<usize>,
    out_w: Vec<f64>,
    in_w: Vec<f64>,
    live: Vec<bool>,
    sinks: BTreeSet<usize>,
    sources: BTreeSet<usize>,
    /// Keyed by `in_w - out_w`, so the first entry has the most outgoing weight relative to
    /// incoming.
    by_delta: BTreeSet<(Key, usize)>,
    left: usize,
}

impl Peel<'_> {
    fn new(g: &Adjacency) -> Peel<'_> {
        let n = g.n;
        let sum = |a: &Vec<(usize, f64)>| a.iter().map(|&(_, w)| w).sum::<f64>();
        let out_w: Vec<f64> = g.out.iter().map(sum).collect();
        let in_w: Vec<f64> = g.inc.iter().map(sum).collect();
        // Filled by insertion, since `BTreeSet::from_iter` sorts stably and would pull the stable
        // sort into the binary next to the unstable one the crate uses everywhere else.
        let mut by_delta = BTreeSet::new();
        let mut sinks = BTreeSet::new();
        let mut sources = BTreeSet::new();
        for v in 0..n {
            by_delta.insert((Key(in_w[v] - out_w[v]), v));
            if g.out[v].is_empty() {
                sinks.insert(v);
            }
            if g.inc[v].is_empty() {
                sources.insert(v);
            }
        }
        Peel {
            g,
            out_cnt: g.out.iter().map(Vec::len).collect(),
            in_cnt: g.inc.iter().map(Vec::len).collect(),
            by_delta,
            sinks,
            sources,
            out_w,
            in_w,
            live: vec![true; n],
            left: n,
        }
    }

    fn key(&self, v: usize) -> (Key, usize) {
        (Key(self.in_w[v] - self.out_w[v]), v)
    }

    fn remove(&mut self, v: usize) {
        let key = self.key(v);
        self.by_delta.remove(&key);
        self.sinks.remove(&v);
        self.sources.remove(&v);
        self.live[v] = false;
        self.left -= 1;
        let g = self.g; // copied out so the neighbour walks can mutate `self`
        for &(u, w) in &g.inc[v] {
            if !self.live[u] {
                continue;
            }
            let old = self.key(u);
            self.by_delta.remove(&old);
            self.out_cnt[u] -= 1;
            self.out_w[u] -= w;
            let new = self.key(u);
            self.by_delta.insert(new);
            if self.out_cnt[u] == 0 {
                self.sinks.insert(u);
            }
        }
        for &(u, w) in &g.out[v] {
            if !self.live[u] {
                continue;
            }
            let old = self.key(u);
            self.by_delta.remove(&old);
            self.in_cnt[u] -= 1;
            self.in_w[u] -= w;
            let new = self.key(u);
            self.by_delta.insert(new);
            if self.in_cnt[u] == 0 {
                self.sources.insert(u);
            }
        }
    }
}

/// Eades-Lin-Smyth greedy sequencing, weight-aware. Sinks and sources are peeled for free, since
/// their arcs can never run backwards. Otherwise the node whose outgoing weight most exceeds its
/// incoming goes next. `O((V + E) log V)`.
fn greedy_fas_order(g: &Adjacency) -> Vec<usize> {
    let mut p = Peel::new(g);
    let mut head: Vec<usize> = Vec::with_capacity(g.n);
    let mut tail: Vec<usize> = Vec::with_capacity(g.n);
    while p.left > 0 {
        while let Some(&v) = p.sinks.iter().next() {
            tail.push(v);
            p.remove(v);
        }
        while let Some(&v) = p.sources.iter().next() {
            head.push(v);
            p.remove(v);
        }
        if let Some(&(_, v)) = p.by_delta.iter().next() {
            head.push(v);
            p.remove(v);
        }
    }
    tail.reverse();
    head.extend(tail);
    head
}

/// Moves each node to the slot where the least arc weight runs backwards. The cost only changes
/// where the slot passes a neighbour, so the `deg(v) + 1` slots right of each neighbour (plus the
/// far left) are scored in one sorted sweep. Ties keep the node in place, so the pass converges.
fn improve_order(g: &Adjacency, seq: &mut [usize]) {
    let n = g.n;
    if n < 3 {
        return;
    }
    let mut pos = vec![0usize; n];
    for (i, &v) in seq.iter().enumerate() {
        pos[v] = i;
    }
    // Reused across nodes. `stamp` marks which entries belong to the current visit, so the arrays
    // can stay dirty between nodes.
    let mut out_w = vec![0.0f64; n];
    let mut in_w = vec![0.0f64; n];
    let mut stamp = vec![0usize; n];
    let mut visit = 0usize;
    let mut nbrs: Vec<usize> = Vec::new();
    let mut breaks: Vec<(usize, f64)> = Vec::new();
    for _ in 0..IMPROVE_ROUNDS {
        let mut moved = false;
        for v in 0..n {
            if g.out[v].is_empty() && g.inc[v].is_empty() {
                continue;
            }
            visit += 1;
            nbrs.clear();
            for &(u, _) in g.out[v].iter().chain(g.inc[v].iter()) {
                if stamp[u] != visit {
                    stamp[u] = visit;
                    out_w[u] = 0.0;
                    in_w[u] = 0.0;
                    nbrs.push(u);
                }
            }
            for &(u, w) in &g.out[v] {
                out_w[u] += w;
            }
            for &(u, w) in &g.inc[v] {
                in_w[u] += w;
            }
            let p = pos[v];
            // Slots index the sequence with `v` lifted out, so a neighbour past `v` shifts down by
            // one.
            breaks.clear();
            let mut incoming = 0.0;
            for &u in &nbrs {
                let q = if pos[u] > p { pos[u] - 1 } else { pos[u] };
                breaks.push((q, out_w[u] - in_w[u]));
                incoming += in_w[u];
            }
            breaks.sort_unstable_by_dyn(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
            // At slot 0 every incoming arc runs backwards. Passing a neighbour turns its outgoing
            // arcs backwards and its incoming ones forwards.
            let (mut best_slot, mut best_cost) = (0usize, incoming);
            let mut running = incoming;
            let mut here = incoming;
            for &(q, delta) in &breaks {
                running += delta;
                if q < p {
                    here = running;
                }
                if running < best_cost {
                    best_cost = running;
                    best_slot = q + 1;
                }
            }
            if best_cost >= here - EPS || best_slot == p {
                continue; // ties stay put, so the pass converges
            }
            if best_slot < p {
                seq[best_slot..=p].rotate_right(1);
                for (i, &u) in seq.iter().enumerate().take(p + 1).skip(best_slot) {
                    pos[u] = i;
                }
            } else {
                seq[p..=best_slot].rotate_left(1);
                for (i, &u) in seq.iter().enumerate().take(best_slot + 1).skip(p) {
                    pos[u] = i;
                }
            }
            moved = true;
        }
        if !moved {
            break;
        }
    }
}

/// In reverse postorder only the DFS back edges run backwards. Iterative, since recursion would
/// nest once per node of a chain.
fn dfs_order(g: &Adjacency) -> Vec<usize> {
    dfs_order_from(g, 0..g.n)
}

/// Starts from the sources first, so every back edge closes a loop onto a node reached earlier.
fn dfs_order_from_sources(g: &Adjacency) -> Vec<usize> {
    let sources = (0..g.n).filter(|&v| g.inc[v].is_empty());
    dfs_order_from(g, sources.chain(0..g.n))
}

fn dfs_order_from(g: &Adjacency, starts: impl Iterator<Item = usize>) -> Vec<usize> {
    let n = g.n;
    let mut seen = vec![false; n];
    let mut post: Vec<usize> = Vec::with_capacity(n);
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for s in starts {
        if seen[s] {
            continue;
        }
        seen[s] = true;
        stack.push((s, 0));
        while let Some(top) = stack.last_mut() {
            let (v, i) = (top.0, top.1);
            if i < g.out[v].len() {
                top.1 += 1;
                let u = g.out[v][i].0;
                if !seen[u] {
                    seen[u] = true;
                    stack.push((u, 0));
                }
            } else {
                post.push(v);
                stack.pop();
            }
        }
    }
    post.reverse();
    post
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::geometry::shape::NodeShape;
    use crate::layered::{LayeredInput, LayeredInputNode};
    use crate::options::LayoutOptions;

    /// Transitive-closure breaker used as the quality reference. Decides one arc at a time,
    /// heaviest first, `O(E * n^2)` time and `O(n^2)` memory.
    fn break_cycles_closure(st: &mut LayeredState) {
        let n = st.len();
        if n == 0 {
            return;
        }
        let mut order: Vec<usize> = (0..st.arcs.len())
            .filter(|&i| st.arcs[i].from != st.arcs[i].to)
            .collect();
        order.sort_unstable_by_dyn(|&a, &b| {
            let wa = st.weight.get(st.arcs[a].edge).copied().unwrap_or(1.0);
            let wb = st.weight.get(st.arcs[b].edge).copied().unwrap_or(1.0);
            wb.total_cmp(&wa).then(a.cmp(&b))
        });
        let mut reach = vec![vec![false; n]; n];
        for i in order {
            let arc = st.arcs[i];
            let (from, to) = (arc.from, arc.to);
            if reach[to][from] {
                st.arcs[i].from = to;
                st.arcs[i].to = from;
                st.arcs[i].reversed = !arc.reversed;
                continue;
            }
            if reach[from][to] {
                continue;
            }
            let reach_to = reach[to].clone();
            for (x, reach_x) in reach.iter_mut().enumerate().take(n) {
                if x == from || reach_x[from] {
                    reach_x[to] = true;
                    for (y, &r) in reach_to.iter().enumerate() {
                        if r {
                            reach_x[y] = true;
                        }
                    }
                }
            }
        }
    }

    fn state(n: usize, edges: &[(usize, usize)], weights: &[f64]) -> LayeredState {
        let input = LayeredInput {
            nodes: (0..n)
                .map(|_| LayeredInputNode {
                    width: 120.0,
                    height: 52.0,
                    shape: NodeShape::Box,
                    constraint: None,
                    category: None,
                    clear_after: 0.0,
                    clear_before: 0.0,
                    port_sides: None,
                    lane_key: None,
                    along_before: 0.0,
                    along_after: 0.0,
                })
                .collect(),
            edges: edges.to_vec(),
            weights: weights.to_vec(),
            thickness: vec![],
            edge_label_sizes: vec![],
            seed: vec![],
            pinned: vec![],
            self_loops: vec![],
            ranked_backwards: Vec::new(),
            options: LayoutOptions::default(),
        };
        LayeredState::new(&input)
    }

    /// Kahn's algorithm, self-loops ignored.
    fn is_acyclic(st: &LayeredState) -> bool {
        let n = st.len();
        let mut indeg = vec![0usize; n];
        let mut out: Vec<Vec<usize>> = vec![vec![]; n];
        for arc in &st.arcs {
            if arc.from == arc.to {
                continue;
            }
            out[arc.from].push(arc.to);
            indeg[arc.to] += 1;
        }
        let mut queue: Vec<usize> = (0..n).filter(|&v| indeg[v] == 0).collect();
        let mut seen = 0;
        while let Some(v) = queue.pop() {
            seen += 1;
            for &u in &out[v] {
                indeg[u] -= 1;
                if indeg[u] == 0 {
                    queue.push(u);
                }
            }
        }
        seen == n
    }

    /// `(reversed arcs, total reversed weight)`.
    fn reversed(st: &LayeredState) -> (usize, f64) {
        let mut count = 0;
        let mut weight = 0.0;
        for arc in &st.arcs {
            if arc.reversed {
                count += 1;
                weight += st.weight.get(arc.edge).copied().unwrap_or(1.0);
            }
        }
        (count, weight)
    }

    /// The LCG the synthetic fixtures use.
    struct Lcg(u64);
    impl Lcg {
        fn below(&mut self, n: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            if n == 0 {
                0
            } else {
                (self.0 >> 33) as usize % n
            }
        }
    }

    #[test]
    fn a_dag_keeps_every_arc() {
        for how in [
            CycleBreaker::GreedyFas,
            CycleBreaker::Dfs,
            CycleBreaker::Best,
            CycleBreaker::FollowFlow,
        ] {
            let mut st = state(4, &[(0, 1), (1, 2), (2, 3), (0, 3), (0, 2)], &[]);
            break_cycles_with(&mut st, how).unwrap();
            assert_eq!(reversed(&st).0, 0, "{how:?}");
            assert!(is_acyclic(&st), "{how:?}");
        }
    }

    /// The weighted greedy reverses the lightest arc of a 3-cycle, wherever it is.
    #[test]
    fn a_three_cycle_reverses_its_lightest_arc() {
        for light in 0..3 {
            let mut weights = vec![10.0, 10.0, 10.0];
            weights[light] = 1.0;
            let mut st = state(3, &[(0, 1), (1, 2), (2, 0)], &weights);
            break_cycles_with(&mut st, DEFAULT_BREAKER).unwrap();
            assert!(is_acyclic(&st));
            let flipped: Vec<usize> = st
                .arcs
                .iter()
                .filter(|a| a.reversed)
                .map(|a| a.edge)
                .collect();
            assert_eq!(flipped, vec![light], "lightest arc is {light}");
        }
    }

    /// Node numbers are chosen so a search from node 0, the loop body, would cut the other way.
    #[test]
    fn a_tied_loop_is_cut_where_it_returns() {
        // 3 -> 4 -> entry 5 -> body 0 -> 1 -> 2 -> back to 5, and 1 -> 6 leaves.
        let edges = [(3, 4), (4, 5), (5, 0), (0, 1), (1, 2), (2, 5), (1, 6)];
        let mut st = state(7, &edges, &[]);
        break_cycles_with(&mut st, DEFAULT_BREAKER).unwrap();
        let cut: Vec<usize> = st
            .arcs
            .iter()
            .filter(|a| a.reversed)
            .map(|a| a.edge)
            .collect();
        assert_eq!(cut, vec![5], "the arc 2 -> 5 closes the loop");
    }

    #[test]
    fn self_loops_are_left_alone() {
        let mut st = state(2, &[(0, 0), (0, 1), (1, 1)], &[]);
        break_cycles_with(&mut st, DEFAULT_BREAKER).unwrap();
        assert_eq!(reversed(&st).0, 0);
        assert_eq!((st.arcs[0].from, st.arcs[0].to), (0, 0));
        assert_eq!((st.arcs[2].from, st.arcs[2].to), (1, 1));
    }

    #[test]
    fn every_strategy_leaves_an_acyclic_graph() {
        let mut rng = Lcg(0x9e37_79b9_7f4a_7c15);
        for case in 0..200 {
            let n = 2 + rng.below(40);
            let m = rng.below(4 * n);
            let edges: Vec<(usize, usize)> = (0..m).map(|_| (rng.below(n), rng.below(n))).collect();
            let weights: Vec<f64> = (0..m).map(|_| 1.0 + rng.below(20) as f64).collect();
            for how in [
                CycleBreaker::GreedyFas,
                CycleBreaker::Dfs,
                CycleBreaker::Best,
            ] {
                let mut st = state(n, &edges, &weights);
                break_cycles_with(&mut st, how).unwrap();
                assert!(is_acyclic(&st), "case {case}, {how:?}, n={n}, m={m}");
            }
        }
    }

    /// Neither breaker dominates case by case, so the bound is on the total, with a loose per-case
    /// cap.
    #[test]
    fn greedy_reverses_no_more_weight_than_the_closure_breaker() {
        let mut rng = Lcg(0x2545_f491_4f6c_dd1d);
        let (mut greedy_total, mut closure_total) = (0.0f64, 0.0f64);
        for case in 0..200 {
            let n = 3 + rng.below(30);
            let m = n + rng.below(3 * n);
            let edges: Vec<(usize, usize)> = (0..m).map(|_| (rng.below(n), rng.below(n))).collect();
            let weights: Vec<f64> = (0..m).map(|_| 1.0 + rng.below(20) as f64).collect();

            let mut a = state(n, &edges, &weights);
            break_cycles_with(&mut a, DEFAULT_BREAKER).unwrap();
            let (_, gw) = reversed(&a);

            let mut b = state(n, &edges, &weights);
            break_cycles_closure(&mut b);
            let (_, cw) = reversed(&b);
            assert!(is_acyclic(&b));

            assert!(
                gw <= cw * 2.0 + 20.0,
                "case {case}: greedy reversed {gw}, closure {cw}"
            );
            greedy_total += gw;
            closure_total += cw;
        }
        assert!(
            greedy_total <= closure_total,
            "greedy reversed {greedy_total} in total, closure {closure_total}"
        );
    }

    /// Start 0, loop 1 -> 2 -> 1 entered at 1 with a heavy return, exit 2 -> 3. Minimum reversal
    /// flips the light step, following the flow flips the heavy return.
    #[test]
    fn following_the_flow_draws_the_return_backwards_whatever_it_weighs() {
        let edges = [(0, 1), (1, 2), (2, 1), (2, 3)];
        let weights = [1.0, 1.0, 10.0, 1.0];
        let backwards = |how| {
            let mut st = state(4, &edges, &weights);
            break_cycles_with(&mut st, how).unwrap();
            assert!(is_acyclic(&st), "{how:?}");
            st.arcs
                .iter()
                .filter(|a| a.reversed)
                .map(|a| a.edge)
                .collect::<Vec<_>>()
        };
        assert_eq!(backwards(CycleBreaker::Best), vec![1]);
        assert_eq!(backwards(CycleBreaker::FollowFlow), vec![2]);
    }

    #[test]
    fn following_the_flow_reverses_only_arcs_inside_cycles() {
        let mut rng = Lcg(7);
        for _ in 0..40 {
            let n = 12;
            let mut edges = vec![];
            for v in 0..n - 1 {
                edges.push((v, v + 1));
            }
            for _ in 0..10 {
                let (a, b) = (rng.below(n), rng.below(n));
                if a != b {
                    edges.push((a, b));
                }
            }
            let mut st = state(n, &edges, &[]);
            let g = Adjacency::of(&st);
            let (comp, _) = sccs(&g);
            break_cycles_with(&mut st, CycleBreaker::FollowFlow).unwrap();
            assert!(is_acyclic(&st));
            for a in st.arcs.iter().filter(|a| a.reversed) {
                assert_eq!(comp[a.from], comp[a.to], "{edges:?}");
            }
        }
    }
}

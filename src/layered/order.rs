//! Crossing minimisation. Each seed gets weighted barycentre or median sweeps, adjacent
//! transposition, sifting and exhaustive permutation of tiny layers, all scored by the exact
//! crossings between adjacent layers. A kick search then perturbs the best order.
//!
//! Sweeps move a node across a whole layer but do not count crossings. Transposition stops when no
//! single swap helps, and sifting ([`sift_node`]) finds the multi-swap moves it misses.

use super::state::LayeredState;
use crate::error::LayoutError;
use crate::geometry::stats::weighted_median;
use crate::options::{LayoutOptions, OrderHeuristic};
use crate::sort::SortDyn;

/// Visits every permutation of `arr` (Heap's algorithm). Only for tiny layers.
pub fn for_each_perm(arr: &mut [usize], f: &mut impl FnMut(&[usize])) {
    let n = arr.len();
    let mut c = vec![0usize; n];
    f(arr);
    let mut i = 0;
    while i < n {
        if c[i] < i {
            if i % 2 == 0 {
                arr.swap(0, i);
            } else {
                arr.swap(c[i], i);
            }
            f(arr);
            c[i] += 1;
            i = 0;
        } else {
            c[i] = 0;
            i += 1;
        }
    }
}

/// Up to this many edges [`bilayer_crossings`] compares pairs directly, because the accumulator
/// tree allocates and sorts. Exhaustive permutation hits this case once per permutation.
const PAIRWISE_MAX_EDGES: usize = 24;

/// Crossings between two adjacent layers, given the edges as `(fixed, free)` position pairs, by
/// the Barth-Junger-Mutzel accumulator tree in `O(E log V)`. `free_len` bounds the free layer's
/// positions. Sorts `pairs` in place.
fn bilayer_crossings(pairs: &mut [(usize, usize)], free_len: usize) -> usize {
    if pairs.len() < 2 {
        return 0;
    }
    if pairs.len() <= PAIRWISE_MAX_EDGES {
        let mut total = 0;
        for i in 0..pairs.len() {
            for j in i + 1..pairs.len() {
                let (a, b) = (pairs[i], pairs[j]);
                if (a.0 < b.0 && a.1 > b.1) || (a.0 > b.0 && a.1 < b.1) {
                    total += 1;
                }
            }
        }
        return total;
    }
    pairs.sort_unstable_by_dyn(Ord::cmp);
    let mut leaves = 1usize;
    while leaves < free_len.max(1) {
        leaves *= 2;
    }
    let first_leaf = leaves - 1;
    let mut tree = vec![0usize; 2 * leaves - 1];
    let mut total = 0usize;
    for &(_, free) in pairs.iter() {
        let mut i = free + first_leaf;
        tree[i] += 1;
        while i > 0 {
            // A left child's right sibling holds every already-inserted edge to this one's right.
            if i % 2 == 1 {
                total += tree[i + 1];
            }
            i = (i - 1) / 2;
            tree[i] += 1;
        }
    }
    total
}

/// Crossings of one layer in candidate `order` against both neighbours, whose positions are fixed
/// in `pos`.
pub fn small_layer_cost(
    order: &[usize],
    pred: &[Vec<usize>],
    succ: &[Vec<usize>],
    pos: &[usize],
    n_layers: usize,
    li: usize,
) -> usize {
    let mut total = 0;
    let mut edges: Vec<(usize, usize)> = Vec::with_capacity(order.len() * 2);
    for (nb, present) in [(pred, li > 0), (succ, li + 1 < n_layers)] {
        if !present {
            continue;
        }
        edges.clear();
        for (nr, &node) in order.iter().enumerate() {
            for &m in &nb[node] {
                edges.push((pos[m], nr));
            }
        }
        total += bilayer_crossings(&mut edges, order.len());
    }
    total
}

/// Exact change in crossings against one neighbouring layer from swapping the adjacent nodes `u`
/// (lower) and `v` (upper). `nb` holds each node's neighbours in that layer. Sum the deltas for
/// both sides to get the full change.
pub fn adjacent_swap_delta(u: usize, v: usize, nb: &[Vec<usize>], pos: &[usize]) -> i32 {
    let mut before = 0i32;
    let mut after = 0i32;
    for &a in &nb[u] {
        for &b in &nb[v] {
            if pos[a] > pos[b] {
                before += 1;
            }
            if pos[a] < pos[b] {
                after += 1;
            }
        }
    }
    after - before
}

/// Sifting. Moves `v` to the best position of its layer by adjacent swaps, summing
/// [`adjacent_swap_delta`] so each step costs `O(deg(v) * deg(w))`. `pred` and `succ` must not
/// hold same-layer arcs. Returns the change in crossings, never positive.
fn sift_node(
    layer: &mut [usize],
    pred: &[Vec<usize>],
    succ: &[Vec<usize>],
    pos: &mut [usize],
    v: usize,
) -> i64 {
    let width = layer.len();
    if width < 2 {
        return 0;
    }
    let (mut cost, mut best_cost, mut at) = (0i64, 0i64, pos[v]);
    let mut best_at = at;
    // Walk down to position 0, then up to the far end. The running total carries across the turn.
    while at > 0 {
        let u = layer[at - 1];
        cost +=
            i64::from(adjacent_swap_delta(u, v, pred, pos) + adjacent_swap_delta(u, v, succ, pos));
        layer.swap(at - 1, at);
        pos[u] = at;
        at -= 1;
        pos[v] = at;
        if cost < best_cost {
            best_cost = cost;
            best_at = at;
        }
    }
    while at + 1 < width {
        let w = layer[at + 1];
        cost +=
            i64::from(adjacent_swap_delta(v, w, pred, pos) + adjacent_swap_delta(v, w, succ, pos));
        layer.swap(at, at + 1);
        pos[w] = at;
        at += 1;
        pos[v] = at;
        if cost < best_cost {
            best_cost = cost;
            best_at = at;
        }
    }
    // Walking back down leaves every other node in its original relative order.
    while at > best_at {
        let u = layer[at - 1];
        layer.swap(at - 1, at);
        pos[u] = at;
        at -= 1;
        pos[v] = at;
    }
    best_cost
}

/// Sifts every node of a layer once, in `sift_order`, usually heaviest degree first so light nodes
/// do not take the slots heavy ones want. Returns the total change, never positive.
fn sift_layer(
    layer: &mut [usize],
    sift_order: &[usize],
    pred: &[Vec<usize>],
    succ: &[Vec<usize>],
    pos: &mut [usize],
) -> i64 {
    let mut gain = 0;
    for &v in sift_order {
        gain += sift_node(layer, pred, succ, pos, v);
    }
    gain
}

/// `pos[node]` for the nodes of one layer.
fn refresh_layer(layer: &[usize], pos: &mut [usize]) {
    for (rank, &node) in layer.iter().enumerate() {
        pos[node] = rank;
    }
}

/// `pos[node]` for every node of the stack.
fn refresh_pos(layers: &[Vec<usize>], pos: &mut [usize]) {
    for layer in layers {
        refresh_layer(layer, pos);
    }
}

/// The sweep key of `node`, its neighbours' positions on one side reduced by [`OrderHeuristic`].
/// A node with no neighbour there keeps its own position so the sweep does not push it to an end.
/// `scratch` is the median's reusable sample buffer.
fn order_key(
    node: usize,
    nb: &[Vec<usize>],
    nb_w: &[Vec<f64>],
    pos: &[usize],
    median: bool,
    scratch: &mut Vec<(f64, f64)>,
) -> f64 {
    let ns = &nb[node];
    if ns.is_empty() {
        return pos[node] as f64;
    }
    let ws = &nb_w[node];
    if median {
        scratch.clear();
        scratch.extend(ns.iter().zip(ws).map(|(&m, &w)| (pos[m] as f64, w)));
        weighted_median(scratch)
    } else {
        let sum: f64 = ns.iter().zip(ws).map(|(&m, &w)| w * pos[m] as f64).sum();
        sum / ws.iter().sum::<f64>()
    }
}

/// Total crossings between adjacent layers. `pos` is each node's index in its layer and `pred`
/// its upper-layer neighbours.
pub fn count_crossings(layers: &[Vec<usize>], pred: &[Vec<usize>], pos: &[usize]) -> usize {
    let mut edges: Vec<(usize, usize)> = vec![];
    let mut total = 0;
    for layer in layers.iter().skip(1) {
        edges.clear();
        for &node in layer {
            for &p in &pred[node] {
                edges.push((pos[p], pos[node]));
            }
        }
        total += bilayer_crossings(&mut edges, layer.len());
    }
    total
}

/// Reorders every layer to minimise crossings between adjacent layers.
pub fn minimize_crossings(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    let mut layers = std::mem::take(&mut st.layers);
    crossing_minimization(&mut layers, st, o);
    st.layers = layers;
    st.reindex();
    Ok(())
}

fn crossing_minimization(layers: &mut Vec<Vec<usize>>, st: &LayeredState, o: &LayoutOptions) {
    let n_layers = layers.len();
    if n_layers <= 1 {
        return;
    }

    let n = st.len();
    let mut succ: Vec<Vec<usize>> = vec![vec![]; n];
    let mut pred: Vec<Vec<usize>> = vec![vec![]; n];
    // Per-neighbour weights, indexed like succ/pred.
    let mut succ_w: Vec<Vec<f64>> = vec![vec![]; n];
    let mut pred_w: Vec<Vec<f64>> = vec![vec![]; n];
    for arc in &st.arcs {
        let (from, to) = (arc.from, arc.to);
        // A self-loop crosses no layer gap, and sifting requires neighbours in other layers.
        if from == to {
            continue;
        }
        let w = st.weight.get(arc.edge).copied().unwrap_or(1.0);
        succ[from].push(to);
        succ_w[from].push(w);
        pred[to].push(from);
        pred_w[to].push(w);
    }

    let layer_of: Vec<usize> = {
        let mut lo = vec![0usize; n];
        for (li, layer) in layers.iter().enumerate() {
            for &v in layer {
                lo[v] = li;
            }
        }
        lo
    };
    // Structural seed as in dagre's `initOrder`: DFS order from `start`, then any unvisited node.
    let structural_seed = |start: &[usize], adj: &[Vec<usize>]| -> Vec<Vec<usize>> {
        let mut new_layers: Vec<Vec<usize>> = vec![vec![]; n_layers];
        let mut seen = vec![false; n];
        let mut stack: Vec<usize> = start.iter().rev().copied().collect();
        while let Some(u) = stack.pop() {
            if seen[u] {
                continue;
            }
            seen[u] = true;
            new_layers[layer_of[u]].push(u);
            for &w in adj[u].iter().rev() {
                if !seen[w] {
                    stack.push(w);
                }
            }
        }
        for (li, layer) in layers.iter().enumerate() {
            for &v in layer {
                if !seen[v] {
                    seen[v] = true;
                    new_layers[li].push(v);
                }
            }
        }
        new_layers
    };
    let dfs_seed = structural_seed(&layers[0], &succ);
    let rev_seed = structural_seed(&layers[n_layers - 1], &pred);
    // Structural seeds plus shuffled restarts.
    let mut dfs_mirror = dfs_seed.clone();
    for l in dfs_mirror.iter_mut() {
        l.reverse();
    }
    let mut seeds: Vec<Vec<Vec<usize>>> = vec![dfs_seed.clone(), dfs_mirror, rev_seed];
    let structural = seeds.len() as u32;
    let mut rng = crate::options::rng_state(o.seed, 0);
    for _ in 0..o.tuning.order.seeds.saturating_sub(structural) {
        let mut s = dfs_seed.clone();
        for layer in s.iter_mut() {
            for i in (1..layer.len()).rev() {
                rng = rng
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let j = (rng >> 33) as usize % (i + 1);
                layer.swap(i, j);
            }
        }
        seeds.push(s);
    }

    // Layer membership and degree never change, so the sift order is fixed for the whole pass.
    let sift_order: Vec<Vec<usize>> = layers
        .iter()
        .map(|layer| {
            let mut ids = layer.clone();
            ids.sort_unstable_by_key_dyn(|&v| {
                (std::cmp::Reverse(pred[v].len() + succ[v].len()), v)
            });
            ids
        })
        .collect();
    // Sifting a layer costs about (arcs on its two sides) squared, checked against
    // `sifting_max_work`.
    let siftable: Vec<bool> = layers
        .iter()
        .map(|layer| {
            if layer.len() <= o.tuning.order.sifting_min_layer {
                return false;
            }
            let e: u64 = layer
                .iter()
                .map(|&v| (pred[v].len() + succ[v].len()) as u64)
                .sum();
            e.saturating_mul(e) <= o.tuning.order.sifting_max_work
        })
        .collect();
    let any_siftable = siftable.iter().any(|&s| s);
    let search = Search {
        o,
        n,
        n_layers,
        widths: layers.iter().map(Vec::len).collect(),
        // Sifting each seed judges it much better than sweeping alone but costs a sift per seed,
        // so the biggest graphs sift only the winner.
        sift_in_sweeps: any_siftable && st.arcs.len() <= o.tuning.order.sifting_sweep_max_arcs,
        pred,
        succ,
        pred_w,
        succ_w,
        sift_order,
        siftable,
    };
    let threads = threads();

    let (mut global_cross, global_best) = search.descend_all(seeds, threads);

    // Sift the winner to convergence. With `sift_in_sweeps` it is already converged.
    *layers = global_best;
    let mut pos: Vec<usize> = vec![0; n];
    refresh_pos(layers, &mut pos);
    if any_siftable {
        // The gain is never positive, so the clamp is only a safeguard.
        let mut touched = vec![false; n_layers];
        let gain = search.refine(layers, &mut pos, search.siftable.clone(), &mut touched);
        global_cross = (global_cross as i64 + gain).max(0) as usize;
    }

    // The count is tracked by arithmetic, so one real count at the end guards against drift.
    let ils_start = layers.clone();
    let ils_start_cross = count_crossings(layers, &search.pred, &pos);
    search.kick_all(layers, &mut pos, global_cross, threads);
    if count_crossings(layers, &search.pred, &pos) > ils_start_cross {
        *layers = ils_start;
    }
}

/// A seed that descends faster than this is not worth a thread.
#[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
const PARALLEL_MIN_SEED: std::time::Duration = std::time::Duration::from_millis(1);

/// Kicks tried in turn before the search decides on threads, and how long they must take for
/// threads to pay: about 5 microseconds a kick.
#[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
const PROBE_KICKS: usize = 64;
#[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
const PARALLEL_MIN_PROBE: std::time::Duration = std::time::Duration::from_micros(320);

/// More threads than this mostly evaluate kicks an earlier acceptance has already made moot.
#[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
const MAX_THREADS: usize = 8;

/// Threads the seeds and the kick search may use: one per core, up to [`MAX_THREADS`]. Whether
/// they do is decided by how long the work turns out to take.
#[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
fn threads() -> usize {
    static CORES: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CORES.get_or_init(|| {
        std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(MAX_THREADS)
    })
}

/// One, without the `parallel` feature or on wasm.
#[cfg(not(all(feature = "parallel", not(target_arch = "wasm32"))))]
fn threads() -> usize {
    1
}

/// What the seeds and kicks of one [`crossing_minimization`] read and never write, so worker
/// threads share it.
struct Search<'a> {
    o: &'a LayoutOptions,
    n: usize,
    n_layers: usize,
    widths: Vec<usize>,
    sift_in_sweeps: bool,
    pred: Vec<Vec<usize>>,
    succ: Vec<Vec<usize>>,
    pred_w: Vec<Vec<f64>>,
    succ_w: Vec<Vec<f64>>,
    sift_order: Vec<Vec<usize>>,
    siftable: Vec<bool>,
}

/// One kick, which reverses `layers[li][from..from + span]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Kick {
    li: usize,
    span: usize,
    from: usize,
}

/// Buffers one kick needs, kept so a kick does not allocate.
struct Lane {
    /// Element `k` of the window wants rank `span - 1 - k`. Bubbling into it reverses the window.
    want: Vec<usize>,
    active: Vec<bool>,
    /// The layers the last kick changed. Only these differ from the order it started from.
    touched: Vec<bool>,
}

impl Lane {
    fn new(n_layers: usize) -> Self {
        Lane {
            want: vec![],
            active: vec![false; n_layers],
            touched: vec![false; n_layers],
        }
    }

    fn touched(&self) -> impl Iterator<Item = usize> + '_ {
        self.touched
            .iter()
            .enumerate()
            .filter(|(_, &t)| t)
            .map(|(li, _)| li)
    }

    /// Undoes the last kick by copying its layers back from `base`. `clone_from` reuses
    /// allocations, and rejection is the common case.
    fn restore(&self, layers: &mut [Vec<usize>], base: &[Vec<usize>], pos: &mut [usize]) {
        for li in self.touched() {
            layers[li].clone_from(&base[li]);
            refresh_layer(&layers[li], pos);
        }
    }

    /// Keeps the last kick by copying its layers into `base`.
    fn commit(&self, layers: &[Vec<usize>], base: &mut [Vec<usize>]) {
        for li in self.touched() {
            base[li].clone_from(&layers[li]);
        }
    }
}

/// The kick search's bookkeeping and random stream. A layer leaves the pool after `kick_patience`
/// rejected kicks in a row. Cheap to clone, so the parallel search can plan a batch ahead.
#[derive(Clone)]
struct Kicks {
    rng: u64,
    rounds: u32,
    patience: u32,
    max_span: usize,
    pool: Vec<usize>,
    in_pool: Vec<bool>,
    reject: Vec<u32>,
    cursor: usize,
}

impl Kicks {
    fn new(o: &LayoutOptions, siftable: &[bool]) -> Self {
        Kicks {
            // A second random stream from the same seed.
            rng: crate::options::rng_state(o.seed, 0x9e37_79b9_7f4a_7c15 ^ 0x2545_f491_4f6c_dd1d),
            rounds: o.tuning.order.kick_rounds,
            patience: o.tuning.order.kick_patience.max(1),
            max_span: o.tuning.order.kick_max_span.max(2),
            pool: (0..siftable.len()).filter(|&li| siftable[li]).collect(),
            in_pool: siftable.to_vec(),
            reject: vec![0; siftable.len()],
            cursor: 0,
        }
    }

    fn next(&mut self, bound: usize) -> usize {
        self.rng = self
            .rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        if bound == 0 {
            0
        } else {
            (self.rng >> 33) as usize % bound
        }
    }

    /// The next kick, `None` once the rounds or the pool run out.
    fn draw(&mut self, widths: &[usize]) -> Option<Kick> {
        if self.rounds == 0 || self.pool.is_empty() {
            return None;
        }
        self.rounds -= 1;
        if self.cursor >= self.pool.len() {
            self.cursor = 0;
        }
        let li = self.pool[self.cursor];
        let width = widths[li];
        let span = 2 + self.next(self.max_span.min(width) - 1);
        let from = self.next(width + 1 - span);
        Some(Kick { li, span, from })
    }

    fn rejected(&mut self, li: usize) {
        self.reject[li] += 1;
        if self.reject[li] >= self.patience {
            self.in_pool[li] = false;
            self.pool.remove(self.cursor);
        } else {
            self.cursor += 1;
        }
    }

    fn accepted(&mut self, li: usize, siftable: &[bool]) {
        for nb in around(li, siftable.len()) {
            self.reject[nb] = 0;
            if siftable[nb] && !self.in_pool[nb] {
                self.in_pool[nb] = true;
                self.pool.push(nb);
            }
        }
        self.cursor += 1;
    }
}

/// A layer and its neighbours, the layers one kick re-sifts.
fn around(li: usize, n_layers: usize) -> [usize; 3] {
    [li.saturating_sub(1), li, (li + 1).min(n_layers - 1)]
}

impl Search<'_> {
    /// Sifts until a round moves nothing, waking a settled layer when a neighbour moves. Returns
    /// the exact change in crossings and marks every layer it sifted in `touched`.
    fn refine(
        &self,
        layers: &mut [Vec<usize>],
        pos: &mut [usize],
        mut active: Vec<bool>,
        touched: &mut [bool],
    ) -> i64 {
        let n_layers = self.n_layers;
        let mut gain = 0i64;
        for _ in 0..self.o.tuning.order.sifting_rounds {
            let mut woken = vec![false; n_layers];
            let mut moved = false;
            for li in 0..n_layers {
                if !active[li] {
                    continue;
                }
                let layer_gain = sift_layer(
                    &mut layers[li],
                    &self.sift_order[li],
                    &self.pred,
                    &self.succ,
                    pos,
                );
                touched[li] = true;
                gain += layer_gain;
                if layer_gain < 0 {
                    moved = true;
                    woken[li.saturating_sub(1)] = true;
                    woken[(li + 1).min(n_layers - 1)] = true;
                } else {
                    active[li] = false;
                }
            }
            if !moved {
                break;
            }
            for li in 0..n_layers {
                active[li] |= woken[li] && self.siftable[li];
            }
        }
        gain
    }

    /// Descends every seed. The first with the fewest crossings wins, as if they ran in turn, and
    /// one that reaches zero crossings ends the search.
    fn descend_all(&self, seeds: Vec<Vec<Vec<usize>>>, threads: usize) -> (usize, Vec<Vec<usize>>) {
        let mut seeds = seeds.into_iter();
        let Some(first) = seeds.next() else {
            return (usize::MAX, vec![]);
        };
        #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
        let clock = std::time::Instant::now();
        let mut best = self.descend(first);
        // The rest go to threads only when one seed takes long enough to pay for them. The result
        // is the same either way.
        #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
        if threads > 1 && best.0 > 0 && clock.elapsed() >= PARALLEL_MIN_SEED {
            let rest: Vec<Vec<Vec<usize>>> = seeds.collect();
            let other = self.descend_parallel(&rest, threads);
            return if other.0 < best.0 { other } else { best };
        }
        let _ = threads;
        for seed in seeds {
            if best.0 == 0 {
                break; // an earlier seed is already crossing-free
            }
            let (cross, layers) = self.descend(seed);
            if cross < best.0 {
                best = (cross, layers);
            }
        }
        best
    }

    #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
    fn descend_parallel(
        &self,
        seeds: &[Vec<Vec<usize>>],
        threads: usize,
    ) -> (usize, Vec<Vec<usize>>) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Mutex;
        let next = AtomicUsize::new(0);
        // The first crossing-free seed. The sequential search would never run a later one.
        let zero_at = AtomicUsize::new(usize::MAX);
        // (crossings, seed index, order)
        type Descent = (usize, usize, Vec<Vec<usize>>);
        let done: Mutex<Vec<Descent>> = Mutex::new(vec![]);
        let work = || loop {
            let i = next.fetch_add(1, Ordering::Relaxed);
            if i >= seeds.len() || i > zero_at.load(Ordering::Relaxed) {
                break;
            }
            let (cross, layers) = self.descend(seeds[i].clone());
            if cross == 0 {
                zero_at.fetch_min(i, Ordering::Relaxed);
            }
            done.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((cross, i, layers));
        };
        std::thread::scope(|s| {
            for _ in 1..threads.min(seeds.len()) {
                s.spawn(work);
            }
            work();
        });
        let done = done
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        done.into_iter()
            .min_by_key(|&(cross, i, _)| (cross, i))
            .map_or((usize::MAX, vec![]), |(cross, _, layers)| (cross, layers))
    }

    /// Sweeps, transposes, sifts and permutes one seed until sweeps stop paying. Returns the best
    /// order it saw and its crossings.
    fn descend(&self, mut layers: Vec<Vec<usize>>) -> (usize, Vec<Vec<usize>>) {
        let (o, n_layers) = (self.o, self.n_layers);
        let (pred, succ) = (&self.pred, &self.succ);
        let stall_limit = o.tuning.order.sweep_stall.max(1);
        let mut pos: Vec<usize> = vec![0; self.n];
        let mut key: Vec<f64> = vec![0.0; self.n];
        let mut scratch: Vec<(f64, f64)> = vec![];
        let mut touched: Vec<bool> = vec![false; n_layers];
        refresh_pos(&layers, &mut pos);
        let mut best = layers.clone();
        let mut best_cross = count_crossings(&layers, pred, &pos);
        let mut stall = 0u32;

        for sweep in 0..o.tuning.order.sweeps {
            if best_cross == 0 {
                break; // crossing-free, nothing to improve
            }
            // Ties keep their current order via `pos`, so a sweep does not undo the exact passes.
            let median = match o.order.heuristic {
                OrderHeuristic::Barycenter => false,
                OrderHeuristic::Median => true,
                OrderHeuristic::Alternate => sweep % 2 == 1,
            };
            for layer in layers.iter_mut().skip(1) {
                for &v in layer.iter() {
                    key[v] = order_key(v, pred, &self.pred_w, &pos, median, &mut scratch);
                }
                layer.sort_unstable_by_dyn(|&a, &b| {
                    key[a].total_cmp(&key[b]).then(pos[a].cmp(&pos[b]))
                });
                refresh_layer(layer, &mut pos);
            }
            for layer in layers[..n_layers - 1].iter_mut().rev() {
                for &v in layer.iter() {
                    key[v] = order_key(v, succ, &self.succ_w, &pos, median, &mut scratch);
                }
                layer.sort_unstable_by_dyn(|&a, &b| {
                    key[a].total_cmp(&key[b]).then(pos[a].cmp(&pos[b]))
                });
                refresh_layer(layer, &mut pos);
            }

            // Greedy transposition. Sifting subsumes it but does not run on every layer, and this
            // is a cheap first descent where it does.
            let mut improved = true;
            while improved {
                improved = false;
                for layer in layers.iter_mut() {
                    for k in 0..layer.len().saturating_sub(1) {
                        let u = layer[k];
                        let v = layer[k + 1];
                        let delta = adjacent_swap_delta(u, v, pred, &pos)
                            + adjacent_swap_delta(u, v, succ, &pos);
                        if delta < 0 {
                            layer.swap(k, k + 1);
                            pos[u] = k + 1;
                            pos[v] = k;
                            improved = true;
                        }
                    }
                }
            }

            if self.sift_in_sweeps {
                self.refine(&mut layers, &mut pos, self.siftable.clone(), &mut touched);
            }

            // Exhaustive search is optimal for the whole layer, sifting only per node. The allow is
            // because `li` is also an argument of the cost function.
            #[allow(clippy::needless_range_loop)]
            for li in 0..n_layers {
                let len = layers[li].len();
                if !(2..=o.tuning.order.exhaustive_layer_max).contains(&len) {
                    continue;
                }
                let mut best_order = layers[li].clone();
                let mut best_c = small_layer_cost(&best_order, pred, succ, &pos, n_layers, li);
                if best_c == 0 {
                    continue; // already crossing-free
                }
                let mut perm = layers[li].clone();
                for_each_perm(&mut perm, &mut |p| {
                    let c = small_layer_cost(p, pred, succ, &pos, n_layers, li);
                    if c < best_c {
                        best_c = c;
                        best_order = p.to_vec();
                    }
                });
                if best_order != layers[li] {
                    layers[li] = best_order;
                    refresh_layer(&layers[li], &mut pos);
                }
            }

            let c = count_crossings(&layers, pred, &pos);
            if c < best_cross {
                best_cross = c;
                best = layers.clone();
                stall = 0;
            } else {
                // Stalled sweeps cost time that is better spent on the next seed.
                stall += 1;
                if stall >= stall_limit {
                    break;
                }
            }
        }
        (best_cross, best)
    }

    /// One step of iterated local search: reverses a short run of one layer and re-sifts it and its
    /// neighbours, which escapes orders where only a group of nodes wants to move. Returns the
    /// change in crossings and marks the changed layers in `lane` for [`Lane::restore`] or
    /// [`Lane::commit`].
    fn kick(&self, layers: &mut [Vec<usize>], pos: &mut [usize], k: Kick, lane: &mut Lane) -> i64 {
        let Kick { li, span, from } = k;
        let (pred, succ) = (&self.pred, &self.succ);
        lane.want.clear();
        lane.want.extend((0..span).rev());
        let mut delta = 0i64;
        let mut settled = false;
        while !settled {
            settled = true;
            for k in 0..span - 1 {
                if lane.want[k] > lane.want[k + 1] {
                    let (u, v) = (layers[li][from + k], layers[li][from + k + 1]);
                    delta += i64::from(
                        adjacent_swap_delta(u, v, pred, pos) + adjacent_swap_delta(u, v, succ, pos),
                    );
                    layers[li].swap(from + k, from + k + 1);
                    pos[u] = from + k + 1;
                    pos[v] = from + k;
                    lane.want.swap(k, k + 1);
                    settled = false;
                }
            }
        }
        lane.active.iter_mut().for_each(|a| *a = false);
        for nb in around(li, self.n_layers) {
            lane.active[nb] = self.siftable[nb];
        }
        lane.touched.iter_mut().for_each(|t| *t = false);
        lane.touched[li] = true;
        delta + self.refine(layers, pos, lane.active.clone(), &mut lane.touched)
    }

    /// Kicks `layers`, at `cross` crossings, until the rounds or the pool run out, and leaves the
    /// best order found in `layers`.
    fn kick_all(&self, layers: &mut [Vec<usize>], pos: &mut [usize], cross: usize, threads: usize) {
        let mut plan = Kicks::new(self.o, &self.siftable);
        let mut base = layers.to_vec();
        // A few kicks in turn tell whether a kick takes long enough to pay for threads. The result
        // is the same either way.
        #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
        let cross = if threads > 1 {
            let clock = std::time::Instant::now();
            let cross = self.kick_serial(layers, &mut base, pos, cross, &mut plan, PROBE_KICKS);
            if cross > 0 && clock.elapsed() >= PARALLEL_MIN_PROBE {
                return self.kick_parallel(layers, pos, cross, plan, threads);
            }
            cross
        } else {
            cross
        };
        let _ = threads;
        self.kick_serial(layers, &mut base, pos, cross, &mut plan, usize::MAX);
    }

    /// Up to `limit` kicks in turn, from `layers` equal to `base` at `cross` crossings. Returns
    /// the crossings after them.
    fn kick_serial(
        &self,
        layers: &mut [Vec<usize>],
        base: &mut [Vec<usize>],
        pos: &mut [usize],
        mut cross: usize,
        plan: &mut Kicks,
        limit: usize,
    ) -> usize {
        let mut lane = Lane::new(self.n_layers);
        for _ in 0..limit {
            if cross == 0 {
                break;
            }
            let Some(k) = plan.draw(&self.widths) else {
                break;
            };
            let delta = self.kick(layers, pos, k, &mut lane);
            if delta < 0 {
                cross = (cross as i64 + delta).max(0) as usize;
                lane.commit(layers, base);
                plan.accepted(k.li, &self.siftable);
            } else {
                lane.restore(layers, base, pos);
                plan.rejected(k.li);
            }
        }
        cross
    }

    /// [`Search::kick_serial`] on `threads` threads, with the same result. Almost every kick is
    /// rejected, so a batch is drawn ahead assuming none is accepted and tried from the same order.
    /// The first accepted kick wins and the tries after it are dropped. A worker panic releases the
    /// others before it is re-raised here.
    #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
    fn kick_parallel(
        &self,
        layers: &mut [Vec<usize>],
        pos: &mut [usize],
        cross: usize,
        plan: Kicks,
        threads: usize,
    ) {
        use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Barrier, Mutex, PoisonError, RwLock};

        /// One thread's copy of the order, `layers` as kicked and `base` as it was.
        struct Worker {
            layers: Vec<Vec<usize>>,
            base: Vec<Vec<usize>>,
            pos: Vec<usize>,
            lane: Lane,
        }
        /// What the workers read during a batch. Written only while they wait to start one.
        struct Batch {
            /// The layers the last accepted kick changed, for every worker to take on first.
            changed: Vec<(usize, Vec<usize>)>,
            kicks: Vec<Kick>,
            stop: bool,
        }
        type Accepted = (usize, i64, Vec<(usize, Vec<usize>)>);

        let mut workers: Vec<Worker> = (0..threads)
            .map(|_| Worker {
                layers: layers.to_vec(),
                base: layers.to_vec(),
                pos: pos.to_vec(),
                lane: Lane::new(self.n_layers),
            })
            .collect();
        let batch = RwLock::new(Batch {
            changed: vec![],
            kicks: vec![],
            stop: false,
        });
        let next = AtomicUsize::new(0);
        let first = AtomicUsize::new(usize::MAX);
        let accepted: Mutex<Vec<Accepted>> = Mutex::new(vec![]);
        let panicked: Mutex<Option<Box<dyn std::any::Any + Send>>> = Mutex::new(None);
        let (start, end) = (Barrier::new(threads), Barrier::new(threads));

        let run = |w: &mut Worker, b: &Batch| {
            for (li, layer) in &b.changed {
                w.layers[*li].clone_from(layer);
                w.base[*li].clone_from(layer);
                refresh_layer(layer, &mut w.pos);
            }
            loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= b.kicks.len() || i > first.load(Ordering::Relaxed) {
                    break;
                }
                let delta = self.kick(&mut w.layers, &mut w.pos, b.kicks[i], &mut w.lane);
                if delta < 0 {
                    first.fetch_min(i, Ordering::Relaxed);
                    let kept = w
                        .lane
                        .touched()
                        .map(|li| (li, w.layers[li].clone()))
                        .collect();
                    accepted
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push((i, delta, kept));
                }
                w.lane.restore(&mut w.layers, &w.base, &mut w.pos);
            }
        };
        // Runs one batch share, keeping a panic for the main thread to raise.
        let share = |w: &mut Worker| {
            let b = batch.read().unwrap_or_else(PoisonError::into_inner);
            if let Err(p) = catch_unwind(AssertUnwindSafe(|| run(w, &b))) {
                *panicked.lock().unwrap_or_else(PoisonError::into_inner) = Some(p);
            }
        };

        std::thread::scope(|s| {
            let (mine, others) = workers.split_at_mut(1);
            for w in others {
                s.spawn(|| loop {
                    start.wait();
                    if batch.read().unwrap_or_else(PoisonError::into_inner).stop {
                        break;
                    }
                    share(w);
                    end.wait();
                });
            }
            let mine = &mut mine[0];
            let (mut cross, mut plan) = (cross, plan);
            let batch_len = 32 * threads;
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                while cross > 0 {
                    let mut ahead = plan.clone();
                    let mut kicks: Vec<Kick> = Vec::with_capacity(batch_len);
                    while kicks.len() < batch_len {
                        let Some(k) = ahead.draw(&self.widths) else {
                            break;
                        };
                        ahead.rejected(k.li);
                        kicks.push(k);
                    }
                    if kicks.is_empty() {
                        return;
                    }
                    batch.write().unwrap_or_else(PoisonError::into_inner).kicks = kicks;
                    next.store(0, Ordering::Relaxed);
                    first.store(usize::MAX, Ordering::Relaxed);
                    start.wait();
                    share(mine);
                    end.wait();
                    if let Some(p) = panicked
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .take()
                    {
                        resume_unwind(p);
                    }
                    let found = std::mem::take(
                        &mut *accepted.lock().unwrap_or_else(PoisonError::into_inner),
                    );
                    let mut b = batch.write().unwrap_or_else(PoisonError::into_inner);
                    b.changed.clear();
                    let Some((i, delta, kept)) = found.into_iter().min_by_key(|a| a.0) else {
                        plan = ahead;
                        continue;
                    };
                    // Replay the bookkeeping with the kicks before `i` rejected and `i` accepted.
                    for _ in 0..i {
                        if let Some(k) = plan.draw(&self.widths) {
                            plan.rejected(k.li);
                        }
                    }
                    if let Some(k) = plan.draw(&self.widths) {
                        plan.accepted(k.li, &self.siftable);
                    }
                    cross = (cross as i64 + delta).max(0) as usize;
                    for (li, layer) in &kept {
                        layers[*li].clone_from(layer);
                        refresh_layer(layer, pos);
                    }
                    b.changed = kept;
                }
            }));
            // Every worker is waiting at `start`. Release them before anything is raised.
            batch.write().unwrap_or_else(PoisonError::into_inner).stop = true;
            start.wait();
            if let Err(p) = outcome {
                resume_unwind(p);
            }
        });
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::geometry::shape::NodeShape;
    use crate::layered::dummy::insert_dummies;
    use crate::layered::prepare::{LayeredInput, LayeredInputNode};

    /// A state of `n_layers` layers of `per_layer` nodes, with node `li * per_layer + k` in layer
    /// `li`, carrying `edges` in the order `layers` gives.
    fn stack(
        n_layers: usize,
        per_layer: usize,
        edges: &[(usize, usize)],
        layers: &[Vec<usize>],
    ) -> LayeredState {
        let n = n_layers * per_layer;
        let input = LayeredInput {
            nodes: vec![
                LayeredInputNode {
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
                };
                n
            ],
            edges: edges.to_vec(),
            weights: vec![1.0; edges.len()],
            thickness: vec![],
            edge_label_sizes: vec![],
            seed: vec![],
            pinned: vec![],
            self_loops: vec![],
            ranked_backwards: Vec::new(),
            options: LayoutOptions::layered_default(),
        };
        let mut st = LayeredState::new(&input);
        for (i, nd) in st.nodes.iter_mut().enumerate() {
            nd.rank = (i / per_layer) as i32;
        }
        insert_dummies(&mut st).unwrap();
        st.rebuild_layers();
        st.layers = layers.to_vec();
        st.reindex();
        st
    }

    fn crossings_of(st: &LayeredState) -> usize {
        count_crossings(&st.layers, &st.adjacency().0, &st.order_of)
    }

    /// A planar bilayer, handed to the pass reversed, ends with zero crossings.
    #[test]
    fn a_planar_bilayer_is_untangled_completely() {
        let per = 8;
        let mut edges = vec![];
        for k in 0..per {
            edges.push((k, per + k));
            if k + 1 < per {
                edges.push((k, per + k + 1));
            }
        }
        let layers = vec![
            (0..per).collect::<Vec<_>>(),
            (per..2 * per).rev().collect::<Vec<_>>(),
        ];
        let mut st = stack(2, per, &edges, &layers);
        let o = LayoutOptions::layered_default();
        assert!(
            crossings_of(&st) > 0,
            "the starting order has to be tangled"
        );
        minimize_crossings(&mut st, &o).unwrap();
        assert_eq!(crossings_of(&st), 0);
    }

    /// A complete bilayer has `C(k,2)^2` crossings in every order, before and after the pass.
    #[test]
    fn a_complete_bilayer_costs_the_same_in_every_order() {
        let per = 5;
        let edges: Vec<(usize, usize)> = (0..per)
            .flat_map(|a| (0..per).map(move |b| (a, per + b)))
            .collect();
        let pairs = per * (per - 1) / 2;
        for start in [
            vec![(0..per).collect::<Vec<_>>(), (per..2 * per).collect()],
            vec![(0..per).rev().collect(), (per..2 * per).collect()],
            vec![vec![2, 0, 4, 1, 3], vec![7, 5, 9, 6, 8]],
        ] {
            let mut st = stack(2, per, &edges, &start);
            assert_eq!(crossings_of(&st), pairs * pairs);
            minimize_crossings(&mut st, &LayoutOptions::layered_default()).unwrap();
            assert_eq!(crossings_of(&st), pairs * pairs);
        }
    }

    /// The same input gives the same order.
    #[test]
    fn the_same_input_gives_the_same_order() {
        let (n_layers, per) = (5, 7);
        let mut rng = Lcg(0x7f4a_7c15_9e37_79b9);
        let mut edges = vec![];
        for _ in 0..90 {
            let li = rng.below(n_layers - 1);
            let to_li = (li + 1 + rng.below(2)).min(n_layers - 1);
            edges.push((li * per + rng.below(per), to_li * per + rng.below(per)));
        }
        let layers: Vec<Vec<usize>> = (0..n_layers)
            .map(|li| (li * per..li * per + per).collect())
            .collect();
        let o = LayoutOptions::layered_default();
        let mut first = stack(n_layers, per, &edges, &layers);
        minimize_crossings(&mut first, &o).unwrap();
        for _ in 0..3 {
            let mut again = stack(n_layers, per, &edges, &layers);
            minimize_crossings(&mut again, &o).unwrap();
            assert_eq!(first.layers, again.layers);
            assert_eq!(first.order_of, again.order_of);
        }
        // The pass must change something, or the check above compares two no-ops.
        let mut untouched = stack(n_layers, per, &edges, &layers);
        untouched.reindex();
        assert!(crossings_of(&first) < crossings_of(&untouched));
    }

    /// Pairwise reference for the accumulator tree.
    fn count_crossings_pairwise(
        layers: &[Vec<usize>],
        pred: &[Vec<usize>],
        pos: &[usize],
    ) -> usize {
        let mut inter: Vec<(usize, usize)> = vec![];
        let mut total = 0;
        for layer in &layers[1..] {
            inter.clear();
            for &node in layer {
                for &p in &pred[node] {
                    inter.push((pos[p], pos[node]));
                }
            }
            for i in 0..inter.len() {
                for j in i + 1..inter.len() {
                    let (a, b) = (inter[i], inter[j]);
                    if (a.0 < b.0 && a.1 > b.1) || (a.0 > b.0 && a.1 < b.1) {
                        total += 1;
                    }
                }
            }
        }
        total
    }

    /// Pairwise reference for [`small_layer_cost`].
    fn small_layer_cost_pairwise(
        order: &[usize],
        pred: &[Vec<usize>],
        succ: &[Vec<usize>],
        pos: &[usize],
        n_layers: usize,
        li: usize,
    ) -> usize {
        let mut total = 0;
        for (nb, present) in [(pred, li > 0), (succ, li + 1 < n_layers)] {
            if !present {
                continue;
            }
            let mut inter: Vec<(usize, usize)> = Vec::with_capacity(order.len() * 2);
            for (nr, &node) in order.iter().enumerate() {
                for &m in &nb[node] {
                    inter.push((pos[m], nr));
                }
            }
            for i in 0..inter.len() {
                for j in i + 1..inter.len() {
                    let (a, b) = (inter[i], inter[j]);
                    if (a.0 < b.0 && a.1 > b.1) || (a.0 > b.0 && a.1 < b.1) {
                        total += 1;
                    }
                }
            }
        }
        total
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

    /// `(layers, pred, succ, pos)`, built by [`random_layers`].
    type RandomLayers = (
        Vec<Vec<usize>>,
        Vec<Vec<usize>>,
        Vec<Vec<usize>>,
        Vec<usize>,
    );

    /// A random layered graph with `sizes[li]` nodes in layer `li` and `edges_per_layer` random
    /// edges between adjacent layers. Duplicates and shared endpoints are allowed, as in real
    /// states.
    fn random_layers(rng: &mut Lcg, sizes: &[usize], edges_per_layer: usize) -> RandomLayers {
        let n: usize = sizes.iter().sum();
        let mut layers: Vec<Vec<usize>> = vec![];
        let mut next = 0usize;
        for &s in sizes {
            layers.push((next..next + s).collect());
            next += s;
        }
        // Shuffle so a node's index and its position are unrelated.
        for layer in layers.iter_mut() {
            for i in (1..layer.len()).rev() {
                let j = rng.below(i + 1);
                layer.swap(i, j);
            }
        }
        let mut pos = vec![0usize; n];
        for layer in &layers {
            for (r, &v) in layer.iter().enumerate() {
                pos[v] = r;
            }
        }
        let mut pred: Vec<Vec<usize>> = vec![vec![]; n];
        let mut succ: Vec<Vec<usize>> = vec![vec![]; n];
        for li in 1..layers.len() {
            for _ in 0..edges_per_layer {
                let up = layers[li - 1][rng.below(layers[li - 1].len())];
                let down = layers[li][rng.below(layers[li].len())];
                pred[down].push(up);
                succ[up].push(down);
            }
        }
        (layers, pred, succ, pos)
    }

    /// A [`Search`] over random layers, with every layer siftable and weights of 1.
    #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
    fn random_search<'a>(rng: &mut Lcg, o: &'a LayoutOptions) -> (Search<'a>, Vec<Vec<usize>>) {
        let sizes: Vec<usize> = (0..6).map(|_| 6 + rng.below(14)).collect();
        let (layers, pred, succ, _) = random_layers(rng, &sizes, 30);
        let ones = |nb: &[Vec<usize>]| nb.iter().map(|l| vec![1.0; l.len()]).collect();
        let search = Search {
            o,
            n: sizes.iter().sum(),
            n_layers: layers.len(),
            widths: layers.iter().map(Vec::len).collect(),
            sift_in_sweeps: true,
            pred_w: ones(&pred),
            succ_w: ones(&succ),
            pred,
            succ,
            sift_order: layers.clone(),
            siftable: vec![true; layers.len()],
        };
        (search, layers)
    }

    /// The kicks tried ahead on four threads keep exactly the order the kicks in turn keep.
    #[test]
    #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
    fn the_parallel_kick_search_matches_the_serial_one() {
        let o = LayoutOptions::layered_default();
        let mut rng = Lcg(0x2545_f491_4f6c_dd1d);
        let mut improved = 0;
        for case in 0..12 {
            let (search, start) = random_search(&mut rng, &o);
            let mut pos = vec![0; search.n];
            refresh_pos(&start, &mut pos);
            let cross = count_crossings(&start, &search.pred, &pos);

            let (mut serial, mut serial_pos) = (start.clone(), pos.clone());
            let mut base = start.clone();
            let mut plan = Kicks::new(&o, &search.siftable);
            search.kick_serial(
                &mut serial,
                &mut base,
                &mut serial_pos,
                cross,
                &mut plan,
                usize::MAX,
            );

            let (mut parallel, mut parallel_pos) = (start.clone(), pos.clone());
            let plan = Kicks::new(&o, &search.siftable);
            search.kick_parallel(&mut parallel, &mut parallel_pos, cross, plan, 4);

            assert_eq!(parallel, serial, "case {case}");
            assert_eq!(parallel_pos, serial_pos, "case {case}");
            if count_crossings(&serial, &search.pred, &serial_pos) < cross {
                improved += 1;
            }
        }
        // Only accepted kicks can tell the two apart.
        assert!(improved >= 6, "only {improved} of 12 cases accepted a kick");
    }

    /// Seeds descended on four threads pick the seed the sequential loop picks.
    #[test]
    #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
    fn parallel_seeds_pick_the_serial_winner() {
        let o = LayoutOptions::layered_default();
        let mut rng = Lcg(0x5851_f42d_4c95_7f2d);
        for case in 0..12 {
            let (search, start) = random_search(&mut rng, &o);
            let seeds: Vec<Vec<Vec<usize>>> = (0..9)
                .map(|_| {
                    let mut s = start.clone();
                    for layer in s.iter_mut() {
                        for i in (1..layer.len()).rev() {
                            layer.swap(i, rng.below(i + 1));
                        }
                    }
                    s
                })
                .collect();
            let serial = search.descend_all(seeds.clone(), 1);
            let parallel = search.descend_parallel(&seeds, 4);
            assert_eq!(parallel, serial, "case {case}");
        }
    }

    /// The crossing count matches the pairwise count on random layers, small and large.
    #[test]
    fn bjm_count_matches_the_pairwise_count() {
        let mut rng = Lcg(0x9e37_79b9_7f4a_7c15);
        for case in 0..300 {
            let sizes: Vec<usize> = (0..2 + case % 4).map(|_| 1 + rng.below(30)).collect();
            let edges = rng.below(60);
            let (layers, pred, _succ, pos) = random_layers(&mut rng, &sizes, edges);
            assert_eq!(
                count_crossings(&layers, &pred, &pos),
                count_crossings_pairwise(&layers, &pred, &pos),
                "case {case}, sizes {sizes:?}, {edges} edges per bilayer"
            );
        }
    }

    /// [`small_layer_cost`] matches the pairwise count at every layer index.
    #[test]
    fn small_layer_cost_matches_the_pairwise_count() {
        let mut rng = Lcg(0x2545_f491_4f6c_dd1d);
        for case in 0..300 {
            let sizes: Vec<usize> = (0..3).map(|_| 1 + rng.below(20)).collect();
            let edges = rng.below(50);
            let (layers, pred, succ, pos) = random_layers(&mut rng, &sizes, edges);
            for li in 0..layers.len() {
                assert_eq!(
                    small_layer_cost(&layers[li], &pred, &succ, &pos, layers.len(), li),
                    small_layer_cost_pairwise(&layers[li], &pred, &succ, &pos, layers.len(), li),
                    "case {case}, layer {li}, sizes {sizes:?}, {edges} edges per bilayer"
                );
            }
        }
    }

    /// [`adjacent_swap_delta`] equals the difference in the full count.
    #[test]
    #[allow(clippy::needless_range_loop)] // the loop swaps inside the layer it indexes
    fn adjacent_swap_delta_matches_the_full_count() {
        let mut rng = Lcg(0x1234_5678_9abc_def1);
        for case in 0..200 {
            let sizes: Vec<usize> = (0..3).map(|_| 2 + rng.below(20)).collect();
            let edges = rng.below(50);
            let (mut layers, pred, succ, mut pos) = random_layers(&mut rng, &sizes, edges);
            let n_layers = layers.len();
            for li in 0..n_layers {
                let width = layers[li].len();
                for k in 0..width - 1 {
                    let (u, v) = (layers[li][k], layers[li][k + 1]);
                    let before = small_layer_cost(&layers[li], &pred, &succ, &pos, n_layers, li);
                    let delta = adjacent_swap_delta(u, v, &pred, &pos)
                        + adjacent_swap_delta(u, v, &succ, &pos);
                    layers[li].swap(k, k + 1);
                    pos[u] = k + 1;
                    pos[v] = k;
                    let after = small_layer_cost(&layers[li], &pred, &succ, &pos, n_layers, li);
                    assert_eq!(
                        after as i64 - before as i64,
                        delta as i64,
                        "case {case}, layer {li}, swap at {k}"
                    );
                    layers[li].swap(k, k + 1);
                    pos[u] = k;
                    pos[v] = k + 1;
                }
            }
        }
    }

    /// Parallel edges and edges sharing an endpoint are not crossings.
    #[test]
    fn ties_are_not_crossings() {
        let mut parallel = vec![(0usize, 0usize); 40];
        assert_eq!(bilayer_crossings(&mut parallel, 1), 0);
        let mut fan: Vec<(usize, usize)> = (0..40).map(|i| (0, i)).collect();
        assert_eq!(bilayer_crossings(&mut fan, 40), 0);
        let mut inverted: Vec<(usize, usize)> = (0..40).map(|i| (i, 39 - i)).collect();
        assert_eq!(bilayer_crossings(&mut inverted, 40), 40 * 39 / 2);
    }

    /// Greedy adjacent transposition to a fixpoint on one layer, to compare against sifting.
    fn transpose_to_fixpoint(
        layer: &mut [usize],
        pred: &[Vec<usize>],
        succ: &[Vec<usize>],
        pos: &mut [usize],
    ) {
        let mut improved = true;
        while improved {
            improved = false;
            for k in 0..layer.len().saturating_sub(1) {
                let (u, v) = (layer[k], layer[k + 1]);
                if adjacent_swap_delta(u, v, pred, pos) + adjacent_swap_delta(u, v, succ, pos) < 0 {
                    layer.swap(k, k + 1);
                    pos[u] = k + 1;
                    pos[v] = k;
                    improved = true;
                }
            }
        }
    }

    /// Sifting takes a two-step move whose first step costs a crossing. Node 5 pays one crossing to
    /// pass node 6 and saves two passing node 7, so transposition stays put.
    #[test]
    fn sifting_takes_the_two_step_move_transposition_cannot() {
        // Fixed layer 0..=4 in index order, free layer [5, 6, 7].
        let mut pred: Vec<Vec<usize>> = vec![vec![]; 8];
        pred[5] = vec![1, 1, 4];
        pred[6] = vec![2];
        pred[7] = vec![0, 3];
        let succ: Vec<Vec<usize>> = vec![vec![]; 8];
        let layers = vec![vec![0, 1, 2, 3, 4], vec![5, 6, 7]];
        let mut pos = vec![0usize; 8];
        refresh_pos(&layers, &mut pos);
        let start = small_layer_cost(&layers[1], &pred, &succ, &pos, 2, 1);
        assert_eq!(start, 6);

        let mut transposed = layers[1].clone();
        let mut tpos = pos.clone();
        transpose_to_fixpoint(&mut transposed, &pred, &succ, &mut tpos);
        assert_eq!(
            transposed,
            vec![5, 6, 7],
            "no single swap pays, so nothing moves"
        );
        assert_eq!(
            small_layer_cost(&transposed, &pred, &succ, &tpos, 2, 1),
            start
        );

        let mut sifted = layers[1].clone();
        let mut spos = pos.clone();
        assert_eq!(sift_node(&mut sifted, &pred, &succ, &mut spos, 5), -1);
        assert_eq!(sifted, vec![6, 7, 5]);
        assert_eq!(
            small_layer_cost(&sifted, &pred, &succ, &spos, 2, 1),
            start - 1
        );
    }

    /// On random bilayers, sifting is never worse than transposition from the same start and is
    /// often better.
    #[test]
    fn sifting_never_does_worse_than_transposition() {
        let mut rng = Lcg(0x5deb_ce2d_3c65_79a1);
        let mut strictly_better = 0;
        for case in 0..200 {
            let sizes = vec![2 + rng.below(14), 2 + rng.below(14)];
            let edges = 4 + rng.below(30);
            let (layers, pred, succ, pos) = random_layers(&mut rng, &sizes, edges);
            let sift_order = {
                let mut ids = layers[1].clone();
                ids.sort_unstable_by_key_dyn(|&v| (std::cmp::Reverse(pred[v].len()), v));
                ids
            };
            let mut transposed = layers[1].clone();
            let mut tpos = pos.clone();
            transpose_to_fixpoint(&mut transposed, &pred, &succ, &mut tpos);
            let t = small_layer_cost(&transposed, &pred, &succ, &tpos, 2, 1);

            let mut sifted = layers[1].clone();
            let mut spos = pos.clone();
            while sift_layer(&mut sifted, &sift_order, &pred, &succ, &mut spos) < 0 {}
            let s = small_layer_cost(&sifted, &pred, &succ, &spos, 2, 1);
            assert!(s <= t, "case {case}: sifting {s} lost to transposition {t}");
            if s < t {
                strictly_better += 1;
            }
        }
        assert!(
            strictly_better > 20,
            "only {strictly_better} of 200 bilayers improved; the comparison has stopped biting"
        );
    }

    /// [`sift_node`] returns the exact change in the real count.
    #[test]
    #[allow(clippy::needless_range_loop)] // the index is the cost function's own argument
    fn sift_node_returns_the_exact_change() {
        let mut rng = Lcg(0x4d59_5df4_d0f3_3173);
        for case in 0..150 {
            let sizes: Vec<usize> = (0..3).map(|_| 2 + rng.below(12)).collect();
            let edges = rng.below(40);
            let (mut layers, pred, succ, mut pos) = random_layers(&mut rng, &sizes, edges);
            let n_layers = layers.len();
            for li in 0..n_layers {
                for vi in 0..layers[li].len() {
                    let v = layers[li][vi];
                    let before = small_layer_cost(&layers[li], &pred, &succ, &pos, n_layers, li);
                    let gain = sift_node(&mut layers[li], &pred, &succ, &mut pos, v);
                    let after = small_layer_cost(&layers[li], &pred, &succ, &pos, n_layers, li);
                    assert_eq!(
                        after as i64 - before as i64,
                        gain,
                        "case {case}, layer {li}, node {v}"
                    );
                    assert!(gain <= 0, "case {case}: sifting made layer {li} worse");
                }
            }
        }
    }
}

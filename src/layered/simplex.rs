//! A network simplex over integer positions, and the auxiliary graph that turns cross-axis
//! placement into one. The program is from Gansner et al., "A Technique for Drawing Directed
//! Graphs" (1993), sections 2.3 and 4.2:
//!
//! ```text
//! minimise   sum over constraints  weight * (x[head] - x[tail])
//! subject to x[head] - x[tail] >= min_len
//! ```
//!
//! [`solve`] is that program over integers and [`place_cross_simplex`] the section 4.2 reduction.
//! Cut values live in the tree as differences of postorder prefix sums, so a pivot only renumbers
//! the subtree it reshaped. Dot's LR balance step is left out, since centring free nodes between
//! their walls turns a chain's single jog into two.

use super::coord::legalize;
use super::state::LayeredState;
use crate::error::LayoutError;
use crate::options::LayoutOptions;
use crate::sort::SortDyn;

/// Weight per unit of cross-axis offset between a fork/join block's fork and its join.
const BLOCK_PULL: f64 = 0.5;

/// Weight multiplier for a hop of the main path.
const SPINE_FACTOR: f64 = 8.0;

/// One constraint: `x[head] - x[tail] >= min_len`, costing `weight` per unit of
/// `x[head] - x[tail]`.
#[derive(Clone, Copy, Debug)]
pub struct Constraint {
    pub tail: usize,
    pub head: usize,
    pub min_len: i64,
    pub weight: f64,
}

/// How much work one [`solve`] may spend. Positions stay feasible at every pivot, so hitting
/// `max_pivots` returns a less optimal answer. Running out of `max_tree_work` returns `None`.
#[derive(Clone, Copy, Debug)]
pub struct Budget {
    /// Leave/enter exchanges allowed.
    pub max_pivots: usize,
    /// Edge visits the initial feasible-tree construction may spend.
    pub max_tree_work: usize,
}

/// Float-noise tolerance for a negative cut value.
const CUT_EPS: f64 = 1e-9;

/// Not a tree edge, or no parent.
const NONE: u32 = u32::MAX;

/// How many negative cut values [`Tree::leave_edge`] compares before it pivots on the most
/// negative, as dot's `search_size`. The steepest of them needs fewer pivots than the first.
const LEAVE_SEARCH: usize = 30;

/// Minimise `sum weight * (x[head] - x[tail])` subject to every `x[head] - x[tail] >= min_len`.
///
/// Returns `None` if the constraint graph has a cycle or the feasible tree runs past its budget.
/// Each disconnected component is solved on its own and starts at 0.
pub fn solve(n: usize, edges: &[Constraint], budget: Budget) -> Option<Vec<i64>> {
    solve_from(n, edges, budget, None)
}

/// [`solve`] from a start: `start[v]` is where variable `v` begins, `None` for no opinion. A start
/// near the optimum, e.g., the previous solution of a slightly changed problem, needs few pivots.
pub fn solve_from(
    n: usize,
    edges: &[Constraint],
    budget: Budget,
    start: Option<&[Option<i64>]>,
) -> Option<Vec<i64>> {
    if n == 0 {
        return Some(vec![]);
    }
    let mut tree = Tree::new(n, edges, start)?;
    tree.build_feasible_tree(budget.max_tree_work)?;
    tree.number();
    let mut pivots = 0usize;
    while let Some(leave) = tree.leave_edge() {
        let Some(enter) = tree.enter_edge(leave) else {
            // No entering edge contradicts the negative cut value. Stop instead of spinning, since
            // `x` is still feasible.
            break;
        };
        tree.exchange(leave, enter);
        pivots += 1;
        if pivots >= budget.max_pivots {
            break;
        }
    }
    tree.normalise_components();
    Some(tree.x)
}

/// The spanning tree the simplex pivots on, plus everything derived from it.
struct Tree<'a> {
    n: usize,
    edges: &'a [Constraint],
    /// Incident edges per node as CSR: `inc[inc_at[v]..inc_at[v + 1]]`.
    inc: Vec<u32>,
    inc_at: Vec<u32>,
    /// Positions, feasible from the first assignment on.
    x: Vec<i64>,
    /// `out-weight(v) - in-weight(v)`, summed over subtrees for cut values.
    d: Vec<f64>,
    in_tree: Vec<bool>,
    /// Incident tree edges per node, packed into each node's slice of `inc_at` (a node has no
    /// more tree edges than edges): `tree_adj[inc_at[v]..][..tree_deg[v]]`.
    tree_adj: Vec<u32>,
    tree_deg: Vec<u32>,
    /// The walk stack of [`Tree::dfs_range`], kept so a pivot does not allocate.
    walk: Vec<(u32, u32, u32)>,
    /// Edge to the parent, `NONE` at a component root.
    par: Vec<u32>,
    /// Postorder numbering: `v`'s subtree is exactly `low[v]..=lim[v]`, and `at_lim` inverts it.
    low: Vec<u32>,
    lim: Vec<u32>,
    at_lim: Vec<u32>,
    /// `prefix[i]` is `d` summed over the first `i` nodes in `lim` order.
    prefix: Vec<f64>,
    /// Cut value per tree edge. Depends on the tree's shape only, not on `x`.
    cut: Vec<f64>,
    /// Tree edges for the cyclic search, and each one's slot in that list.
    tree_list: Vec<u32>,
    slot: Vec<u32>,
    /// Where the last [`Tree::leave_edge`] stopped.
    cursor: usize,
    /// One root per component, in discovery order.
    roots: Vec<u32>,
    comp: Vec<u32>,
}

impl<'a> Tree<'a> {
    /// Incidence lists, `d`, and a feasible starting position. `None` on a cycle or bad index.
    fn new(n: usize, edges: &'a [Constraint], start: Option<&[Option<i64>]>) -> Option<Self> {
        let m = edges.len();
        let mut inc_at = vec![0u32; n + 1];
        for c in edges {
            if c.tail >= n || c.head >= n || c.tail == c.head {
                return None;
            }
            inc_at[c.tail] += 1;
            inc_at[c.head] += 1;
        }
        let mut acc = 0u32;
        for slot in inc_at.iter_mut() {
            let deg = *slot;
            *slot = acc;
            acc = acc.saturating_add(deg);
        }
        let mut fill = inc_at.clone();
        let mut inc = vec![0u32; 2 * m];
        let mut d = vec![0.0f64; n];
        for (i, c) in edges.iter().enumerate() {
            inc[fill[c.tail] as usize] = i as u32;
            fill[c.tail] += 1;
            inc[fill[c.head] as usize] = i as u32;
            fill[c.head] += 1;
            d[c.tail] += c.weight;
            d[c.head] -= c.weight;
        }
        let x = feasible_start(n, edges, start)?;
        Some(Tree {
            n,
            edges,
            inc,
            inc_at,
            x,
            d,
            in_tree: vec![false; m],
            tree_adj: vec![NONE; 2 * m],
            tree_deg: vec![0; n],
            walk: vec![],
            par: vec![NONE; n],
            low: vec![0; n],
            lim: vec![0; n],
            at_lim: vec![0; n],
            prefix: vec![0.0; n + 1],
            cut: vec![0.0; m],
            tree_list: Vec::with_capacity(n),
            slot: vec![NONE; m],
            cursor: 0,
            roots: vec![],
            comp: vec![0; n],
        })
    }

    fn slack(&self, e: u32) -> i64 {
        let c = self.edges[e as usize];
        self.x[c.head] - self.x[c.tail] - c.min_len
    }

    fn incident(&self, v: usize) -> &[u32] {
        &self.inc[self.inc_at[v] as usize..self.inc_at[v + 1] as usize]
    }

    fn push_tree_incident(&mut self, v: usize, e: u32) {
        let at = self.inc_at[v] as usize + self.tree_deg[v] as usize;
        self.tree_adj[at] = e;
        self.tree_deg[v] += 1;
    }

    /// Drop `e` from `v`'s tree edges by moving the last one into its place, the order a
    /// renumbering walk sees afterwards.
    fn remove_tree_incident(&mut self, v: usize, e: u32) {
        let at = self.inc_at[v] as usize;
        let deg = self.tree_deg[v] as usize;
        if let Some(i) = self.tree_adj[at..at + deg].iter().position(|&f| f == e) {
            self.tree_adj[at + i] = self.tree_adj[at + deg - 1];
            self.tree_deg[v] -= 1;
        }
    }

    fn other(&self, e: u32, v: usize) -> usize {
        let c = self.edges[e as usize];
        if c.tail == v {
            c.head
        } else {
            c.tail
        }
    }

    /// Grows a tight spanning tree per component over slack-0 edges. When stuck, shifts the tree by
    /// the least slack of a crossing edge, which keeps every other crossing constraint feasible.
    fn build_feasible_tree(&mut self, max_work: usize) -> Option<()> {
        let mut in_set = vec![false; self.n];
        let mut work = 0usize;
        for root in 0..self.n {
            if in_set[root] {
                continue;
            }
            let comp = self.roots.len() as u32;
            self.roots.push(root as u32);
            in_set[root] = true;
            self.comp[root] = comp;
            let mut members: Vec<u32> = vec![root as u32];
            let mut pending: Vec<u32> = vec![root as u32];
            let mut leaving: Vec<u32> = vec![];
            loop {
                while let Some(u) = pending.pop() {
                    let (from, to) = (
                        self.inc_at[u as usize] as usize,
                        self.inc_at[u as usize + 1] as usize,
                    );
                    for i in from..to {
                        work += 1;
                        if work > max_work {
                            return None;
                        }
                        let e = self.inc[i];
                        let w = self.other(e, u as usize);
                        if in_set[w] {
                            continue;
                        }
                        if self.slack(e) == 0 {
                            in_set[w] = true;
                            self.comp[w] = comp;
                            members.push(w as u32);
                            self.add_tree_edge(e);
                            pending.push(w as u32);
                        } else {
                            leaving.push(e);
                        }
                    }
                }
                // Stuck. Either the component is complete, or one shift lets it grow again.
                let mut best: Option<(i64, u32)> = None;
                let (edges, in_tree, x) = (self.edges, &self.in_tree, &self.x);
                leaving.retain(|&e| {
                    let c = edges[e as usize];
                    let crosses = in_set[c.tail] != in_set[c.head] && !in_tree[e as usize];
                    if crosses {
                        let s = x[c.head] - x[c.tail] - c.min_len;
                        if best.is_none_or(|(bs, be)| s < bs || (s == bs && e < be)) {
                            best = Some((s, e));
                        }
                    }
                    crosses
                });
                work = work.saturating_add(leaving.len());
                if work > max_work {
                    return None;
                }
                let Some((s, e)) = best else { break };
                let c = self.edges[e as usize];
                let delta = if in_set[c.head] { -s } else { s };
                if delta != 0 {
                    for &v in &members {
                        self.x[v as usize] += delta;
                    }
                }
                let w = if in_set[c.tail] { c.head } else { c.tail };
                in_set[w] = true;
                self.comp[w] = comp;
                members.push(w as u32);
                self.add_tree_edge(e);
                pending.push(w as u32);
            }
        }
        Some(())
    }

    fn add_tree_edge(&mut self, e: u32) {
        let c = self.edges[e as usize];
        self.in_tree[e as usize] = true;
        self.push_tree_incident(c.tail, e);
        self.push_tree_incident(c.head, e);
        self.slot[e as usize] = self.tree_list.len() as u32;
        self.tree_list.push(e);
    }

    fn number(&mut self) {
        let mut next = 0u32;
        for i in 0..self.roots.len() {
            let root = self.roots[i] as usize;
            next = self.dfs_range(root, NONE, next);
        }
        self.refresh(0, self.n.saturating_sub(1) as u32);
    }

    /// Postorder-number the subtree of `root` from index `first`, returning the next free index.
    /// Iterative because a 3000-deep walk would overflow a small wasm or worker stack.
    fn dfs_range(&mut self, root: usize, par_edge: u32, first: u32) -> u32 {
        let mut next = first;
        self.par[root] = par_edge;
        self.low[root] = next;
        // (node, parent edge, tree edges walked so far)
        let mut stack = std::mem::take(&mut self.walk);
        stack.clear();
        stack.push((root as u32, par_edge, 0));
        while let Some(top) = stack.last_mut() {
            let (v, pe, seen) = (top.0 as usize, top.1, top.2);
            if seen < self.tree_deg[v] {
                top.2 += 1;
                let e = self.tree_adj[self.inc_at[v] as usize + seen as usize];
                if e == pe {
                    continue;
                }
                let w = self.other(e, v);
                self.par[w] = e;
                self.low[w] = next;
                stack.push((w as u32, e, 0));
            } else {
                self.lim[v] = next;
                self.at_lim[next as usize] = v as u32;
                next += 1;
                stack.pop();
            }
        }
        self.walk = stack;
        next
    }

    /// Recompute prefix sums and cut values over the renumbered range `lo..=hi`.
    fn refresh(&mut self, lo: u32, hi: u32) {
        if self.n == 0 {
            return;
        }
        for i in lo as usize..=hi as usize {
            self.prefix[i + 1] = self.prefix[i] + self.d[self.at_lim[i] as usize];
        }
        // The subtree root at `hi` still splits the same node sets, so its cut value is unchanged.
        for i in lo as usize..hi as usize {
            let v = self.at_lim[i] as usize;
            let e = self.par[v];
            if e == NONE {
                continue;
            }
            self.cut[e as usize] = self.cut_value(e, v);
        }
    }

    /// Cut value of tree edge `e`: `d` summed over `child`'s subtree, negated when the child is
    /// the head.
    fn cut_value(&self, e: u32, child: usize) -> f64 {
        let sum = self.prefix[self.lim[child] as usize + 1] - self.prefix[self.low[child] as usize];
        if self.edges[e as usize].tail == child {
            sum
        } else {
            -sum
        }
    }

    /// The most negative cut value among the next [`LEAVE_SEARCH`] negative tree edges. Searches
    /// round-robin from the last stop, as dot does, because a pivot only changes one subtree's
    /// cut values.
    fn leave_edge(&mut self) -> Option<u32> {
        let len = self.tree_list.len();
        if len == 0 {
            return None;
        }
        let mut best: Option<(f64, usize)> = None;
        let mut found = 0usize;
        for k in 0..len {
            let i = (self.cursor + k) % len;
            let e = self.tree_list[i];
            let c = self.cut[e as usize];
            if c < -CUT_EPS {
                if best.is_none_or(|(bc, _)| c < bc) {
                    best = Some((c, i));
                }
                found += 1;
                if found >= LEAVE_SEARCH {
                    break;
                }
            }
        }
        let (_, i) = best?;
        self.cursor = i;
        Some(self.tree_list[i])
    }

    /// The least-slack non-tree edge from `e`'s head component back into its tail component,
    /// scanning the child side `low[child]..=lim[child]`.
    fn enter_edge(&self, e: u32) -> Option<u32> {
        let c = self.edges[e as usize];
        // The child of a tree edge is its endpoint with the lower index.
        let (child, want_head_inside) = if self.lim[c.tail] < self.lim[c.head] {
            (c.tail, true)
        } else {
            (c.head, false)
        };
        let (lo, hi) = (self.low[child], self.lim[child]);
        let inside = |v: usize| -> bool { self.lim[v] >= lo && self.lim[v] <= hi };
        let mut best: Option<(i64, u32)> = None;
        for i in lo as usize..=hi as usize {
            let v = self.at_lim[i] as usize;
            for &g in self.incident(v) {
                if self.in_tree[g as usize] {
                    continue;
                }
                let cg = self.edges[g as usize];
                let candidate = if want_head_inside {
                    cg.head == v && !inside(cg.tail)
                } else {
                    cg.tail == v && !inside(cg.head)
                };
                if !candidate {
                    continue;
                }
                let s = self.slack(g);
                if best.is_none_or(|(bs, bg)| s < bs || (s == bs && g < bg)) {
                    best = Some((s, g));
                }
            }
        }
        best.map(|(_, g)| g)
    }

    /// Swap tree edge `leave` for `enter`: shift until `enter` is tight, then renumber.
    fn exchange(&mut self, leave: u32, enter: u32) {
        let cl = self.edges[leave as usize];
        let ce = self.edges[enter as usize];
        let delta = self.slack(enter);
        if delta != 0 {
            // `enter` had the least slack of the crossing edges, so none goes negative.
            if self.lim[cl.tail] < self.lim[cl.head] {
                self.shift(cl.tail, -delta);
            } else {
                self.shift(cl.head, delta);
            }
        }
        // The tree path between `enter`'s ends runs through `leave`, so the LCA bounds the change.
        let lca = self.lca(ce.tail, ce.head);
        let (lo, hi, par_lca) = (self.low[lca], self.lim[lca], self.par[lca]);
        self.remove_tree_edge(leave);
        self.insert_tree_edge(leave, enter);
        self.dfs_range(lca, par_lca, lo);
        self.refresh(lo, hi);
    }

    fn shift(&mut self, v: usize, delta: i64) {
        for i in self.low[v] as usize..=self.lim[v] as usize {
            let u = self.at_lim[i] as usize;
            self.x[u] += delta;
        }
    }

    fn lca(&self, a: usize, b: usize) -> usize {
        let mut v = a;
        let mut guard = 0usize;
        while !(self.lim[b] >= self.low[v] && self.lim[b] <= self.lim[v]) {
            let e = self.par[v];
            if e == NONE || guard > self.n {
                return v;
            }
            v = self.other(e, v);
            guard += 1;
        }
        v
    }

    fn remove_tree_edge(&mut self, e: u32) {
        let c = self.edges[e as usize];
        self.in_tree[e as usize] = false;
        self.remove_tree_incident(c.tail, e);
        self.remove_tree_incident(c.head, e);
    }

    /// Put `enter` in the search-list slot `leave` held.
    fn insert_tree_edge(&mut self, leave: u32, enter: u32) {
        let c = self.edges[enter as usize];
        self.in_tree[enter as usize] = true;
        self.push_tree_incident(c.tail, enter);
        self.push_tree_incident(c.head, enter);
        let slot = self.slot[leave as usize];
        self.slot[leave as usize] = NONE;
        self.slot[enter as usize] = slot;
        if let Some(entry) = self.tree_list.get_mut(slot as usize) {
            *entry = enter;
        }
    }

    /// Start every component at 0. Their relative offsets are arbitrary, and a common anchor keeps
    /// the drawing together.
    fn normalise_components(&mut self) {
        let mut min = vec![i64::MAX; self.roots.len().max(1)];
        for v in 0..self.n {
            let c = self.comp[v] as usize;
            min[c] = min[c].min(self.x[v]);
        }
        for v in 0..self.n {
            let c = self.comp[v] as usize;
            if min[c] != i64::MAX {
                self.x[v] -= min[c];
            }
        }
    }
}

/// The Gansner et al. section 4.2 weight multiplier, with the paper's values: 1 real-real,
/// 2 real-dummy, 8 dummy-dummy. The ratio keeps long edges straight.
pub(super) fn omega(a_dummy: bool, b_dummy: bool) -> f64 {
    match (a_dummy, b_dummy) {
        (false, false) => 1.0,
        (true, true) => 8.0,
        _ => 2.0,
    }
}

/// Places every node on the order axis by solving the Gansner et al. section 4.2 program exactly.
/// Returns `Ok(false)` when the problem exceeds [`CoordTuning::simplex_max_nodes`] or no feasible
/// tree fits the work budget.
///
/// [`CoordTuning::simplex_max_nodes`]: crate::options::CoordTuning::simplex_max_nodes
pub fn place_cross_simplex(st: &mut LayeredState, o: &LayoutOptions) -> Result<bool, LayoutError> {
    let n = st.len();
    if n == 0 {
        return Ok(true);
    }
    // Weighting the main path here packs everything else around it. Straightening it in a later
    // pass would leave a gap on the side it moved away from.
    let spine_hops = super::mainpath::spine_hops(st, o);
    // An end hinted to attach off its border's middle pulls its neighbour level with that point,
    // not with the node's centre.
    let hints = super::route::hints::Hints::of(st, &st.boxes());
    // (from, to, weight, offset): minimise `weight * |cross[from] - (cross[to] + offset)|`.
    let arcs: Vec<(usize, usize, f64, i64)> = st
        .arcs
        .iter()
        .filter(|a| a.from != a.to)
        .map(|a| {
            let w = st.weight.get(a.edge).copied().unwrap_or(1.0);
            let omega = omega(st.nodes[a.from].is_dummy(), st.nodes[a.to].is_dummy());
            let spine =
                if spine_hops.contains(&(a.from, a.to)) || spine_hops.contains(&(a.to, a.from)) {
                    SPINE_FACTOR
                } else {
                    1.0
                };
            let offset = hints.order_offset(a.edge, a.to) - hints.order_offset(a.edge, a.from);
            (a.from, a.to, w * omega * spine, offset.round() as i64)
        })
        .collect();
    // A block's join costs the same anywhere between its branches. A light pull to its fork makes
    // the level position cheapest.
    let mut arcs = arcs;
    if super::balance::frames_the_drawing(st, o) {
        for (fork, join) in super::splits::block_pairs(st) {
            arcs.push((fork, join, BLOCK_PULL, 0));
        }
    }
    let n_aux = n + arcs.len();
    if n_aux > o.tuning.coords.simplex_max_nodes {
        return Ok(false);
    }
    let mut cons: Vec<Constraint> = Vec::with_capacity(2 * arcs.len() + n);
    for (i, &(from, to, weight, offset)) in arcs.iter().enumerate() {
        // With `m <= from - offset` and `m <= to`, the cost `weight * (from - m + to - m)` is
        // `weight * |from - (to + offset)|` plus a constant. A negative offset moves to the other
        // constraint so both lengths stay non-negative.
        let m = n + i;
        cons.push(Constraint {
            tail: m,
            head: from,
            min_len: offset.max(0),
            weight,
        });
        cons.push(Constraint {
            tail: m,
            head: to,
            min_len: (-offset).max(0),
            weight,
        });
    }
    for layer in &st.layers {
        for pair in layer.windows(2) {
            cons.push(Constraint {
                tail: pair[0],
                head: pair[1],
                // `as i64` saturates, and widths are validated finite.
                min_len: st.sep(pair[0], pair[1], o).ceil() as i64,
                weight: 0.0,
            });
        }
    }
    // One weightless boundary variable per pair of branch bands, with each node at least half its
    // extent plus half a gap on its own side.
    let bands = super::balance::branch_bands(st, o);
    let half = |v: usize| ((st.nodes[v].w_cross + st.gap(v, v, o)) / 2.0).ceil() as i64;
    for (k, (before, after)) in bands.iter().enumerate() {
        let z = n_aux + k;
        for &v in before {
            cons.push(Constraint {
                tail: v,
                head: z,
                min_len: half(v),
                weight: 0.0,
            });
        }
        for &v in after {
            cons.push(Constraint {
                tail: z,
                head: v,
                min_len: half(v),
                weight: 0.0,
            });
        }
    }
    let n_vars = n_aux + bands.len();
    let budget = Budget {
        max_pivots: o.tuning.coords.simplex_max_pivots,
        // A safety net. Tree construction is linear per node in practice.
        max_tree_work: 64usize.saturating_mul(cons.len().saturating_add(n_vars)),
    };
    // Repeated solves differ by a few moved waypoints, so the last solution is a close start.
    let start: Option<Vec<Option<i64>>> = st
        .warm_cross
        .as_ref()
        .filter(|w| w.len() == n)
        .map(|w| (0..n_vars).map(|v| w.get(v).copied()).collect());
    let Some(x) = solve_from(n_vars, &cons, budget, start.as_deref()) else {
        return Ok(false);
    };
    st.warm_cross = Some(x[..n].to_vec());
    // The solver stops at one vertex of an optimal face. Walk the face (see `super::face`) for
    // the point whose long edges change level least often.
    for (node, &v) in st.nodes.iter_mut().zip(x.iter()) {
        node.cross = v as f64;
    }
    super::face::settle_chains(st, o);
    let mut cross: Vec<f64> = st.nodes.iter().map(|nd| nd.cross).collect();
    // Centre on the median, as Brandes-Kopf does, so both placements look alike mid-pipeline.
    let mut sorted = cross.clone();
    sorted.sort_unstable_by_dyn(f64::total_cmp);
    let median = sorted.get(n / 2).copied().unwrap_or(0.0);
    for c in cross.iter_mut() {
        *c -= median;
    }
    // Only a guard, since the integer program already honours every separation.
    legalize(st, o, &mut cross);
    for (node, &c) in st.nodes.iter_mut().zip(cross.iter()) {
        node.cross = c;
    }
    Ok(true)
}

/// A feasible start, `None` on a cycle. Without `start` every variable sits as low as its
/// constraints allow (longest path). With one, each begins at its start and moves up only as far
/// as its constraints demand.
fn feasible_start(
    n: usize,
    edges: &[Constraint],
    start: Option<&[Option<i64>]>,
) -> Option<Vec<i64>> {
    // Far below any position, yet far from overflowing when a length is added.
    const UNSET: i64 = i64::MIN / 4;
    let given = |v: usize| start.and_then(|s| s.get(v).copied().flatten());
    let mut in_deg = vec![0u32; n];
    let mut out_at = vec![0u32; n + 1];
    for c in edges {
        in_deg[c.head] += 1;
        out_at[c.tail] += 1;
    }
    let mut acc = 0u32;
    for slot in out_at.iter_mut() {
        let deg = *slot;
        *slot = acc;
        acc = acc.saturating_add(deg);
    }
    let mut fill = out_at.clone();
    let mut out = vec![0u32; edges.len()];
    for (i, c) in edges.iter().enumerate() {
        out[fill[c.tail] as usize] = i as u32;
        fill[c.tail] += 1;
    }
    let mut x: Vec<i64> = (0..n)
        .map(|v| match start {
            Some(_) => given(v).unwrap_or(UNSET),
            None => 0,
        })
        .collect();
    let mut remaining = in_deg.clone();
    let mut queue: Vec<u32> = (0..n as u32).filter(|&v| in_deg[v as usize] == 0).collect();
    let mut head = 0usize;
    let mut seen = 0usize;
    while head < queue.len() {
        let v = queue[head] as usize;
        head += 1;
        seen += 1;
        for &e in &out[out_at[v] as usize..out_at[v + 1] as usize] {
            let c = edges[e as usize];
            x[c.head] = x[c.head].max(x[v] + c.min_len);
            remaining[c.head] -= 1;
            if remaining[c.head] == 0 {
                queue.push(c.head as u32);
            }
        }
    }
    if seen != n {
        return None;
    }
    for v in 0..n {
        if in_deg[v] != 0 || given(v).is_some() {
            continue;
        }
        let mut room: Option<i64> = None;
        for &e in &out[out_at[v] as usize..out_at[v + 1] as usize] {
            let c = edges[e as usize];
            let bound = x[c.head] - c.min_len;
            room = Some(room.map_or(bound, |b: i64| b.min(bound)));
        }
        if let Some(bound) = room {
            x[v] = bound;
        }
    }
    Some(x)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::geometry::shape::NodeShape;
    use crate::layered::dummy::{build_edge_chains, insert_dummies};
    use crate::layered::prepare::{LayeredInput, LayeredInputNode};
    use crate::options::{Direction, LayoutOptions};

    const BUDGET: Budget = Budget {
        max_pivots: 10_000,
        max_tree_work: 1_000_000,
    };

    /// The objective, for tests that assert an optimum instead of a position.
    fn cost(edges: &[Constraint], x: &[i64]) -> f64 {
        edges
            .iter()
            .map(|c| c.weight * (x[c.head] - x[c.tail]) as f64)
            .sum()
    }

    fn feasible(edges: &[Constraint], x: &[i64]) -> bool {
        edges.iter().all(|c| x[c.head] - x[c.tail] >= c.min_len)
    }

    #[test]
    fn a_single_constraint_comes_out_tight() {
        let edges = [Constraint {
            tail: 0,
            head: 1,
            min_len: 5,
            weight: 1.0,
        }];
        assert_eq!(solve(2, &edges, BUDGET).unwrap(), vec![0, 5]);
    }

    /// The section 4.2 reduction in miniature. The auxiliary node settles on the lower node.
    #[test]
    fn an_auxiliary_node_lands_on_the_nearer_endpoint() {
        let edges = [
            Constraint {
                tail: 2,
                head: 0,
                min_len: 0,
                weight: 1.0,
            },
            Constraint {
                tail: 2,
                head: 1,
                min_len: 0,
                weight: 1.0,
            },
            Constraint {
                tail: 0,
                head: 1,
                min_len: 10,
                weight: 0.0,
            },
        ];
        let x = solve(3, &edges, BUDGET).unwrap();
        assert!(feasible(&edges, &x));
        assert_eq!(x[2], x[0], "the auxiliary node sits on the lower node");
        assert_eq!(x[1] - x[0], 10);
        assert_eq!(cost(&edges, &x), 10.0);
    }

    /// Node 3 is priced against nodes 0 and 2, five times as heavily against 0, and aligns with it.
    #[test]
    fn a_node_aligns_with_its_heaviest_neighbour() {
        let sep = |tail: usize, head: usize| Constraint {
            tail,
            head,
            min_len: 10,
            weight: 0.0,
        };
        let edges = [
            sep(0, 1),
            sep(1, 2),
            // aux node 4 prices the (3, 0) edge at 5, aux node 5 prices the (3, 2) edge at 1.
            Constraint {
                tail: 4,
                head: 3,
                min_len: 0,
                weight: 5.0,
            },
            Constraint {
                tail: 4,
                head: 0,
                min_len: 0,
                weight: 5.0,
            },
            Constraint {
                tail: 5,
                head: 3,
                min_len: 0,
                weight: 1.0,
            },
            Constraint {
                tail: 5,
                head: 2,
                min_len: 0,
                weight: 1.0,
            },
        ];
        let x = solve(6, &edges, BUDGET).unwrap();
        assert!(feasible(&edges, &x));
        assert_eq!(x[3], x[0], "5 * |x3 - x0| dominates 1 * |x3 - x2|");
        assert_eq!(cost(&edges, &x), 20.0);
    }

    #[test]
    fn disconnected_components_are_each_anchored() {
        let edges = [
            Constraint {
                tail: 0,
                head: 1,
                min_len: 4,
                weight: 1.0,
            },
            Constraint {
                tail: 2,
                head: 3,
                min_len: 7,
                weight: 1.0,
            },
        ];
        assert_eq!(solve(4, &edges, BUDGET).unwrap(), vec![0, 4, 0, 7]);
    }

    #[test]
    fn a_cyclic_program_is_declined() {
        let edges = [
            Constraint {
                tail: 0,
                head: 1,
                min_len: 1,
                weight: 1.0,
            },
            Constraint {
                tail: 1,
                head: 0,
                min_len: 1,
                weight: 1.0,
            },
        ];
        assert!(solve(2, &edges, BUDGET).is_none());
    }

    #[test]
    fn a_capped_solve_is_still_feasible() {
        let (st, o) = long_edge_state();
        let mut st2 = st;
        let capped = LayoutOptions {
            tuning: crate::options::Tuning {
                coords: crate::options::CoordTuning {
                    simplex_max_pivots: 1,
                    ..o.tuning.coords
                },
                ..o.tuning
            },
            ..o
        };
        assert!(place_cross_simplex(&mut st2, &capped).unwrap());
        for layer in &st2.layers {
            for pair in layer.windows(2) {
                let need = st2.sep(pair[0], pair[1], &capped);
                let got = st2.nodes[pair[1]].cross - st2.nodes[pair[0]].cross;
                assert!(got >= need - 1e-6, "{pair:?}: {got} < {need}");
            }
        }
    }

    #[test]
    fn any_start_reaches_the_cold_optimum() {
        // Edges only run from lower to higher index, so the program is acyclic.
        let mut rng = 7u64;
        let mut next = |bound: u64| {
            rng = rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (rng >> 33) % bound
        };
        let n = 40;
        let edges: Vec<Constraint> = (0..120)
            .filter_map(|_| {
                let a = next(n as u64) as usize;
                let b = next(n as u64) as usize;
                (a != b).then(|| Constraint {
                    tail: a.min(b),
                    head: a.max(b),
                    min_len: next(6) as i64,
                    weight: next(4) as f64,
                })
            })
            .collect();
        let cold = solve(n, &edges, BUDGET).unwrap();
        let best = cost(&edges, &cold);
        // The optimum itself, the optimum mirrored (infeasible almost everywhere), and only every
        // third variable placed.
        let starts: [Vec<Option<i64>>; 3] = [
            cold.iter().map(|&x| Some(x)).collect(),
            cold.iter().map(|&x| Some(-x)).collect(),
            (0..n).map(|v| (v % 3 == 0).then_some(cold[v])).collect(),
        ];
        for start in &starts {
            let warm = solve_from(n, &edges, BUDGET, Some(start)).unwrap();
            assert!(feasible(&edges, &warm));
            assert!((cost(&edges, &warm) - best).abs() < 1e-9);
        }
    }

    fn node(width: f64, height: f64) -> LayeredInputNode {
        LayeredInputNode {
            width,
            height,
            shape: NodeShape::Box,
            constraint: None,
            category: None,
            clear_after: 0.0,
            clear_before: 0.0,
            port_sides: None,
            lane_key: None,
            along_before: 0.0,
            along_after: 0.0,
        }
    }

    /// Layers `[0, 1]`, `[4, 2]`, `[3]`, where 4 is the waypoint of long edge `0 -> 3`. Either the
    /// long edge or the path through node 2 can run level, not both.
    fn long_edge_state() -> (LayeredState, LayoutOptions) {
        let input = LayeredInput {
            nodes: vec![node(120.0, 52.0); 4],
            edges: vec![(0, 3), (0, 2), (2, 3)],
            weights: vec![],
            thickness: vec![],
            edge_label_sizes: vec![],
            seed: vec![],
            pinned: vec![],
            self_loops: vec![],
            ranked_backwards: Vec::new(),
            options: {
                // Otherwise 0 -> 2 -> 3 would be weighted up as the main path.
                let mut o = LayoutOptions {
                    direction: Direction::LeftRight,
                    ..LayoutOptions::layered_default()
                };
                o.tuning.coords.main_path_room = 0;
                o.coords.branch_bands = false;
                o
            },
        };
        let o = input.options;
        let mut st = LayeredState::new(&input);
        for (i, r) in [0i32, 0, 1, 2].iter().enumerate() {
            st.nodes[i].rank = *r;
        }
        // Node 1 has no edges.
        st.nodes[1].rank = 0;
        insert_dummies(&mut st).unwrap();
        st.rebuild_layers();
        build_edge_chains(&mut st).unwrap();
        st.layers = vec![vec![0, 1], vec![4, 2], vec![3]];
        st.reindex();
        (st, o)
    }

    /// An inner chain segment outweighs the two short edges, so the long edge stays straight.
    #[test]
    fn a_long_edge_is_drawn_straight() {
        let (mut st, o) = long_edge_state();
        assert!(place_cross_simplex(&mut st, &o).unwrap());
        let (source, waypoint, target) = (st.nodes[0].cross, st.nodes[4].cross, st.nodes[3].cross);
        assert!(
            (waypoint - source).abs() < 1.0 && (waypoint - target).abs() < 1.0,
            "the chain is not level: {source} -> {waypoint} -> {target}"
        );
        assert!(
            (st.nodes[2].cross - waypoint).abs() >= st.sep(4, 2, &o) - 1e-6,
            "the node beside the chain has to stand clear of it"
        );
    }

    #[test]
    fn every_layer_keeps_its_order_and_separation() {
        let (mut st, o) = long_edge_state();
        assert!(place_cross_simplex(&mut st, &o).unwrap());
        for layer in &st.layers {
            for pair in layer.windows(2) {
                let need = st.sep(pair[0], pair[1], &o);
                let got = st.nodes[pair[1]].cross - st.nodes[pair[0]].cross;
                assert!(got >= need - 1e-6, "{pair:?}: {got} < {need}");
            }
        }
    }

    #[test]
    fn the_placement_is_deterministic() {
        let (mut a, o) = long_edge_state();
        let (mut b, _) = long_edge_state();
        assert!(place_cross_simplex(&mut a, &o).unwrap());
        assert!(place_cross_simplex(&mut b, &o).unwrap());
        let of = |st: &LayeredState| {
            st.nodes
                .iter()
                .map(|nd| nd.cross.to_bits())
                .collect::<Vec<_>>()
        };
        assert_eq!(of(&a), of(&b));
    }

    #[test]
    fn an_oversized_problem_is_declined() {
        let (mut st, o) = long_edge_state();
        let tiny = LayoutOptions {
            tuning: crate::options::Tuning {
                coords: crate::options::CoordTuning {
                    simplex_max_nodes: 3,
                    ..o.tuning.coords
                },
                ..o.tuning
            },
            ..o
        };
        assert!(!place_cross_simplex(&mut st, &tiny).unwrap());
    }
}

//! Searches the optimal face of the placement program for the point whose long edges change level
//! least often.
//!
//! The L1 optimum of [`super::simplex::place_cross_simplex`] is usually a whole face, and the
//! vertex the simplex stops at is often a staircase. This pass only moves along the face, so the
//! objective never rises. [`walk_chain`] re-lanes one chain by shortest path. [`Walk::try_slide`]
//! slides whole straight components ([`Classes`]), since moving one end of a level arc is never
//! free.
//!
//! Positions stay integers ([`super::simplex::solve`] rounds separations up, slot bounds round
//! inward), so placements that should tie do tie up to [`COST_EPS`].

use std::collections::{BTreeMap, BTreeSet};

use super::coord::{admits, best_lane, chain_lane_target, dummy_slot, lane_levels};
use super::simplex::omega;
use super::state::LayeredState;
use crate::geometry::SLACK;
use crate::options::LayoutOptions;
use crate::sort::SortDyn;

/// Float-noise tolerance for chain costs, which are sums of `weight * omega * integer`.
const COST_EPS: f64 = 1e-9;

const NONE: u32 = u32::MAX;

/// Lane-steps the walk may spend per auxiliary node the solver was allowed
/// ([`crate::options::CoordTuning::simplex_max_nodes`]). A chain costs at most its length squared.
const WORK_PER_NODE: usize = 64;

/// Nodes one slide may move. A component can be most of the drawing, e.g., a tree with every edge
/// level, and sliding it just translates the picture.
const MAX_SET: usize = 4096;

/// Runs up to [`crate::options::CoordTuning::simplex_face_rounds`] rounds over the chains until a
/// round moves nothing. `LNode::cross` must hold the solver's integer positions. Heavy chains go
/// first, since their lane is the one worth keeping.
pub(super) fn settle_chains(st: &mut LayeredState, o: &LayoutOptions) {
    let rounds = o.tuning.coords.simplex_face_rounds;
    if rounds == 0 {
        return;
    }
    st.reindex();
    // Taken out so the walk can write `cross` while reading them.
    let layers = std::mem::take(&mut st.layers);
    let chains = std::mem::take(&mut st.chains);
    let mut order: Vec<(usize, f64)> = chains
        .iter()
        .filter(|(_, c)| c.len() >= 3 && c[1..c.len() - 1].iter().any(|&n| st.nodes[n].is_dummy()))
        .map(|(&e, _)| (e, st.weight.get(e).copied().unwrap_or(1.0)))
        .collect();
    order.sort_unstable_by_dyn(super::coord::heaviest_first);
    let work: usize = order
        .iter()
        .map(|&(e, _)| {
            chains
                .get(&e)
                .map_or(0, |c| c.len().saturating_mul(c.len()))
        })
        .sum();
    if work <= WORK_PER_NODE.saturating_mul(o.tuning.coords.simplex_max_nodes) {
        let walk = Walk::new(st, o, &layers, &chains);
        for _ in 0..rounds {
            let mut moved = false;
            // A landed slide changes the components, so they are rebuilt per round and after each
            // chain that moved.
            let mut class = Classes::new(st);
            for &(edge, weight) in &order {
                let Some(chain) = chains.get(&edge) else {
                    continue;
                };
                let mut did = walk_chain(&walk, st, chain, weight);
                if did {
                    class = Classes::new(st);
                }
                if walk.straighten_arcs(st, &class, chain) {
                    did = true;
                    class = Classes::new(st);
                    walk_chain(&walk, st, chain, weight);
                }
                moved |= did;
            }
            if !moved {
                break;
            }
        }
    }
    st.layers = layers;
    st.chains = chains;
}

/// The tables taken out of the state plus two incidences `LayeredState` does not keep.
struct Walk<'a> {
    o: &'a LayoutOptions,
    layers: &'a [Vec<usize>],
    chains: &'a BTreeMap<usize, Vec<usize>>,
    /// All true, since every node already has its final position and no room has to be reserved.
    settled: Vec<bool>,
    /// Per node, `(other end, weight * omega)` for each arc.
    adj: Vec<Vec<(usize, f64)>>,
    /// Per node, the caller edges whose chain runs through it.
    incident: Vec<Vec<usize>>,
}

impl<'a> Walk<'a> {
    fn new(
        st: &LayeredState,
        o: &'a LayoutOptions,
        layers: &'a [Vec<usize>],
        chains: &'a BTreeMap<usize, Vec<usize>>,
    ) -> Walk<'a> {
        let n = st.len();
        let mut adj: Vec<Vec<(usize, f64)>> = vec![vec![]; n];
        for arc in &st.arcs {
            if arc.from == arc.to {
                continue;
            }
            let w = st.weight.get(arc.edge).copied().unwrap_or(1.0)
                * omega(st.nodes[arc.from].is_dummy(), st.nodes[arc.to].is_dummy());
            adj[arc.from].push((arc.to, w));
            adj[arc.to].push((arc.from, w));
        }
        let mut incident: Vec<Vec<usize>> = vec![vec![]; n];
        for (&edge, chain) in chains {
            for &v in chain {
                if incident[v].last() != Some(&edge) {
                    incident[v].push(edge);
                }
            }
        }
        Walk {
            o,
            layers,
            chains,
            settled: vec![true; n],
            adj,
            incident,
        }
    }

    /// Slots rounded inward to whole pixels. The solver rounded separations up, so the inward bound
    /// is the one the placement already satisfies.
    fn slots(&self, st: &LayeredState, dummies: &[usize]) -> Vec<(f64, f64)> {
        dummies
            .iter()
            .map(|&d| {
                let (lo, hi) = dummy_slot(st, self.layers, &self.settled, d, self.o);
                (round_in(lo, true), round_in(hi, false))
            })
            .collect()
    }

    /// `(level changes, same-direction pairs)`, compared lexicographically. Changes are bends in
    /// the route (`LayoutMetrics::bends`), pairs are `LayoutMetrics::staircase_steps`.
    fn stairs(&self, st: &LayeredState, edge: usize) -> (usize, usize) {
        let Some(chain) = self.chains.get(&edge) else {
            return (0, 0);
        };
        let (mut changes, mut pairs) = (0usize, 0usize);
        let mut last: Option<bool> = None;
        for w in chain.windows(2) {
            let d = st.nodes[w[1]].cross - st.nodes[w[0]].cross;
            if d.abs() <= 0.5 {
                continue;
            }
            changes += 1;
            if last == Some(d > 0.0) {
                pairs += 1;
            }
            last = Some(d > 0.0);
        }
        (changes, pairs)
    }

    fn stairs_of(&self, st: &LayeredState, edges: &[usize]) -> (usize, usize) {
        edges.iter().fold((0, 0), |acc, &e| {
            let (c, p) = self.stairs(st, e);
            (acc.0 + c, acc.1 + p)
        })
    }

    /// For each slanted arc of the chain, tries to slide one end's straight component onto the
    /// other end's level. Returns whether anything moved.
    fn straighten_arcs(&self, st: &mut LayeredState, class: &Classes, chain: &[usize]) -> bool {
        let mut moved = false;
        for w in chain.windows(2) {
            let (a, b) = (w[0], w[1]);
            let gap = st.nodes[b].cross - st.nodes[a].cross;
            if gap.abs() < 0.5 {
                continue;
            }
            // The waypoint side moves first, since a real node carries other edges.
            let mut ends = [(a, gap), (b, -gap)];
            if !st.nodes[a].is_dummy() && st.nodes[b].is_dummy() {
                ends.swap(0, 1);
            }
            for (seed, delta) in ends {
                if self.try_slide(st, class, seed, delta) {
                    moved = true;
                    break;
                }
            }
        }
        moved
    }

    /// Slides `seed`'s component and every component it pushes into by `delta`. Kept only if the
    /// boundary arcs cost no more and the touched chains change level less often. `class` may be
    /// one slide stale, which only loses opportunities.
    fn try_slide(&self, st: &mut LayeredState, class: &Classes, seed: usize, delta: f64) -> bool {
        if delta.abs() < 0.5 {
            return false;
        }
        // A component joins the set when its gap on the travel side is smaller than the slide.
        let mut set: BTreeSet<u32> = BTreeSet::new();
        let mut queue: Vec<u32> = vec![class.of[seed]];
        set.insert(class.of[seed]);
        let mut members = 0usize;
        while let Some(c) = queue.pop() {
            let Some(nodes) = class.nodes.get(c as usize) else {
                continue;
            };
            members = members.saturating_add(nodes.len());
            if members > MAX_SET {
                return false;
            }
            for &v in nodes {
                let l = st.nodes[v].layer_clamped();
                let Some(layer) = self.layers.get(l) else {
                    continue;
                };
                let i = st.order_of[v];
                if layer.get(i) != Some(&v) {
                    continue;
                }
                let j = if delta > 0.0 {
                    i + 1
                } else {
                    i.wrapping_sub(1)
                };
                let Some(&u) = layer.get(j) else { continue };
                let (left, right) = if delta > 0.0 { (v, u) } else { (u, v) };
                let slack =
                    st.nodes[right].cross - st.nodes[left].cross - st.sep(left, right, self.o);
                if slack >= delta.abs() - SLACK {
                    continue;
                }
                if set.insert(class.of[u]) {
                    queue.push(class.of[u]);
                }
            }
        }
        // Only arcs with exactly one end in the set change length.
        let mut delta_cost = 0.0;
        for &c in &set {
            for &v in class.nodes.get(c as usize).into_iter().flatten() {
                let from = st.nodes[v].cross;
                for &(u, w) in &self.adj[v] {
                    if set.contains(&class.of[u]) {
                        continue;
                    }
                    let other = st.nodes[u].cross;
                    delta_cost += w * ((from + delta - other).abs() - (from - other).abs());
                }
            }
        }
        if delta_cost > COST_EPS {
            return false;
        }
        let mut touched: Vec<usize> = Vec::new();
        for &c in &set {
            for &v in class.nodes.get(c as usize).into_iter().flatten() {
                touched.extend(self.incident[v].iter().copied());
            }
        }
        touched.sort_unstable_by_dyn(Ord::cmp);
        touched.dedup();
        let before = self.stairs_of(st, &touched);
        for &c in &set {
            for &v in class.nodes.get(c as usize).into_iter().flatten() {
                st.nodes[v].cross += delta;
            }
        }
        let after = self.stairs_of(st, &touched);
        if after < before || delta_cost < -COST_EPS {
            return true;
        }
        for &c in &set {
            for &v in class.nodes.get(c as usize).into_iter().flatten() {
                st.nodes[v].cross -= delta;
            }
        }
        false
    }
}

/// The drawing's straight components, i.e., nodes joined by level arcs. Rebuilt whenever a slide
/// lands.
struct Classes {
    of: Vec<u32>,
    nodes: Vec<Vec<usize>>,
}

impl Classes {
    fn new(st: &LayeredState) -> Classes {
        let n = st.len();
        // Path halving without ranks, since the sets are shallow.
        let mut parent: Vec<u32> = (0..n as u32).collect();
        fn root(parent: &mut [u32], mut v: u32) -> u32 {
            while parent[v as usize] != v {
                let up = parent[v as usize];
                parent[v as usize] = parent[up as usize];
                v = parent[v as usize];
            }
            v
        }
        for arc in &st.arcs {
            if arc.from == arc.to
                || (st.nodes[arc.from].cross - st.nodes[arc.to].cross).abs() >= 0.5
            {
                continue;
            }
            let (ra, rb) = (
                root(&mut parent, arc.from as u32),
                root(&mut parent, arc.to as u32),
            );
            if ra != rb {
                parent[ra.max(rb) as usize] = ra.min(rb);
            }
        }
        let mut of = vec![NONE; n];
        let mut nodes: Vec<Vec<usize>> = Vec::new();
        for v in 0..n {
            let r = root(&mut parent, v as u32) as usize;
            if of[r] == NONE {
                of[r] = nodes.len() as u32;
                nodes.push(Vec::new());
            }
            of[v] = of[r];
            nodes[of[r] as usize].push(v);
        }
        Classes { of, nodes }
    }
}

/// Cost first, level changes second, lexicographically. Both are sums along the chain, so the DP
/// is sound, and breaking cost ties by changes keeps the walk on the optimal face.
#[derive(Clone, Copy)]
struct Val {
    cost: f64,
    changes: u32,
}

impl Val {
    const ZERO: Val = Val {
        cost: 0.0,
        changes: 0,
    };
    const INF: Val = Val {
        cost: f64::INFINITY,
        changes: u32::MAX,
    };

    fn plus(self, cost: f64, changes: u32) -> Val {
        Val {
            cost: self.cost + cost,
            changes: self.changes.saturating_add(changes),
        }
    }

    fn better(self, other: Val) -> bool {
        if (self.cost - other.cost).abs() <= COST_EPS {
            self.changes < other.changes
        } else {
            self.cost < other.cost
        }
    }
}

/// Re-places one chain's waypoints on the cheapest, then straightest, lanes their slots allow, in
/// `O(span * candidates)` via running minima below and above each lane.
fn walk_chain(walk: &Walk, st: &mut LayeredState, chain: &[usize], weight: f64) -> bool {
    let m = chain.len();
    if m < 3 {
        return false;
    }
    // Only interior waypoints move. Ends and interior real nodes carry costs the walk does not see.
    let free: Vec<bool> = (0..m)
        .map(|i| i > 0 && i + 1 < m && st.nodes[chain[i]].is_dummy())
        .collect();
    let dummies: Vec<usize> = (0..m).filter(|&i| free[i]).map(|i| chain[i]).collect();
    if dummies.is_empty() {
        return false;
    }
    let slots = walk.slots(st, &dummies);
    // Candidates include the current positions, so the walk can reproduce the simplex result and
    // never costs more.
    let target = chain_lane_target(st, chain, &dummies, &slots);
    let mut pool: Vec<f64> = chain.iter().map(|&v| st.nodes[v].cross).collect();
    pool.push(target);
    pool.push(best_lane(&slots, target));
    pool.extend(lane_levels(&slots, target));
    for &(lo, hi) in &slots {
        if lo.is_finite() {
            pool.push(lo);
        }
        if hi.is_finite() {
            pool.push(hi);
        }
    }
    pool.sort_unstable_by_dyn(f64::total_cmp);
    pool.dedup();
    let p = pool.len();

    // A fixed position may only take the lane it is on.
    let mut allowed: Vec<Vec<bool>> = Vec::with_capacity(m);
    let mut next_dummy = 0usize;
    for i in 0..m {
        let cur = st.nodes[chain[i]].cross;
        let mut row: Vec<bool> = if free[i] {
            let slot = slots[next_dummy];
            next_dummy += 1;
            pool.iter().map(|&y| admits(slot, y)).collect()
        } else {
            pool.iter().map(|&y| y == cur).collect()
        };
        // Staying put is always allowed, whatever the slot arithmetic says on a degenerate layer.
        if let Some(k) = pool.iter().position(|&y| y == cur) {
            row[k] = true;
        }
        allowed.push(row);
    }
    let step_weight: Vec<f64> = (0..m - 1)
        .map(|i| {
            weight
                * omega(
                    st.nodes[chain[i]].is_dummy(),
                    st.nodes[chain[i + 1]].is_dummy(),
                )
        })
        .collect();

    let mut val: Vec<Val> = (0..p)
        .map(|b| if allowed[0][b] { Val::ZERO } else { Val::INF })
        .collect();
    let mut parent: Vec<Vec<u32>> = vec![vec![NONE; p]; m];
    for i in 0..m - 1 {
        let w = step_weight[i];
        // The cost from a lane below splits into a term in `a` and one in `b`, so the best
        // predecessor below `b` is a running minimum of the `a` term. The same holds from above.
        let mut up: Vec<(Val, u32)> = vec![(Val::INF, NONE); p];
        let mut run = (Val::INF, NONE);
        for a in 0..p {
            let cand = val[a].plus(-w * pool[a], 1);
            if cand.better(run.0) {
                run = (cand, a as u32);
            }
            up[a] = run;
        }
        let mut down: Vec<(Val, u32)> = vec![(Val::INF, NONE); p];
        run = (Val::INF, NONE);
        for a in (0..p).rev() {
            let cand = val[a].plus(w * pool[a], 1);
            if cand.better(run.0) {
                run = (cand, a as u32);
            }
            down[a] = run;
        }
        let mut next = vec![Val::INF; p];
        for b in 0..p {
            if !allowed[i + 1][b] {
                continue;
            }
            let mut best = Val::INF;
            let mut arg = NONE;
            for (cand, a) in [
                (val[b].plus(0.0, 0), b as u32),
                (up[b].0.plus(w * pool[b], 0), up[b].1),
                (down[b].0.plus(-w * pool[b], 0), down[b].1),
            ] {
                if a != NONE && cand.better(best) {
                    best = cand;
                    arg = a;
                }
            }
            next[b] = best;
            parent[i + 1][b] = arg;
        }
        val = next;
    }

    let mut end = NONE;
    let mut best = Val::INF;
    for (b, &v) in val.iter().enumerate() {
        if v.better(best) {
            best = v;
            end = b as u32;
        }
    }
    if end == NONE {
        return false;
    }
    let mut current = Val::ZERO;
    for i in 0..m - 1 {
        let step = (st.nodes[chain[i + 1]].cross - st.nodes[chain[i]].cross).abs();
        current = current.plus(step_weight[i] * step, u32::from(step > 0.5));
    }
    if !best.better(current) {
        return false;
    }
    let mut path = vec![NONE; m];
    path[m - 1] = end;
    for i in (1..m).rev() {
        let prev = parent[i][path[i] as usize];
        if prev == NONE {
            return false;
        }
        path[i - 1] = prev;
    }
    for i in 0..m {
        if free[i] {
            st.nodes[chain[i]].cross = pool[path[i] as usize];
        }
    }
    true
}

/// Rounds a slot bound to the whole pixel inside the slot. Infinite bounds stay.
fn round_in(v: f64, is_low: bool) -> f64 {
    if !v.is_finite() {
        return v;
    }
    if is_low {
        v.ceil()
    } else {
        v.floor()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::geometry::shape::NodeShape;
    use crate::layered::dummy::{build_edge_chains, insert_dummies};
    use crate::layered::prepare::{LayeredInput, LayeredInputNode};
    use crate::options::{Direction, LayoutOptions};

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

    /// One long edge `0 -> 1` across `n_layers` ranks, with `blockers` real nodes in each
    /// intermediate rank bounding its waypoint. Layers are `[0]`, then `[waypoint, blocker]` or
    /// `[blocker, waypoint, blocker]`, then `[1]`.
    fn long_chain(n_layers: usize, blockers: usize) -> (LayeredState, LayoutOptions) {
        let mid = n_layers - 2;
        let input = LayeredInput {
            nodes: vec![node(100.0, 50.0); 2 + mid * blockers],
            edges: vec![(0, 1)],
            weights: vec![],
            thickness: vec![],
            edge_label_sizes: vec![],
            seed: vec![],
            pinned: vec![],
            self_loops: vec![],
            ranked_backwards: Vec::new(),
            options: LayoutOptions {
                direction: Direction::LeftRight,
                ..LayoutOptions::layered_default()
            },
        };
        let o = input.options;
        let mut st = LayeredState::new(&input);
        st.nodes[0].rank = 0;
        st.nodes[1].rank = n_layers as i32 - 1;
        for l in 0..mid {
            for k in 0..blockers {
                st.nodes[2 + l * blockers + k].rank = l as i32 + 1;
            }
        }
        insert_dummies(&mut st).unwrap();
        st.rebuild_layers();
        build_edge_chains(&mut st).unwrap();
        // Waypoints come after the real nodes, one per intermediate rank.
        let first_dummy = 2 + mid * blockers;
        st.layers = (0..n_layers)
            .map(|l| {
                if l == 0 {
                    return vec![0];
                }
                if l == n_layers - 1 {
                    return vec![1];
                }
                let d = first_dummy + l - 1;
                match blockers {
                    0 => vec![d],
                    1 => vec![d, 2 + l - 1],
                    _ => vec![blocker(blockers, l - 1, 0), d, blocker(blockers, l - 1, 1)],
                }
            })
            .collect();
        st.reindex();
        (st, o)
    }

    fn blocker(blockers: usize, l: usize, k: usize) -> usize {
        2 + l * blockers + k
    }

    fn waypoints(st: &LayeredState) -> Vec<usize> {
        let chain = st.chains.values().next().cloned().unwrap_or_default();
        chain[1..chain.len() - 1].to_vec()
    }

    /// Ends included.
    fn levels(st: &LayeredState) -> Vec<f64> {
        let chain = st.chains.values().next().cloned().unwrap_or_default();
        chain.iter().map(|&v| st.nodes[v].cross).collect()
    }

    /// The chain's cost under the placement program's `omega` weights.
    fn chain_cost(st: &LayeredState) -> f64 {
        let chain = st.chains.values().next().cloned().unwrap_or_default();
        chain
            .windows(2)
            .map(|w| {
                omega(st.nodes[w[0]].is_dummy(), st.nodes[w[1]].is_dummy())
                    * (st.nodes[w[1]].cross - st.nodes[w[0]].cross).abs()
            })
            .sum()
    }

    fn level_changes(st: &LayeredState) -> usize {
        levels(st)
            .windows(2)
            .filter(|w| (w[1] - w[0]).abs() > 0.5)
            .count()
    }

    fn separations_hold(st: &LayeredState, o: &LayoutOptions) {
        for layer in &st.layers {
            for pair in layer.windows(2) {
                let need = st.sep(pair[0], pair[1], o);
                let got = st.nodes[pair[1]].cross - st.nodes[pair[0]].cross;
                assert!(got >= need - 1e-6, "{pair:?}: {got} < {need}");
            }
        }
    }

    /// The chain walk alone, without the component slide that can widen a slot.
    fn walk_only(st: &mut LayeredState, o: &LayoutOptions) -> bool {
        st.reindex();
        let layers = st.layers.clone();
        let chains = st.chains.clone();
        let walk = Walk::new(st, o, &layers, &chains);
        let chain = chains.values().next().cloned().unwrap_or_default();
        walk_chain(&walk, st, &chain, 1.0)
    }

    /// Blockers pin the first waypoint to lane 0 and the last to 400, so the chain must cross 400
    /// px on inner segments all priced at 8. Spreading that out does not change the cost, so the
    /// walk takes the single jog.
    #[test]
    fn a_free_walk_puts_a_chain_on_one_lane() {
        let (mut st, o) = long_chain(6, 2);
        st.nodes[0].cross = 0.0;
        st.nodes[1].cross = 400.0;
        let dummies = waypoints(&st);
        for (l, &d) in dummies.iter().enumerate() {
            st.nodes[d].cross = [0.0, 100.0, 200.0, 400.0][l];
            let (below, above) = (blocker(2, l, 0), blocker(2, l, 1));
            // The two end waypoints are pinned by their blockers, the middle ones are free.
            let room = if l == 0 || l + 1 == dummies.len() {
                0.0
            } else {
                2000.0
            };
            st.nodes[below].cross = st.nodes[d].cross - st.sep(below, d, &o) - room;
            st.nodes[above].cross = st.nodes[d].cross + st.sep(d, above, &o) + room;
        }
        let before = (chain_cost(&st), level_changes(&st));
        assert_eq!(before.1, 3, "the staircase steps in every free layer");
        assert!(walk_only(&mut st, &o), "the walk found nothing to do");
        let after = (chain_cost(&st), level_changes(&st));
        assert!(
            (after.0 - before.0).abs() < 1e-6,
            "the walk changed the cost: {before:?} -> {after:?}"
        );
        assert_eq!(after.1, 1, "one jog, not three: {:?}", levels(&st));
        separations_hold(&st, &o);
    }

    /// The chain is straight with both ends on one lane, so any move pays for two slanted segments.
    #[test]
    fn a_straight_chain_is_left_alone() {
        let (mut st, o) = long_chain(6, 1);
        st.nodes[0].cross = 0.0;
        st.nodes[1].cross = 0.0;
        for &d in &waypoints(&st) {
            st.nodes[d].cross = 0.0;
        }
        for l in 0..4 {
            st.nodes[blocker(1, l, 0)].cross = 2000.0;
        }
        let before = levels(&st);
        settle_chains(&mut st, &o);
        assert_eq!(levels(&st), before, "a straight chain has nothing to gain");
    }

    /// Blockers sit exactly two separations apart around each waypoint, so every slot is one point
    /// and the staircase is forced. The pass must not invent room.
    #[test]
    fn a_packed_layer_leaves_nothing_free() {
        let (mut st, o) = long_chain(6, 2);
        st.nodes[0].cross = 0.0;
        st.nodes[1].cross = 400.0;
        for (l, &d) in waypoints(&st).iter().enumerate() {
            let want = 100.0 * (l as f64 + 1.0);
            let (below, above) = (blocker(2, l, 0), blocker(2, l, 1));
            st.nodes[d].cross = want;
            st.nodes[below].cross = want - st.sep(below, d, &o);
            st.nodes[above].cross = want + st.sep(d, above, &o);
        }
        let before = levels(&st);
        assert!(!walk_only(&mut st, &o), "there is no free move to find");
        assert_eq!(
            levels(&st),
            before,
            "a waypoint with a one-point slot has nowhere to go"
        );
        separations_hold(&st, &o);
    }

    /// A long edge `0 -> 1` over four ranks, a blocker (2) under its second waypoint, and a puller
    /// (3) holding the blocker by an edge of weight `pull`. Layers are `[0]`, `[w1, puller]`,
    /// `[blocker, w2]`, `[1]`. Also returns the blocker's separation, i.e., how far the waypoint is
    /// pushed.
    fn blocked_chain(pull: f64) -> (LayeredState, LayoutOptions, f64) {
        let input = LayeredInput {
            nodes: vec![node(100.0, 50.0); 4],
            edges: vec![(0, 1), (3, 2)],
            weights: vec![1.0, pull],
            thickness: vec![],
            edge_label_sizes: vec![],
            seed: vec![],
            pinned: vec![],
            self_loops: vec![],
            ranked_backwards: Vec::new(),
            options: LayoutOptions {
                direction: Direction::LeftRight,
                ..LayoutOptions::layered_default()
            },
        };
        let o = input.options;
        let mut st = LayeredState::new(&input);
        for (i, r) in [0i32, 3, 2, 1].iter().enumerate() {
            st.nodes[i].rank = *r;
        }
        insert_dummies(&mut st).unwrap();
        st.rebuild_layers();
        build_edge_chains(&mut st).unwrap();
        let (w1, w2) = (4, 5);
        st.layers = vec![vec![0], vec![w1, 3], vec![2, w2], vec![1]];
        st.reindex();
        // The chain wants to run level at 0. The blocker pushes the second waypoint a separation
        // clear, and the puller sits a separation above the first.
        let sep = st.sep(2, w2, &o);
        for v in [0, 1, 2, w1] {
            st.nodes[v].cross = 0.0;
        }
        st.nodes[w2].cross = sep;
        st.nodes[3].cross = st.sep(w1, 3, &o);
        (st, o, sep)
    }

    fn total_cost(st: &LayeredState) -> f64 {
        st.arcs
            .iter()
            .filter(|a| a.from != a.to)
            .map(|a| {
                st.weight.get(a.edge).copied().unwrap_or(1.0)
                    * omega(st.nodes[a.from].is_dummy(), st.nodes[a.to].is_dummy())
                    * (st.nodes[a.to].cross - st.nodes[a.from].cross).abs()
            })
            .sum()
    }

    /// The waypoint's slot is one point, so only moving the blocker frees the lane. At `pull = 10`
    /// the holding edge costs exactly what the chain's two slants save, and the pass takes the tie.
    #[test]
    fn a_free_component_slide_levels_a_blocked_chain() {
        let (mut st, o, _) = blocked_chain(10.0);
        let before = total_cost(&st);
        settle_chains(&mut st, &o);
        assert!(
            (total_cost(&st) - before).abs() < 1e-6,
            "the slide was not free: {before} -> {}",
            total_cost(&st)
        );
        assert_eq!(
            levels(&st),
            vec![0.0; 4],
            "the chain is not level: {:?}",
            levels(&st)
        );
        separations_hold(&st, &o);
    }

    /// Held twice as hard, every slide raises the objective. A straighter drawing exists but is off
    /// the optimal face, so it is not taken.
    #[test]
    fn a_slide_that_costs_is_refused() {
        let (mut st, o, sep) = blocked_chain(20.0);
        let before = total_cost(&st);
        settle_chains(&mut st, &o);
        assert!(
            total_cost(&st) <= before + 1e-6,
            "the pass raised the objective: {before} -> {}",
            total_cost(&st)
        );
        assert_eq!(
            st.nodes[2].cross, 0.0,
            "the blocker was moved out of the way"
        );
        assert_eq!(st.nodes[5].cross, sep, "the waypoint is still pushed aside");
        separations_hold(&st, &o);
    }

    /// The walk sorts by `total_cmp`, breaks ties by index and reads no clock.
    #[test]
    fn the_walk_is_deterministic() {
        let build = || {
            let (mut st, o) = long_chain(7, 1);
            st.nodes[0].cross = 0.0;
            st.nodes[1].cross = 300.0;
            for (k, &d) in waypoints(&st).iter().enumerate() {
                st.nodes[d].cross = 60.0 * k as f64;
            }
            for l in 0..5 {
                st.nodes[blocker(1, l, 0)].cross = 900.0 + 10.0 * l as f64;
            }
            (st, o)
        };
        let (mut a, o) = build();
        let (mut b, _) = build();
        settle_chains(&mut a, &o);
        settle_chains(&mut b, &o);
        let bits = |st: &LayeredState| {
            st.nodes
                .iter()
                .map(|n| n.cross.to_bits())
                .collect::<Vec<_>>()
        };
        assert_eq!(bits(&a), bits(&b));
    }

    #[test]
    fn zero_rounds_does_nothing() {
        let (mut st, mut o) = long_chain(6, 1);
        o.tuning.coords.simplex_face_rounds = 0;
        st.nodes[0].cross = 0.0;
        st.nodes[1].cross = 400.0;
        for (k, &d) in waypoints(&st).iter().enumerate() {
            st.nodes[d].cross = 100.0 * k as f64;
        }
        for l in 0..4 {
            st.nodes[blocker(1, l, 0)].cross = 2000.0;
        }
        let before = levels(&st);
        settle_chains(&mut st, &o);
        assert_eq!(levels(&st), before);
    }
}

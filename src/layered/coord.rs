//! Coordinate assignment. [`assign_layer_positions`] spaces the layer columns, and the rest places
//! nodes on the order axis: Brandes-Kopf as the base, then the straightness passes (snap-align,
//! long-edge straightening) or the seeded placement for a stable relayout.
//!
//! Every pass here and in `compact` takes the spacing of order-adjacent nodes from
//! [`LayeredState::sep`].

use super::simplex;
use super::state::LayeredState;
use crate::error::LayoutError;
use crate::geometry::monotone::{project_gaps, project_gaps_pinned, project_gaps_weighted};
use crate::geometry::stats::weighted_median;
use crate::geometry::{EPS, SLACK};
use crate::options::{LayoutOptions, Placement};
use crate::sort::SortDyn;

/// Cross-axis placement with the configured optimiser, then the seeds if any.
pub fn place_cross(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    if st.has_seeds() {
        // The warm start is the fresh layout's placement, so a relayout with every node seeded
        // where the fresh one put it reproduces the fresh drawing, waypoints included.
        if !(o.coords.placement == Placement::NetworkSimplex
            && simplex::place_cross_simplex(st, o)?)
        {
            brandes_koepf(st, o)?;
        }
        return seeded_coords(st, o);
    }
    if o.coords.placement == Placement::NetworkSimplex && simplex::place_cross_simplex(st, o)? {
        return Ok(());
    }
    // Brandes-Kopf was requested, or the simplex declined because the graph is too large or no
    // feasible tree fit the budget.
    brandes_koepf(st, o)?;
    snap_align(st, o)
}

/// Projects `cross` per layer onto the closest positions that honour [`LayeredState::sep`],
/// keeping each layer's order. Brandes-Kopf can leave a layer overlapping or out of order. [`chain_prices`] makes real nodes move before waypoints, since a
/// waypoint moved alone is a jog that [`straighten_long_edges`] may not undo.
pub(super) fn legalize(st: &LayeredState, o: &LayoutOptions, cross: &mut [f64]) {
    for layer in &st.layers {
        if layer.len() < 2 {
            continue;
        }
        let d: Vec<f64> = layer.iter().map(|&v| cross[v]).collect();
        let gap: Vec<f64> = (0..layer.len() - 1)
            .map(|i| st.sep(layer[i], layer[i + 1], o))
            .collect();
        if gaps_hold(&d, &gap) {
            continue;
        }
        let placed = project_gaps_weighted(&d, &gap, &chain_prices(st, layer));
        for (i, &v) in layer.iter().enumerate() {
            cross[v] = placed[i];
        }
    }
}

pub(super) fn chain_prices(st: &LayeredState, layer: &[usize]) -> Vec<f64> {
    layer
        .iter()
        .map(|&v| {
            if st.nodes[v].is_dummy() {
                CHAIN_STIFFNESS
            } else {
                1.0
            }
        })
        .collect()
}

/// Cost of a pixel of waypoint displacement, against 1.0 for a real node. It matches the network
/// simplex weight `omega` = 8 on a dummy-dummy segment (Gansner et al. 1993).
pub(super) const CHAIN_STIFFNESS: f64 = 8.0;

/// Tolerates pixel-invisible slack. Gaps are strictly positive, so holding them also means the
/// layer order is preserved, which [`super::splits`] relies on.
pub(super) fn gaps_hold(placed: &[f64], gap: &[f64]) -> bool {
    placed
        .windows(2)
        .zip(gap)
        .all(|(w, &g)| w[1] - w[0] >= g - SLACK)
}

/// Centres each rank-pinned terminal (a START/END node) on the weighted median of its neighbours,
/// then re-projects the layer so terminals sharing a neighbour do not stack. Non-terminals stay
/// pinned unless that is infeasible, so a terminal never drags the flow it is centred on.
pub fn centre_terminals(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    let n_all = st.len();
    let mut nb: Vec<Vec<(usize, f64)>> = vec![vec![]; n_all];
    for arc in &st.arcs {
        let (a, b) = (arc.from, arc.to);
        if a == b {
            continue;
        }
        let w = st.weight.get(arc.edge).copied().unwrap_or(1.0);
        nb[a].push((b, w));
        nb[b].push((a, w));
    }
    let mut desired: Vec<f64> = st.nodes.iter().map(|nd| nd.cross).collect();
    let mut moved = vec![false; n_all];
    for i in 0..st.n_real {
        if !st.nodes[i]
            .constraint
            .is_some_and(super::state::LayerConstraint::is_terminal)
            || nb[i].is_empty()
        {
            continue;
        }
        // Weighted, so the terminal sits over its dominant flow.
        let mut s: Vec<(f64, f64)> = nb[i].iter().map(|&(u, w)| (st.nodes[u].cross, w)).collect();
        desired[i] = weighted_median(&mut s);
        moved[i] = true;
    }
    if !moved.iter().any(|&m| m) {
        return Ok(());
    }
    let layers = std::mem::take(&mut st.layers);
    for layer in &layers {
        if !layer.iter().any(|&v| moved[v]) {
            continue;
        }
        if layer.len() < 2 {
            if let Some(&v) = layer.first() {
                st.nodes[v].cross = desired[v];
            }
            continue;
        }
        let d: Vec<f64> = layer.iter().map(|&v| desired[v]).collect();
        let gap: Vec<f64> = (0..layer.len() - 1)
            .map(|i| st.sep(layer[i], layer[i + 1], o))
            .collect();
        let fixed: Vec<Option<f64>> = layer
            .iter()
            .map(|&v| (!moved[v]).then_some(st.nodes[v].cross))
            .collect();
        let mut placed = project_gaps_pinned(&d, &gap, &fixed);
        if !gaps_hold(&placed, &gap) {
            placed = project_gaps(&d, &gap);
        }
        for (i, &v) in layer.iter().enumerate() {
            st.nodes[v].cross = placed[i];
        }
    }
    st.layers = layers;
    Ok(())
}

/// Widens each layer gap to hold its risers `Routing::lane_gap` apart, then re-spaces the layers.
/// Runs once the cross axis is final, so only passages that change height count.
pub fn reserve_lane_room(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    let n_layers = st.layers.len();
    if n_layers < 2 || o.routing.lane_gap <= 0.0 {
        return Ok(());
    }
    let mut risers = vec![0usize; n_layers];
    for chain in st.chains.values() {
        for w in chain.windows(2) {
            let l = st.nodes[w[0]].layer();
            if l < n_layers && (st.nodes[w[0]].cross - st.nodes[w[1]].cross).abs() >= 0.5 {
                risers[l] += 1;
            }
        }
    }
    let mut gap = std::mem::take(&mut st.gap_after);
    if gap.len() < n_layers {
        gap.resize(n_layers, o.spacing.layer_gap);
    }
    let mut widened = false;
    for (l, &k) in risers.iter().enumerate() {
        let need = (k as f64 + 1.0) * o.routing.lane_gap;
        if need > gap[l] {
            gap[l] = need;
            widened = true;
        }
    }
    let result = if widened {
        assign_layer_positions(st, &gap)
    } else {
        Ok(())
    };
    st.gap_after = gap;
    result
}

/// Heaviest first, ties by arc index. All chain passes use this order so each settles a chain
/// against the same neighbours. One comparator also means one `sort` instantiation in wasm.
pub(super) fn heaviest_first(a: &(usize, f64), b: &(usize, f64)) -> std::cmp::Ordering {
    b.1.total_cmp(&a.1).then(a.0.cmp(&b.0))
}

/// Pulls each long edge's dummy chain onto as few order-lanes as possible, heaviest first. Real
/// nodes do not move.
///
/// A chain is bounded only by real nodes and already aligned chains, so two parallel long edges do
/// not clamp against each other's staircases. A final [`legalize`] covers any shortfall.
pub fn straighten_long_edges(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    st.reindex();
    let layers = std::mem::take(&mut st.layers);
    let chains = std::mem::take(&mut st.chains);
    let mut arcs: Vec<(usize, f64)> = chains
        .iter()
        .filter(|(_, c)| c.len() >= 3 && c[1..c.len() - 1].iter().any(|&n| st.nodes[n].is_dummy()))
        .map(|(&a, _)| (a, st.weight.get(a).copied().unwrap_or(1.0)))
        .collect();
    arcs.sort_unstable_by_dyn(heaviest_first);
    // Settled nodes may bound a chain: every real node, and the dummies of aligned chains.
    let mut settled: Vec<bool> = (0..st.len()).map(|v| !st.nodes[v].is_dummy()).collect();
    for (arc, _) in &arcs {
        let chain = &chains[arc];
        let dummies: Vec<usize> = chain[1..chain.len() - 1]
            .iter()
            .copied()
            .filter(|&n| st.nodes[n].is_dummy())
            .collect();
        if dummies.is_empty() {
            continue;
        }
        let slots: Vec<(f64, f64)> = dummies
            .iter()
            .map(|&d| dummy_slot(st, &layers, &settled, d, o))
            .collect();
        let target = chain_lane_target(st, chain, &dummies, &slots);
        // The end heights are worth trying even when no lane fits every slot. A chain level out of
        // its source and level into its target has one jog wherever it lands.
        let ends = [
            st.nodes[chain[0]].cross,
            st.nodes[chain[chain.len() - 1]].cross,
        ];
        let lanes = lane_plan(&slots, target, &ends);
        for (&d, (&(lo, hi), &lane)) in dummies.iter().zip(slots.iter().zip(&lanes)) {
            if lo <= hi {
                st.nodes[d].cross = lane.clamp(lo, hi);
            }
            settled[d] = true;
        }
    }
    let mut cross: Vec<f64> = st.nodes.iter().map(|n| n.cross).collect();
    st.layers = layers;
    legalize(st, o, &mut cross);
    for (node, &c) in st.nodes.iter_mut().zip(&cross) {
        node.cross = c;
    }
    st.chains = chains;
    Ok(())
}

/// The heights dummy `d` can take between the nearest settled nodes on either side, less the
/// separations of everything between. Unsettled nodes only reserve room. A bound is infinite when
/// no settled node lies on that side.
pub(super) fn dummy_slot(
    st: &LayeredState,
    layers: &[Vec<usize>],
    settled: &[bool],
    d: usize,
    o: &LayoutOptions,
) -> (f64, f64) {
    let l = st.nodes[d].layer();
    let layer = &layers[l];
    let i = st.order_of[d];
    let mut lo = f64::NEG_INFINITY;
    let mut need = 0.0;
    for j in (0..i).rev() {
        need += st.sep(layer[j], layer[j + 1], o);
        if settled[layer[j]] {
            lo = st.nodes[layer[j]].cross + need;
            break;
        }
    }
    let mut hi = f64::INFINITY;
    need = 0.0;
    for j in i + 1..layer.len() {
        need += st.sep(layer[j - 1], layer[j], o);
        if settled[layer[j]] {
            hi = st.nodes[layer[j]].cross - need;
            break;
        }
    }
    (lo, hi)
}

/// Prefers an end height every slot admits, the source's over the target's, so the route jogs
/// once instead of at both ends. Otherwise returns the median of the current dummy heights.
pub(super) fn chain_lane_target(
    st: &LayeredState,
    chain: &[usize],
    dummies: &[usize],
    slots: &[(f64, f64)],
) -> f64 {
    let mut ys: Vec<f64> = dummies.iter().map(|&d| st.nodes[d].cross).collect();
    ys.sort_unstable_by_dyn(f64::total_cmp);
    let median = ys.get(ys.len() / 2).copied().unwrap_or(0.0);
    let (Some(&first), Some(&last)) = (chain.first(), chain.last()) else {
        return median;
    };
    let mut target = median;
    for y in [st.nodes[last].cross, st.nodes[first].cross] {
        if covered(slots, y) == slots.len() {
            target = y;
        }
    }
    target
}

pub(super) fn covered(slots: &[(f64, f64)], y: f64) -> usize {
    slots.iter().filter(|s| admits(**s, y)).count()
}

/// Tolerates pixel-invisible slack.
pub(super) fn admits(s: (f64, f64), y: f64) -> bool {
    s.0 <= y + EPS && y <= s.1 + EPS
}

/// Returns `(jogs, steps)`. A jog is a level change between consecutive chain nodes, and a step is
/// two consecutive jogs in the same direction, as in `LayoutMetrics::staircase_steps`. Endpoints
/// count, since a route that jogs twice on the way in is a staircase too.
pub(super) fn chain_jogs(cross: &[f64], chain: &[usize]) -> (usize, usize) {
    let (mut jogs, mut steps) = (0usize, 0usize);
    let mut last: Option<bool> = None;
    for w in chain.windows(2) {
        let d = cross[w[1]] - cross[w[0]];
        if d.abs() < LEVEL_PX {
            continue;
        }
        let sign = d > 0.0;
        jogs += 1;
        if last == Some(sign) {
            steps += 1;
        }
        last = Some(sign);
    }
    (jogs, steps)
}

/// The smallest level change the drawing shows. Lanes closer than a pixel draw as one lane.
pub(super) const LEVEL_PX: f64 = 1.0;

/// Weight of a real node against 1.0 for a waypoint when a seeded layer that keeps its structure is
/// re-projected. Large enough that the waypoint always gives way.
const SEEDED_REAL_STIFFNESS: f64 = 1000.0;

/// Budget of `(position, lane)` states [`lane_plan`] may sweep for one chain. The sweep is
/// quadratic in the span, so past this the lanes are thinned to those nearest the aim and the plan
/// stays exact over that smaller choice.
const LANE_PLAN_STATES: usize = 1 << 18;

const LANE_PLAN_MIN_LANES: usize = 8;

/// [`lane_plan`] without end heights. Pass the ends when they are known, since a lane at an end
/// height saves a jog and that height need not be a slot bound.
pub(super) fn lane_levels(slots: &[(f64, f64)], target: f64) -> Vec<f64> {
    lane_plan(slots, target, &[])
}

/// Each waypoint's lane, with the fewest level changes the slots allow and, among those, the plan
/// nearest `target`. A slot no candidate lane fits leaves that waypoint free.
///
/// A shortest path, since greedy coverage can turn `a a a b b b` into `a a b a b b`. A level change
/// costs one wherever it lands, so one running minimum over the previous row gives the best
/// predecessor and the sweep is `O(span * lanes)`.
pub(super) fn lane_plan(slots: &[(f64, f64)], target: f64, extra: &[f64]) -> Vec<f64> {
    let m = slots.len();
    if m == 0 {
        return vec![];
    }
    let pool = lane_pool(slots, target, extra);
    let p = pool.len();
    if p == 0 {
        return vec![target; m];
    }
    // `None` means no plan reaches this lane here.
    let better = |a: Option<(u32, f64)>, b: Option<(u32, f64)>| -> bool {
        match (a, b) {
            (Some(x), Some(y)) => x.0 < y.0 || (x.0 == y.0 && x.1 < y.1 - EPS),
            (Some(_), None) => true,
            _ => false,
        }
    };
    // A slot that admits no lane constrains nothing. Computed once per waypoint, since per state it
    // would square the sweep.
    let open: Vec<bool> = (0..m)
        .map(|i| !pool.iter().any(|&y| admits(slots[i], y)))
        .collect();
    let allows = |i: usize, b: usize| open[i] || admits(slots[i], pool[b]);
    let mut val: Vec<Option<(u32, f64)>> = (0..p)
        .map(|b| allows(0, b).then(|| (0u32, (pool[b] - target).abs())))
        .collect();
    let mut parent: Vec<u32> = vec![NO_LANE; m * p];
    for i in 1..m {
        let mut prev: Option<(u32, f64)> = None;
        let mut prev_at = NO_LANE;
        for (b, &v) in val.iter().enumerate() {
            if better(v, prev) {
                prev = v;
                prev_at = b as u32;
            }
        }
        let mut next: Vec<Option<(u32, f64)>> = vec![None; p];
        for b in 0..p {
            if !allows(i, b) {
                continue;
            }
            let drift = (pool[b] - target).abs();
            let stay = val[b].map(|(c, d)| (c, d + drift));
            let mut best = stay;
            let mut at = if stay.is_some() { b as u32 } else { NO_LANE };
            if let Some((c, d)) = prev {
                let moved = Some((c + 1, d + drift));
                if better(moved, best) {
                    best = moved;
                    at = prev_at;
                }
            }
            next[b] = best;
            parent[i * p + b] = at;
        }
        val = next;
    }
    let mut end = NO_LANE;
    let mut best: Option<(u32, f64)> = None;
    for (b, &v) in val.iter().enumerate() {
        if better(v, best) {
            best = v;
            end = b as u32;
        }
    }
    // Unreachable given `open`, but the aim is a safe answer.
    if end == NO_LANE {
        return vec![target; m];
    }
    let mut out = vec![target; m];
    let mut at = end;
    for i in (0..m).rev() {
        match pool.get(at as usize) {
            Some(&y) => out[i] = y,
            None => break,
        }
        if i == 0 {
            break;
        }
        at = parent[i * p + at as usize];
        if at == NO_LANE {
            break;
        }
    }
    out
}

const NO_LANE: u32 = u32::MAX;

/// The aim, the extra heights and every finite slot bound, thinned past [`LANE_PLAN_STATES`]. An
/// L1 optimum along a path sits at a bound or a neighbour's height, so the bounds are all the
/// choices.
fn lane_pool(slots: &[(f64, f64)], target: f64, extra: &[f64]) -> Vec<f64> {
    let m = slots.len().max(1);
    let mut pool: Vec<f64> = Vec::with_capacity(2 * slots.len() + 1 + extra.len());
    if target.is_finite() {
        pool.push(target);
    }
    pool.extend(extra.iter().copied().filter(|y| y.is_finite()));
    for &(lo, hi) in slots {
        if lo.is_finite() {
            pool.push(lo);
        }
        if hi.is_finite() {
            pool.push(hi);
        }
    }
    pool.sort_unstable_by_dyn(f64::total_cmp);
    // Lanes under a pixel apart are one lane downstream, and packed layers repeat the same bound,
    // so this removes most of the sweep's width.
    pool.dedup_by(|a, b| (*a - *b).abs() < LEVEL_PX);
    let room = (LANE_PLAN_STATES / m).max(LANE_PLAN_MIN_LANES);
    if pool.len() > room {
        pool.sort_unstable_by_dyn(|a, b| {
            (a - target)
                .abs()
                .total_cmp(&(b - target).abs())
                .then(a.total_cmp(b))
        });
        pool.truncate(room);
        pool.sort_unstable_by_dyn(f64::total_cmp);
    }
    pool
}

/// `target` clamped into the common range if there is one, else the height inside the most slots,
/// nearest `target` among those.
pub(super) fn best_lane(slots: &[(f64, f64)], target: f64) -> f64 {
    let lo = slots.iter().map(|s| s.0).fold(f64::NEG_INFINITY, f64::max);
    let hi = slots.iter().map(|s| s.1).fold(f64::INFINITY, f64::min);
    if lo <= hi {
        return target.clamp(lo, hi);
    }
    let mut cands: Vec<f64> = vec![target];
    for &(a, b) in slots {
        if a.is_finite() {
            cands.push(a);
        }
        if b.is_finite() {
            cands.push(b);
        }
    }
    let mut best = target;
    let mut best_key = (covered(slots, target), 0.0f64);
    for &y in &cands[1..] {
        let key = (covered(slots, y), -(y - target).abs());
        if key.0 > best_key.0 || (key.0 == best_key.0 && key.1 > best_key.1) {
            best = y;
            best_key = key;
        }
    }
    best
}

/// Each real node at its seed (dragged nodes pinned), dummies at the Brandes-Kopf warm start,
/// then a minimal projection restores order and separation.
pub fn seeded_coords(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    let order_seed =
        |st: &LayeredState, i: usize| -> Option<f64> { st.nodes[i].seed.map(|(_, cross)| cross) };
    let desired: Vec<f64> = (0..st.len())
        .map(|v| {
            if v < st.n_real {
                order_seed(st, v).unwrap_or(st.nodes[v].cross)
            } else {
                st.nodes[v].cross
            }
        })
        .collect();
    let layers = std::mem::take(&mut st.layers);
    for layer in &layers {
        if layer.len() < 2 {
            if let Some(&v) = layer.first() {
                st.nodes[v].cross = desired[v];
            }
            continue;
        }
        let d: Vec<f64> = layer.iter().map(|&v| desired[v]).collect();
        let gap: Vec<f64> = (0..layer.len() - 1)
            .map(|i| st.sep(layer[i], layer[i + 1], o))
            .collect();
        let fixed: Vec<Option<f64>> = layer
            .iter()
            .map(|&v| {
                if v < st.n_real && st.nodes[v].pinned {
                    order_seed(st, v)
                } else {
                    None
                }
            })
            .collect();
        // The straight-line guess at a waypoint may cross a real node the seeded drawing routed
        // around, so the waypoint gives way.
        let placed = if st.keeps_structure() && fixed.iter().all(Option::is_none) {
            let w: Vec<f64> = layer
                .iter()
                .map(|&v| {
                    if v < st.n_real {
                        SEEDED_REAL_STIFFNESS
                    } else {
                        1.0
                    }
                })
                .collect();
            project_gaps_weighted(&d, &gap, &w)
        } else {
            project_gaps_pinned(&d, &gap, &fixed)
        };
        for (i, &v) in layer.iter().enumerate() {
            st.nodes[v].cross = placed[i];
        }
    }
    st.layers = layers;
    Ok(())
}

/// Snaps each real node onto the neighbour height that straightens the most edge weight. Only a
/// strict gain moves a node, and the node stays in its slot.
pub fn snap_align(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    let n = st.len();
    let mut nb: Vec<Vec<(usize, f64)>> = vec![vec![]; n];
    for arc in &st.arcs {
        let (a, b) = (arc.from, arc.to);
        if a == b {
            continue;
        }
        let w = st.weight.get(arc.edge).copied().unwrap_or(1.0);
        nb[a].push((b, w));
        nb[b].push((a, w));
    }
    let layers = std::mem::take(&mut st.layers);
    let mut ys: Vec<f64> = st.nodes.iter().map(|nd| nd.cross).collect();
    for _ in 0..o.tuning.coords.snap_rounds {
        let mut changed = false;
        for layer in &layers {
            for (i, &v) in layer.iter().enumerate() {
                if st.nodes[v].is_dummy() || nb[v].is_empty() {
                    continue;
                }
                let lo = if i > 0 {
                    ys[layer[i - 1]] + st.sep(layer[i - 1], v, o)
                } else {
                    f64::NEG_INFINITY
                };
                let hi = if i + 1 < layer.len() {
                    ys[layer[i + 1]] - st.sep(v, layer[i + 1], o)
                } else {
                    f64::INFINITY
                };
                // A layer tighter than `sep` has no legal slot, and `f64::clamp` would panic on
                // `lo > hi`.
                if lo > hi {
                    continue;
                }
                let cur = ys[v];
                let straight_at = |y: f64| -> f64 {
                    nb[v]
                        .iter()
                        .filter(|&&(u, _)| (ys[u] - y).abs() < 1.0)
                        .map(|&(_, w)| w)
                        .sum()
                };
                let mut best_y = cur;
                let mut best_cnt = straight_at(cur);
                for &(u, _) in &nb[v] {
                    let y = ys[u];
                    if y < lo - SLACK || y > hi + SLACK {
                        continue;
                    }
                    let c = straight_at(y);
                    // Ties keep the position nearest the current one, i.e., no move.
                    if c > best_cnt + EPS
                        || ((c - best_cnt).abs() < EPS && (y - cur).abs() < (best_y - cur).abs())
                    {
                        best_cnt = c;
                        best_y = y;
                    }
                }
                if (best_y - cur).abs() > 1.0 {
                    ys[v] = best_y.clamp(lo, hi);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    for (node, &y) in st.nodes.iter_mut().zip(ys.iter()) {
        node.cross = y;
    }
    st.layers = layers;
    Ok(())
}

/// One biased Brandes-Kopf pass: blocks (`root`/`align`) over a transformed layer stack, plus the
/// compaction state (`sink`/`shift`/`xs`) that [`BkPass::place_block`] fills in.
struct BkPass<'a> {
    /// Possibly reversed vertically and/or within each layer.
    ord: &'a [Vec<usize>],
    tlayer: Vec<usize>,
    tpos: Vec<usize>,
    root: Vec<usize>,
    align: Vec<usize>,
    sink: Vec<usize>,
    shift: Vec<f64>,
    xs: Vec<f64>,
    /// Right-biased pass. This swaps which node of an adjacent pair owns the `clear_after` room.
    flipped: bool,
}

/// One suspended [`BkPass::place_block`] call: block `v`, the `align` walk position `w`.
#[derive(Clone, Copy)]
struct PlaceFrame {
    v: usize,
    w: usize,
    /// `xs[v]` is claimed and `w` is walking the block.
    entered: bool,
    /// `(u_above, u)` while `u`'s block is placed further up the stack. Consumed where the
    /// recursive call would return.
    pending: Option<(usize, usize)>,
}

impl BkPass<'_> {
    /// Brandes-Kopf horizontal compaction of the block containing `v`, in recursive call order.
    /// Iterative because the recursion nests once per node of a layer and overflows a wasm stack.
    fn place_block(&mut self, v: usize, st: &LayeredState, o: &LayoutOptions) {
        let mut stack = vec![PlaceFrame {
            v,
            w: v,
            entered: false,
            pending: None,
        }];
        while let Some(&frame) = stack.last() {
            let top = stack.len() - 1;
            let (v, w) = (frame.v, frame.w);
            if !frame.entered {
                // A block already placed, or being placed lower on this stack, returns at once.
                if !self.xs[v].is_nan() {
                    stack.pop();
                    continue;
                }
                self.xs[v] = 0.0;
                stack[top].entered = true;
                stack[top].w = v;
                continue;
            }
            if let Some((u_above, u)) = frame.pending {
                stack[top].pending = None;
                if self.sink[v] == v {
                    self.sink[v] = self.sink[u];
                }
                // In a within-layer-reversed pass the transformed "above" node is on the true
                // positive side, so `w` owns the clearance.
                let owner = if self.flipped { w } else { u_above };
                let s = st.nodes[u_above].w_cross / 2.0
                    + st.gap(u_above, w, o).max(st.nodes[owner].clear_after)
                    + st.nodes[w].w_cross / 2.0;
                if self.sink[v] == self.sink[u] {
                    self.xs[v] = self.xs[v].max(self.xs[u] + s);
                } else {
                    let su = self.sink[u];
                    self.shift[su] = self.shift[su].min(self.xs[v] - self.xs[u] - s);
                }
            } else if self.tpos[w] > 0 {
                // Suspend and place the block above first. The arm above resumes the walk.
                let u_above = self.ord[self.tlayer[w]][self.tpos[w] - 1];
                stack[top].pending = Some((u_above, self.root[u_above]));
                stack.push(PlaceFrame {
                    v: self.root[u_above],
                    w: self.root[u_above],
                    entered: false,
                    pending: None,
                });
                continue;
            }
            let next = self.align[w];
            if next == v {
                stack.pop();
            } else {
                stack[top].w = next;
            }
        }
    }
}

/// Brandes-Kopf placement on the order axis: median-aligned blocks, four biased passes
/// (top/bottom x left/right), then a combination of them.
pub fn brandes_koepf(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    let n = st.len();
    let n_layers = st.layers.len();
    if n == 0 {
        return Ok(());
    }

    let mut adj: Vec<Vec<usize>> = vec![vec![]; n];
    let mut preds: Vec<Vec<usize>> = vec![vec![]; n];
    for arc in &st.arcs {
        let (from, to) = (arc.from, arc.to);
        adj[from].push(to);
        adj[to].push(from);
        if st.nodes[from].rank < st.nodes[to].rank {
            preds[to].push(from);
        } else if st.nodes[to].rank < st.nodes[from].rank {
            preds[from].push(to);
        }
    }

    let mut pos0 = vec![0usize; n];
    for layer in &st.layers {
        for (i, &v) in layer.iter().enumerate() {
            pos0[v] = i;
        }
    }

    // Type-1 conflicts: a non-inner segment crossing an inner (dummy-dummy) one. Inner segments
    // win, so long edges stay straight through their dummy chain.
    let inner_upper = |v: usize| -> Option<usize> {
        if st.nodes[v].is_dummy() {
            preds[v].iter().copied().find(|&u| st.nodes[u].is_dummy())
        } else {
            None
        }
    };
    let mut conflicts: std::collections::BTreeSet<(usize, usize)> = Default::default();
    for i in 0..n_layers.saturating_sub(1) {
        let (upper, lower) = (&st.layers[i], &st.layers[i + 1]);
        let mut k0 = 0usize;
        let mut scan = 0usize;
        for l1 in 0..lower.len() {
            let v = lower[l1];
            let w = inner_upper(v);
            if w.is_some() || l1 == lower.len() - 1 {
                let k1 = w.map(|w| pos0[w]).unwrap_or(upper.len().saturating_sub(1));
                for &vl in &lower[scan..=l1] {
                    for &u in &adj[vl] {
                        if st.nodes[u].layer() == i {
                            let up = pos0[u];
                            if (up < k0 || up > k1)
                                && !(st.nodes[u].is_dummy() && st.nodes[vl].is_dummy())
                            {
                                conflicts.insert((u.min(vl), u.max(vl)));
                            }
                        }
                    }
                }
                scan = l1 + 1;
                k0 = k1;
            }
        }
    }

    let align_compact = |ord: &[Vec<usize>], flipped: bool| -> Vec<f64> {
        let mut tlayer = vec![0usize; n];
        let mut tpos = vec![0usize; n];
        for (i, layer) in ord.iter().enumerate() {
            for (j, &v) in layer.iter().enumerate() {
                tlayer[v] = i;
                tpos[v] = j;
            }
        }
        let mut root: Vec<usize> = (0..n).collect();
        let mut align: Vec<usize> = (0..n).collect();
        // Each node aligns with one of its two median upper neighbours, with a monotone claim
        // pointer.
        for (i, layer) in ord.iter().enumerate().skip(1) {
            let mut prev: i64 = -1;
            for &v in layer {
                let mut ws: Vec<usize> = adj[v]
                    .iter()
                    .copied()
                    .filter(|&u| tlayer[u] + 1 == i)
                    .collect();
                ws.sort_unstable_by_key_dyn(|&u| tpos[u]);
                if ws.is_empty() {
                    continue;
                }
                let m = ws.len();
                for &w in &[ws[(m - 1) / 2], ws[m / 2]] {
                    if align[v] == v
                        && prev < tpos[w] as i64
                        && !conflicts.contains(&(w.min(v), w.max(v)))
                    {
                        align[w] = v;
                        root[v] = root[w];
                        align[v] = root[w];
                        prev = tpos[w] as i64;
                    }
                }
            }
        }
        let mut pass = BkPass {
            ord,
            tlayer,
            tpos,
            root,
            align,
            sink: (0..n).collect(),
            shift: vec![f64::INFINITY; n],
            xs: vec![f64::NAN; n],
            flipped,
        };
        for v in 0..n {
            if pass.root[v] == v {
                pass.place_block(v, st, o);
            }
        }
        let BkPass {
            root,
            sink,
            shift,
            mut xs,
            ..
        } = pass;
        for v in 0..n {
            xs[v] = xs[root[v]];
        }
        for v in 0..n {
            let s = sink[root[v]];
            if shift[s].is_finite() {
                xs[v] += shift[s];
            }
        }
        xs
    };

    // Reversing within a layer flips the axis, so those passes are negated afterwards.
    let mut assigns: Vec<Vec<f64>> = vec![];
    for up in [false, true] {
        for left in [false, true] {
            let mut ord: Vec<Vec<usize>> = st.layers.to_vec();
            if up {
                ord.reverse();
            }
            if !left {
                for l in &mut ord {
                    l.reverse();
                }
            }
            let mut y = align_compact(&ord, !left);
            if !left {
                for v in y.iter_mut() {
                    *v = -*v;
                }
            }
            // Legalized in true coordinates, since `sep` is asymmetric (the negative-side node owns
            // the clearance).
            legalize(st, o, &mut y);
            assigns.push(y);
        }
    }

    // Each pass is centred on its median. Aligning to the narrowest pass, as Brandes and Kopf do,
    // costs more crossings and collinear overlaps than it saves in area.
    for a in assigns.iter_mut() {
        let mut s = a.clone();
        s.sort_unstable_by_dyn(f64::total_cmp);
        let med = s[n / 2];
        for v in a.iter_mut() {
            *v -= med;
        }
    }
    // ELK-style balance takes, per node, the mean of the middle two of the four passes.
    let balanced: Vec<f64> = (0..n)
        .map(|v| {
            let mut vals = [assigns[0][v], assigns[1][v], assigns[2][v], assigns[3][v]];
            vals.sort_unstable_by_dyn(f64::total_cmp);
            (vals[1] + vals[2]) / 2.0
        })
        .collect();
    let mut cross = pick_placement(st, o, assigns, balanced);
    // The passes may order a pair differently, so their median needs legalizing too.
    legalize(st, o, &mut cross);
    for (node, &c) in st.nodes.iter_mut().zip(cross.iter()) {
        node.cross = c;
    }
    Ok(())
}

/// Keeps the balance or one of the four passes, whichever costs least in weighted edge length plus
/// [`CHAIN_STIFFNESS`] node gaps per staircase step. Whole placements compete because a per-node
/// mix bends chains that one pass kept straight. A pass may be at most
/// [`crate::CoordTuning::balance_width_slack`] percent wider than the balance.
fn pick_placement(
    st: &LayeredState,
    o: &LayoutOptions,
    assigns: Vec<Vec<f64>>,
    balanced: Vec<f64>,
) -> Vec<f64> {
    let extent = |y: &[f64]| -> f64 {
        let (lo, hi) = y
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), &v| {
                (l.min(v), h.max(v))
            });
        if hi >= lo {
            hi - lo
        } else {
            0.0
        }
    };
    let cost = |y: &[f64]| -> f64 {
        let length: f64 = st
            .arcs
            .iter()
            .filter(|a| a.from != a.to)
            .map(|a| (y[a.from] - y[a.to]).abs() * st.weight.get(a.edge).copied().unwrap_or(1.0))
            .sum();
        let steps: usize = st
            .chains
            .values()
            .filter(|c| c.len() >= 3)
            .map(|c| chain_jogs(y, c).1)
            .sum();
        length + steps as f64 * CHAIN_STIFFNESS * o.spacing.node_gap
    };
    let widest = extent(&balanced) * (1.0 + f64::from(o.tuning.coords.balance_width_slack) / 100.0);
    let mut best_cost = cost(&balanced);
    let mut best = balanced;
    for y in assigns {
        if extent(&y) > widest {
            continue;
        }
        let c = cost(&y);
        if c < best_cost {
            best_cost = c;
            best = y;
        }
    }
    best
}

/// Each layer is as wide as its widest node, and `gap_after[l]` is the space kept after layer `l`.
pub fn assign_layer_positions(st: &mut LayeredState, gap_after: &[f64]) -> Result<(), LayoutError> {
    let layers = std::mem::take(&mut st.layers);
    let mut x = 0.0f64;
    for (li, layer) in layers.iter().enumerate() {
        // All nodes of a layer share one centre line. The column extends by the largest half-width
        // plus clearance on each side.
        let back = layer
            .iter()
            .map(|&n| st.nodes[n].w_along / 2.0 + st.nodes[n].along_before)
            .fold(0.0f64, f64::max);
        let ahead = layer
            .iter()
            .map(|&n| st.nodes[n].w_along / 2.0 + st.nodes[n].along_after)
            .fold(0.0f64, f64::max);
        let centre_x = x + back;
        for &node in layer {
            st.nodes[node].along = centre_x;
        }
        x += back + ahead + gap_after[li];
    }
    st.layers = layers;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::geometry::shape::NodeShape;
    use crate::layered::prepare::{LayeredInput, LayeredInputNode};
    use crate::layered::state::LayerConstraint;
    use crate::options::{Direction, LayoutOptions};

    fn node(width: f64, height: f64, constraint: Option<LayerConstraint>) -> LayeredInputNode {
        LayeredInputNode {
            width,
            height,
            shape: NodeShape::Box,
            constraint,
            category: None,
            clear_after: 0.0,
            clear_before: 0.0,
            port_sides: None,
            lane_key: None,
            along_before: 0.0,
            along_after: 0.0,
        }
    }

    fn shared_successor() -> LayeredState {
        let input = LayeredInput {
            nodes: vec![
                node(80.0, 36.0, Some(LayerConstraint::First)),
                node(80.0, 36.0, Some(LayerConstraint::First)),
                node(120.0, 52.0, None),
            ],
            edges: vec![(0, 2), (1, 2)],
            weights: vec![],
            thickness: vec![],
            edge_label_sizes: vec![],
            seed: vec![],
            pinned: vec![],
            self_loops: vec![],
            ranked_backwards: Vec::new(),
            options: LayoutOptions {
                direction: Direction::TopBottom,
                ..LayoutOptions::process_map()
            },
        };
        let mut st = LayeredState::new(&input);
        st.nodes[2].rank = 1;
        st.nodes[0].cross = 0.0;
        st.nodes[1].cross = 200.0;
        st.nodes[2].cross = 100.0;
        st.rebuild_layers();
        st
    }

    /// Both terminals want the same median, so the projection keeps them `sep` apart, centred on it
    /// as a block and in their original order.
    #[test]
    fn terminals_sharing_a_neighbour_end_up_separated() {
        let o = LayoutOptions::process_map();
        let mut st = shared_successor();
        centre_terminals(&mut st, &o).unwrap();
        let (a, b) = (st.nodes[0].cross, st.nodes[1].cross);
        let need = st.sep(0, 1, &o);
        assert!(b - a >= need - 1e-6, "{a} .. {b} is closer than {need}");
        assert!(((a + b) / 2.0 - st.nodes[2].cross).abs() < 1e-6);
    }

    /// The projection is the identity on its feasible set, so a terminal with room is not moved.
    #[test]
    fn a_terminal_with_room_lands_exactly_on_its_median() {
        let o = LayoutOptions::process_map();
        let mut st = shared_successor();
        // Node 1 becomes an ordinary node far enough away to constrain nothing.
        st.nodes[1].constraint = None;
        st.nodes[1].cross = 600.0;
        st.nodes[0].cross = 0.0;
        centre_terminals(&mut st, &o).unwrap();
        assert_eq!(st.nodes[0].cross, st.nodes[2].cross);
        assert_eq!(st.nodes[1].cross, 600.0);
    }

    /// The graph (`fixtures/regression/bk-infeasible-layer.json`) has two block classes in one
    /// layer whose dependency the compaction never records, so the layer needs the projection.
    #[test]
    fn brandes_koepf_output_satisfies_separation() {
        let input = LayeredInput {
            nodes: vec![node(120.0, 52.0, None); 12],
            edges: vec![
                (0, 1),
                (1, 2),
                (2, 3),
                (2, 5),
                (5, 6),
                (5, 7),
                (5, 8),
                (2, 9),
                (10, 11),
                (1, 4),
                (1, 3),
            ],
            weights: vec![],
            thickness: vec![],
            edge_label_sizes: vec![],
            seed: vec![],
            pinned: vec![],
            self_loops: vec![],
            ranked_backwards: Vec::new(),
            options: LayoutOptions::layered_default(),
        };
        let o = input.options;
        let mut st = LayeredState::new(&input);
        crate::layered::cycle::break_cycles(&mut st).unwrap();
        crate::layered::rank::assign_ranks(&mut st).unwrap();
        crate::layered::dummy::insert_dummies(&mut st).unwrap();
        st.rebuild_layers();
        crate::layered::order::minimize_crossings(&mut st, &o).unwrap();
        brandes_koepf(&mut st, &o).unwrap();
        for layer in &st.layers {
            for w in layer.windows(2) {
                let need = st.sep(w[0], w[1], &o);
                let got = st.nodes[w[1]].cross - st.nodes[w[0]].cross;
                assert!(got >= need - 1e-6, "{w:?}: {got} < {need}");
            }
        }
    }

    /// A cheaper pass that leaves the balance's width slack is refused.
    #[test]
    fn a_pass_is_taken_only_if_it_stays_inside_the_width_slack() {
        let input = LayeredInput {
            nodes: vec![
                node(80.0, 36.0, None),
                node(80.0, 36.0, None),
                node(80.0, 36.0, None),
            ],
            edges: vec![(0, 1)],
            weights: vec![],
            thickness: vec![],
            edge_label_sizes: vec![],
            seed: vec![],
            pinned: vec![],
            self_loops: vec![],
            ranked_backwards: Vec::new(),
            options: LayoutOptions {
                direction: Direction::TopBottom,
                ..LayoutOptions::process_map()
            },
        };
        let mut st = LayeredState::new(&input);
        st.nodes[1].rank = 1;
        st.rebuild_layers();
        let mut o = LayoutOptions::process_map();

        // The balance bends the one edge by 50 px over a 100 px drawing. The candidate draws it
        // straight but is nine times as wide because of the node nothing pins.
        let balanced = vec![0.0, 50.0, 100.0];
        let straight_but_wide = vec![0.0, 0.0, 900.0];

        o.tuning.coords.balance_width_slack = 0;
        let picked = pick_placement(&st, &o, vec![straight_but_wide.clone()], balanced.clone());
        assert_eq!(picked, balanced, "a drawing 9x as wide is not a better one");

        o.tuning.coords.balance_width_slack = 900;
        let picked = pick_placement(&st, &o, vec![straight_but_wide.clone()], balanced);
        assert_eq!(
            picked, straight_but_wide,
            "inside the slack, straighter wins"
        );
    }

    /// `legalize` leaves a feasible layer alone and spreads an infeasible one without reordering.
    #[test]
    fn legalize_fixes_only_what_is_infeasible() {
        let o = LayoutOptions::layered_default();
        let mut st = shared_successor();
        st.nodes[1].constraint = None;
        let sep = st.sep(0, 1, &o);
        let mut cross = vec![0.0, sep + 10.0, 0.0];
        legalize(&st, &o, &mut cross);
        assert_eq!(cross[..2], [0.0, sep + 10.0]);
        let mut cross = vec![0.0, 5.0, 0.0];
        legalize(&st, &o, &mut cross);
        assert!(cross[1] - cross[0] >= sep - 1e-6);
        assert!(cross[0] < cross[1]);
    }

    #[test]
    fn centring_leaves_the_non_terminals_alone() {
        let o = LayoutOptions::process_map();
        let mut st = shared_successor();
        let before = st.nodes[2].cross;
        centre_terminals(&mut st, &o).unwrap();
        assert_eq!(st.nodes[2].cross, before);
    }

    #[test]
    fn lane_plan_takes_one_lane_where_one_lane_fits() {
        let slots = [(0.0, 200.0), (0.0, 200.0), (0.0, 200.0)];
        assert_eq!(lane_plan(&slots, 100.0, &[]), vec![100.0, 100.0, 100.0]);
    }

    /// The middle waypoint is pinned to one lane and the aim is admitted by the outer two. Greedy
    /// coverage draws `6 4 6` with two level changes, where following the middle lane costs one.
    #[test]
    fn lane_plan_beats_the_coverage_greedy_shape() {
        let slots = [(5.0, 9.0), (4.0, 4.0), (3.0, 7.0)];
        let plan = lane_plan(&slots, 6.0, &[]);
        assert_eq!(plan, vec![6.0, 4.0, 4.0]);
        assert_eq!(changes(&plan), 1);
        let greedy = greedy_levels(&slots, 6.0);
        assert_eq!(greedy, vec![6.0, 4.0, 6.0]);
        assert_eq!(changes(&greedy), 2);
    }

    #[test]
    fn lane_plan_steps_once_when_it_has_to() {
        let slots = [(0.0, 10.0), (0.0, 10.0), (90.0, 100.0), (90.0, 100.0)];
        let plan = lane_plan(&slots, 0.0, &[]);
        assert_eq!(changes(&plan), 1, "{plan:?}");
        assert_eq!(plan, vec![0.0, 0.0, 90.0, 90.0]);
    }

    /// `50` is no slot bound, but it is inside every slot and nearer the aim than either bound.
    #[test]
    fn lane_plan_uses_the_lanes_it_is_given() {
        let slots = [(0.0, 100.0), (0.0, 100.0)];
        assert_eq!(lane_plan(&slots, 50.0, &[50.0]), vec![50.0, 50.0]);
    }

    /// A slot no lane fits must not cause a panic or a NaN.
    #[test]
    fn lane_plan_survives_an_impossible_slot() {
        let slots = [(0.0, 0.0), (100.0, 50.0), (0.0, 0.0)];
        let plan = lane_plan(&slots, 0.0, &[]);
        assert_eq!(plan.len(), 3);
        assert!(plan.iter().all(|y| y.is_finite()), "{plan:?}");
        assert!(lane_plan(&[], 0.0, &[]).is_empty());
    }

    fn changes(plan: &[f64]) -> usize {
        plan.windows(2)
            .filter(|w| (w[1] - w[0]).abs() >= 1.0)
            .count()
    }

    /// Greedy by coverage, the baseline `lane_plan` has to beat.
    fn greedy_levels(slots: &[(f64, f64)], target: f64) -> Vec<f64> {
        let mut out = vec![target; slots.len()];
        let mut stack = vec![(0usize, slots.len(), target)];
        while let Some((from, to, aim)) = stack.pop() {
            let part = &slots[from..to];
            if part.is_empty() {
                continue;
            }
            let lane = best_lane(part, aim);
            for (i, &s) in part.iter().enumerate() {
                if admits(s, lane) {
                    out[from + i] = lane;
                }
            }
            let mut i = 0;
            while i < part.len() {
                if admits(part[i], lane) {
                    i += 1;
                    continue;
                }
                let start = i;
                while i < part.len() && !admits(part[i], lane) {
                    i += 1;
                }
                stack.push((from + start, from + i, lane));
            }
        }
        out
    }

    #[test]
    fn legalize_spreads_the_real_node_rather_than_the_waypoint() {
        let o = LayoutOptions::layered_default();
        let mut st = shared_successor();
        st.nodes[1].constraint = None;
        // Node 1 becomes a waypoint of layer 0, on top of real node 0.
        st.nodes[1].kind = crate::layered::state::LKind::Dummy;
        st.nodes[1].w_cross = 0.0;
        st.nodes[1].w_along = 0.0;
        st.rebuild_layers();
        let sep = st.sep(0, 1, &o);
        let mut cross = vec![100.0, 100.0, 0.0];
        legalize(&st, &o, &mut cross);
        assert!(cross[1] - cross[0] >= sep - 1e-6);
        // The waypoint holds its lane to within an eighth of the move.
        assert!(
            (cross[1] - 100.0).abs() * 8.0 <= (cross[0] - 100.0).abs() + 1e-9,
            "{cross:?}"
        );
    }

    /// One rank of `n` nodes, each its own block, so a recursive `place_block` would nest `n` deep.
    fn one_wide_rank(n: usize) -> LayeredState {
        let input = LayeredInput {
            nodes: (0..n).map(|_| node(120.0, 52.0, None)).collect(),
            edges: vec![],
            weights: vec![],
            thickness: vec![],
            edge_label_sizes: vec![],
            seed: vec![],
            pinned: vec![],
            self_loops: vec![],
            ranked_backwards: Vec::new(),
            options: LayoutOptions::default(),
        };
        let mut st = LayeredState::new(&input);
        for (i, nd) in st.nodes.iter_mut().enumerate() {
            nd.rank = 0;
            nd.cross = i as f64 * 200.0;
        }
        st.rebuild_layers();
        st
    }

    /// Brandes-Kopf on a 3 000-wide rank runs on a 512 KiB stack.
    #[test]
    fn place_block_survives_a_wide_rank_on_a_small_stack() {
        let placed = std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(|| {
                let o = LayoutOptions::default();
                let mut st = one_wide_rank(3_000);
                brandes_koepf(&mut st, &o).unwrap();
                st.nodes.iter().map(|nd| nd.cross).collect::<Vec<f64>>()
            })
            .expect("spawn")
            .join()
            .expect("a 3 000-wide rank must not overflow a 512 KiB stack");
        assert_eq!(placed.len(), 3_000);
        assert!(placed.iter().all(|c| c.is_finite()));
        let o = LayoutOptions::default();
        let st = one_wide_rank(3_000);
        for i in 1..placed.len() {
            let need = st.sep(i - 1, i, &o);
            assert!(
                placed[i] - placed[i - 1] >= need - 1e-6,
                "nodes {} and {i} are {} apart, need {need}",
                i - 1,
                placed[i] - placed[i - 1]
            );
        }
    }
}

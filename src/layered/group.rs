//! Group lanes: one band of the order axis per group.
//!
//! Every layer agrees on one band order, so a reader following one group never crosses another.
//! Ungrouped nodes may sit anywhere, and a node shared by two groups takes the mean of its
//! neighbours' bands. Shared nodes rarely allow a crossing-free banding, so the pass may spend
//! [`crate::options::OrderOptions::group_lane_budget_percent`] of the remaining crossings.

use super::order::{adjacent_swap_delta, count_crossings};
use super::state::LayeredState;
use crate::error::LayoutError;
use crate::geometry::stats::barycentre;
use crate::geometry::EPS;
use crate::options::{GroupLanes, LayoutOptions};
use crate::sort::SortDyn;

pub fn enforce_groups(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    let strict = o.order.group_lanes == GroupLanes::Strict;
    let layers = std::mem::take(&mut st.layers);
    st.layers = match Lanes::of(st, strict) {
        // No node carries a group, so the crossing minimiser's order stands.
        None => layers,
        Some(lanes) => {
            // The crossing-neutral pass is free and is the fallback when the banding is refused.
            let free = lanes.free_pass(&layers, o);
            match lanes.banded(&layers, &free, o, strict) {
                Some(mut banded) => {
                    lanes.sort_by_lane_key(st, &mut banded);
                    if strict {
                        attach_waypoints(st, &mut banded);
                    }
                    // Recorded for the coordinate side, which turns bands into strips.
                    st.band_count = lanes.ids.len();
                    st.band_of = banded.slots;
                    st.band_groups = banded
                        .order
                        .iter()
                        .map(|&id| (Some(id) != lanes.ungrouped).then_some(id))
                        .collect();
                    banded.stack
                }
                None => free,
            }
        }
    };
    st.reindex();
    Ok(())
}

/// In swimlanes, a waypoint joins the lane of the real node before it in its layer (after it, at
/// a layer's start). Its own nearest band could split another band's run in two.
fn attach_waypoints(st: &LayeredState, banded: &mut Banded) {
    for layer in &banded.stack {
        let first_real = layer
            .iter()
            .find(|&&v| v < st.n_real)
            .map(|&v| banded.slots[v]);
        let mut current: Option<usize> = None;
        for &v in layer {
            if v < st.n_real {
                current = Some(current.map_or(banded.slots[v], |c| c.max(banded.slots[v])));
            } else {
                let own = banded.slots[v];
                let lane = current.or(first_real).unwrap_or(own);
                let lane = current.map_or(lane, |c| c.max(lane));
                banded.slots[v] = lane;
                current = Some(lane);
            }
        }
    }
}

/// Pulls every band onto its own strip of the order axis, each as wide as its widest run. Runs
/// shift rigidly, so only edges across bands get longer.
///
/// The stack is squeezed toward one line by the largest factor (found by bisection) that stays
/// within [`crate::options::OrderOptions::group_lane_area_percent`]. Runs after compaction, which
/// would repack the slots, and before the straightener, which repairs the staircases shifted bands
/// leave. Does nothing on a seeded relayout.
pub fn align_lanes(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    if st.band_count < 2 || st.band_of.len() != st.len() || st.has_seeds() {
        return Ok(());
    }
    let strict = o.order.group_lanes == GroupLanes::Strict;
    // Swimlanes have an extra lane for ungrouped nodes, so the shortcut does not apply.
    if !strict && bands_are_already_strips(st) {
        return Ok(()); // the coordinate phases already drew the strips
    }
    let layers = std::mem::take(&mut st.layers);
    let plan = Strips::of(st, &layers, o);
    if strict {
        // Swimlanes have no area budget.
        plan.apply(st, &layers, 1.0);
        st.layers = layers;
        return Ok(());
    }
    let allowed = drawing_extent(st) * (1.0 + o.order.group_lane_area_percent as f64 / 100.0);

    // `needed` is the smallest squeeze whose bands do not overlap. If it does not fit the area
    // budget, the largest affordable squeeze is used.
    let needed = bisect(|squeeze| plan.bands_are_disjoint(squeeze));
    let squeeze = if plan.extent(needed) <= allowed {
        needed
    } else {
        // The extent test is monotone the other way, so bisect its complement.
        1.0 - bisect(|squeeze| plan.extent(1.0 - squeeze) <= allowed)
    };
    if plan.extent(squeeze) <= allowed {
        plan.apply(st, &layers, squeeze);
    }
    st.layers = layers;
    Ok(())
}

/// Measures strict swimlanes, one rectangle per band spanning the whole drawing along the layer
/// axis. Neighbouring lanes meet halfway across their gap.
pub fn measure_lanes(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    if o.order.group_lanes != GroupLanes::Strict
        || st.band_groups.is_empty()
        || st.band_of.len() < st.n_real
    {
        return Ok(());
    }
    let pad = o.spacing.node_gap / 2.0;
    let mut along = (f64::INFINITY, f64::NEG_INFINITY);
    let mut cross = vec![(f64::INFINITY, f64::NEG_INFINITY); st.band_groups.len()];
    for (i, nd) in st.nodes[..st.n_real].iter().enumerate() {
        along.0 = along.0.min(nd.along - nd.w_along / 2.0);
        along.1 = along.1.max(nd.along + nd.w_along / 2.0);
        if let Some(c) = cross.get_mut(st.band_of[i]) {
            c.0 = c.0.min(nd.cross - nd.w_cross / 2.0);
            c.1 = c.1.max(nd.cross + nd.w_cross / 2.0);
        }
    }
    for p in st.routes.iter().flatten() {
        along.0 = along.0.min(p.0);
        along.1 = along.1.max(p.0);
    }
    for (p, (w, _)) in st.label_anchors.iter().zip(&st.label_sizes) {
        if let Some(p) = p {
            along.0 = along.0.min(p.0 - w / 2.0);
            along.1 = along.1.max(p.0 + w / 2.0);
        }
    }
    if !along.0.is_finite() {
        return Ok(());
    }
    // Drawing order, which is band order unless a later phase swapped two.
    let mut lanes: Vec<usize> = (0..cross.len())
        .filter(|&k| cross[k].0.is_finite())
        .collect();
    lanes.sort_unstable_by_dyn(|&a, &b| {
        (cross[a].0 + cross[a].1)
            .total_cmp(&(cross[b].0 + cross[b].1))
            .then(a.cmp(&b))
    });
    let (a0, a1) = (along.0 - pad, along.1 + pad);
    st.swimlanes = lanes
        .iter()
        .enumerate()
        .map(|(j, &k)| {
            let lo = match j.checked_sub(1) {
                Some(p) => (cross[lanes[p]].1 + cross[k].0) / 2.0,
                None => cross[k].0 - pad,
            };
            let hi = match lanes.get(j + 1) {
                Some(&n) => (cross[k].1 + cross[n].0) / 2.0,
                None => cross[k].1 + pad,
            };
            (st.band_groups[k], (a0, a1, lo, hi))
        })
        .collect();
    Ok(())
}

/// The smallest `squeeze` in `0.0..=1.0` for which `holds` is true, or `1.0` if it never holds.
/// `holds` must be monotone in the squeeze.
fn bisect(holds: impl Fn(f64) -> bool) -> f64 {
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    if holds(lo) {
        return lo;
    }
    if !holds(hi) {
        return hi;
    }
    for _ in 0..SQUEEZE_STEPS {
        let mid = (lo + hi) / 2.0;
        if holds(mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

/// Twelve halvings pin the factor to within 0.025%, finer than any separation in the drawing.
const SQUEEZE_STEPS: u32 = 12;

/// The squeeze-independent part of the strip layout, so the search can try a factor without
/// touching a node.
struct Strips {
    runs: Vec<Vec<Run>>,
    /// Slot centres with the slots fully stacked, measured from the middle of the drawing so a
    /// squeeze to nothing leaves it in place.
    centre: Vec<f64>,
    /// Half the widest run of each band.
    half: Vec<f64>,
}

struct Run {
    slot: usize,
    /// Index range inside the layer, both ends included.
    from: usize,
    to: usize,
    lo: f64,
    hi: f64,
    /// Borders over the run's grouped nodes only. Waypoints help place the run, but
    /// `metrics::lane_purity` only sees grouped nodes.
    real: Option<(f64, f64)>,
    /// Room to the next run's first border, from [`LayeredState::sep`].
    clear_after: f64,
}

impl Strips {
    fn of(st: &LayeredState, layers: &[Vec<usize>], o: &LayoutOptions) -> Strips {
        let mut width = vec![0.0f64; st.band_count];
        let mut runs: Vec<Vec<Run>> = vec![];
        for layer in layers {
            let blocks = blocks_of(layer, &st.band_of);
            let mut row: Vec<Run> = vec![];
            for (bi, b) in blocks.iter().enumerate() {
                let (lo, hi) = extent(st, layer, b);
                width[b.slot] = width[b.slot].max(hi - lo);
                let clear_after = match (
                    layer.get(b.to),
                    blocks.get(bi + 1).and_then(|next| layer.get(next.from)),
                ) {
                    (Some(&a), Some(&c)) => {
                        let halves = (st.nodes[a].w_cross + st.nodes[c].w_cross) / 2.0;
                        st.sep(a, c, o) - halves
                    }
                    _ => 0.0,
                };
                row.push(Run {
                    slot: b.slot,
                    from: b.from,
                    to: b.to,
                    lo,
                    hi,
                    real: real_extent(st, layer, b),
                    clear_after,
                });
            }
            runs.push(row);
        }
        // Slots are a plain node gap apart. A pair wanting more (e.g., a self-loop) pushes its run
        // along instead of widening every slot.
        let mut centre = vec![0.0f64; st.band_count];
        let mut cursor = 0.0;
        for (k, c) in centre.iter_mut().enumerate() {
            *c = cursor + width[k] / 2.0;
            cursor += width[k] + o.spacing.node_gap;
        }
        // Centred on the drawing's middle so nodes move as little as possible.
        let shift = drawing_middle(st) - cursor / 2.0 + o.spacing.node_gap / 2.0;
        for c in centre.iter_mut() {
            *c += shift;
        }
        let half = width.iter().map(|w| w / 2.0).collect();
        Strips { runs, centre, half }
    }

    /// A squeeze of 1 is the full stack, 0 puts every band on the middle of the drawing.
    fn line(&self, slot: usize, squeeze: f64) -> f64 {
        let mid = self.middle();
        mid + (self.centre.get(slot).copied().unwrap_or(mid) - mid) * squeeze
    }

    fn middle(&self) -> f64 {
        match (
            self.centre.first().zip(self.half.first()),
            self.centre.last().zip(self.half.last()),
        ) {
            (Some((&a, &ha)), Some((&b, &hb))) => ((a - ha) + (b + hb)) / 2.0,
            _ => 0.0,
        }
    }

    /// Centres the run on its band's line, never before `reach`. Centring keeps runs from hugging
    /// neighbouring slots, which would raise `lane_intruders`.
    fn start(&self, run: &Run, squeeze: f64, reach: f64) -> f64 {
        (self.line(run.slot, squeeze) - (run.hi - run.lo) / 2.0).max(reach)
    }

    fn extent(&self, squeeze: f64) -> f64 {
        let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
        for row in &self.runs {
            let mut reach = f64::NEG_INFINITY;
            for run in row {
                let start = self.start(run, squeeze, reach);
                min = min.min(start);
                max = max.max(start + (run.hi - run.lo));
                reach = start + (run.hi - run.lo) + run.clear_after;
            }
        }
        if min.is_finite() {
            max - min
        } else {
            0.0
        }
    }

    /// The test of [`bands_are_already_strips`], over the runs as placed with `squeeze`.
    fn bands_are_disjoint(&self, squeeze: f64) -> bool {
        let mut span = vec![(f64::INFINITY, f64::NEG_INFINITY); self.centre.len()];
        for row in &self.runs {
            let mut reach = f64::NEG_INFINITY;
            for run in row {
                let start = self.start(run, squeeze, reach);
                if let (Some((lo, hi)), Some(s)) = (run.real, span.get_mut(run.slot)) {
                    let shift = start - run.lo;
                    s.0 = s.0.min(lo + shift);
                    s.1 = s.1.max(hi + shift);
                }
                reach = start + (run.hi - run.lo) + run.clear_after;
            }
        }
        let mut used: Vec<(f64, f64)> = span.into_iter().filter(|s| s.0 <= s.1).collect();
        used.sort_by_dyn(|a, b| a.0.total_cmp(&b.0));
        used.windows(2).all(|w| w[0].1 <= w[1].0)
    }

    fn apply(&self, st: &mut LayeredState, layers: &[Vec<usize>], squeeze: f64) {
        for (layer, row) in layers.iter().zip(&self.runs) {
            let mut reach = f64::NEG_INFINITY;
            for run in row {
                let start = self.start(run, squeeze, reach);
                let shift = start - run.lo;
                for &v in layer.get(run.from..=run.to).unwrap_or_default() {
                    st.nodes[v].cross += shift;
                }
                reach = start + (run.hi - run.lo) + run.clear_after;
            }
        }
    }
}

/// Whether each band's grouped nodes already sit in disjoint strips. Only grouped nodes count, as
/// in `metrics::lane_purity`.
fn bands_are_already_strips(st: &LayeredState) -> bool {
    let mut span = vec![(f64::INFINITY, f64::NEG_INFINITY); st.band_count];
    for (i, nd) in st.nodes.iter().enumerate().take(st.n_real) {
        if nd.group.is_none() {
            continue; // ungrouped nodes are in no strip and intrude into none
        }
        if let Some(s) = st.band_of.get(i).and_then(|&k| span.get_mut(k)) {
            s.0 = s.0.min(nd.cross - nd.w_cross / 2.0);
            s.1 = s.1.max(nd.cross + nd.w_cross / 2.0);
        }
    }
    let mut used: Vec<(f64, f64)> = span.into_iter().filter(|s| s.0 <= s.1).collect();
    used.sort_by_dyn(|a, b| a.0.total_cmp(&b.0));
    used.windows(2).all(|w| w[0].1 <= w[1].0)
}

/// Borders included.
fn drawing_middle(st: &LayeredState) -> f64 {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for nd in &st.nodes {
        lo = lo.min(nd.cross - nd.w_cross / 2.0);
        hi = hi.max(nd.cross + nd.w_cross / 2.0);
    }
    if lo.is_finite() {
        (lo + hi) / 2.0
    } else {
        0.0
    }
}

/// Borders included.
fn drawing_extent(st: &LayeredState) -> f64 {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for nd in &st.nodes {
        lo = lo.min(nd.cross - nd.w_cross / 2.0);
        hi = hi.max(nd.cross + nd.w_cross / 2.0);
    }
    if lo.is_finite() {
        hi - lo
    } else {
        0.0
    }
}

/// A maximal run of order-adjacent nodes of one band, `from..=to` in its layer.
struct Block {
    slot: usize,
    from: usize,
    to: usize,
}

fn blocks_of(layer: &[usize], band_of: &[usize]) -> Vec<Block> {
    let mut out: Vec<Block> = vec![];
    for (rank, &v) in layer.iter().enumerate() {
        let slot = band_of.get(v).copied().unwrap_or(0);
        match out.last_mut() {
            Some(last) if last.slot == slot => last.to = rank,
            _ => out.push(Block {
                slot,
                from: rank,
                to: rank,
            }),
        }
    }
    out
}

fn real_extent(st: &LayeredState, layer: &[usize], b: &Block) -> Option<(f64, f64)> {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for &v in layer.get(b.from..=b.to).unwrap_or_default() {
        let nd = &st.nodes[v];
        if nd.group.is_none() {
            continue; // a waypoint, or a node shared between bands
        }
        lo = lo.min(nd.cross - nd.w_cross / 2.0);
        hi = hi.max(nd.cross + nd.w_cross / 2.0);
    }
    if lo.is_finite() {
        Some((lo, hi))
    } else {
        None
    }
}

fn extent(st: &LayeredState, layer: &[usize], b: &Block) -> (f64, f64) {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for &v in layer.get(b.from..=b.to).unwrap_or_default() {
        let nd = &st.nodes[v];
        lo = lo.min(nd.cross - nd.w_cross / 2.0);
        hi = hi.max(nd.cross + nd.w_cross / 2.0);
    }
    if lo.is_finite() {
        (lo, hi)
    } else {
        (0.0, 0.0)
    }
}

struct Banded {
    stack: Vec<Vec<usize>>,
    cross: usize,
    slots: Vec<usize>,
    /// Group id per band, in band order.
    order: Vec<u32>,
}

struct Lanes {
    /// `None` for shared nodes and routing waypoints.
    group: Vec<Option<u32>>,
    /// Ascending and deduplicated.
    ids: Vec<u32>,
    /// Both directions, a parallel arc counted once per arc.
    adj: Vec<Vec<usize>>,
    /// Upper-layer neighbours, as [`count_crossings`] reads them.
    pred: Vec<Vec<usize>>,
    succ: Vec<Vec<usize>>,
    /// Strict lanes only: the id every real node without a group carries, so they share a lane.
    ungrouped: Option<u32>,
}

impl Lanes {
    /// `None` when no node carries a group, which must leave the drawing bit-for-bit unchanged.
    fn of(st: &LayeredState, strict: bool) -> Option<Lanes> {
        let mut group: Vec<Option<u32>> = st.nodes.iter().map(|nd| nd.group).collect();
        if !group.iter().any(Option::is_some) {
            return None;
        }
        // Swimlanes put real nodes without a group in one extra lane. Waypoints stay free.
        let mut ungrouped = None;
        if strict && group[..st.n_real].iter().any(Option::is_none) {
            let id = group
                .iter()
                .flatten()
                .max()
                .map_or(0, |&m| m.saturating_add(1));
            for g in group[..st.n_real].iter_mut().filter(|g| g.is_none()) {
                *g = Some(id);
            }
            ungrouped = Some(id);
        }
        let n = group.len();
        let (pred, succ) = st.adjacency();
        let mut adj: Vec<Vec<usize>> = vec![vec![]; n];
        for arc in &st.arcs {
            let (from, to) = (arc.from, arc.to);
            if from == to {
                continue; // self-loops leave through a border and cross nothing here
            }
            adj[from].push(to);
            adj[to].push(from);
        }
        let mut ids: Vec<u32> = group.iter().flatten().copied().collect();
        ids.sort_unstable_by_dyn(Ord::cmp);
        ids.dedup();
        Some(Lanes {
            group,
            ids,
            adj,
            pred,
            succ,
            ungrouped,
        })
    }

    /// Grouped nodes pin their id as a lane value, ungrouped ones relax to the neighbour mean, and
    /// each layer is re-sorted by it. Returns the input order if crossings rise.
    fn free_pass(&self, layers: &[Vec<usize>], o: &LayoutOptions) -> Vec<Vec<usize>> {
        let n = self.group.len();
        let mut lane = vec![f64::NAN; n];
        for (i, c) in self.group.iter().enumerate() {
            if let Some(v) = c {
                lane[i] = *v as f64;
            }
        }
        self.relax(&mut lane, o);
        // Nodes no group reaches settle at 0.5 and keep their crossing-minimised order.
        for l in lane.iter_mut() {
            if l.is_nan() {
                *l = 0.5;
            }
        }
        let mut out = layers.to_vec();
        let mut pos = positions(&out, n);
        let orig = count_crossings(&out, &self.pred, &pos);
        let saved = out.clone();
        for _ in 0..o.tuning.order.group_sort_rounds {
            for li in 0..out.len() {
                out[li].sort_unstable_by_dyn(|&a, &b| {
                    lane[a]
                        .total_cmp(&lane[b])
                        .then(
                            barycentre(&self.adj, a, &pos)
                                .total_cmp(&barycentre(&self.adj, b, &pos)),
                        )
                        .then(a.cmp(&b))
                });
                refresh(&out, &mut pos);
            }
        }
        if count_crossings(&out, &self.pred, &pos) > orig {
            saved
        } else {
            out
        }
    }

    /// `None` keeps `free`, because nothing is gained or the gain exceeds the budget. Built from
    /// `original`, since the group-id band order of `free` is arbitrary.
    fn banded(
        &self,
        original: &[Vec<usize>],
        free: &[Vec<usize>],
        o: &LayoutOptions,
        strict: bool,
    ) -> Option<Banded> {
        if self.ids.len() < 2 {
            // One band keeps nothing apart, but swimlanes still draw their one lane.
            return strict.then(|| self.keep(free, &self.ids, o, 0));
        }
        let pos = positions(free, self.group.len());
        let base = count_crossings(free, &self.pred, &pos);
        // Swimlanes are drawn whatever they cost.
        let allowance = if strict {
            usize::MAX
        } else {
            base * o.order.group_lane_budget_percent as usize / 100
                + o.tuning.order.group_lane_budget_floor
        };
        if allowance == 0 {
            return None; // the budget is off
        }
        let mut order = self.band_order(original);
        if self.inversions(free, &self.lanes_for(&order)) == 0 {
            // Already banded, so nothing to pay. The bands are still recorded for the coordinate
            // side.
            return Some(self.keep(free, &order, o, base));
        }
        let mut best = self.build(original, &order, o);
        for _ in 0..o.tuning.order.group_lane_order_rounds {
            let mut improved = false;
            for k in 0..order.len().saturating_sub(1) {
                let mut trial = order.clone();
                trial.swap(k, k + 1);
                let candidate = self.build(original, &trial, o);
                // Strictly fewer only, so ties keep the flow order deterministically.
                if candidate.cross < best.cross {
                    best = candidate;
                    order = trial;
                    improved = true;
                }
            }
            if !improved {
                break;
            }
        }
        // The search may have moved to another band order, so check it again.
        if self.inversions(free, &self.lanes_for(&order)) == 0 {
            return Some(self.keep(free, &order, o, base));
        }
        if best.cross > base.saturating_add(allowance) {
            return None;
        }
        Some(best)
    }

    /// Sorts keyed members of a band by [`crate::Node::lane_key`] among the positions they hold,
    /// so unkeyed members and waypoints stay put.
    fn sort_by_lane_key(&self, st: &LayeredState, banded: &mut Banded) {
        if !st.nodes.iter().any(|nd| nd.lane_key.is_some()) {
            return;
        }
        for layer in banded.stack.iter_mut() {
            for slot in 0..self.ids.len() {
                let at: Vec<usize> = (0..layer.len())
                    .filter(|&k| {
                        let v = layer[k];
                        self.group[v].is_some()
                            && banded.slots.get(v) == Some(&slot)
                            && st.nodes[v].lane_key.is_some()
                    })
                    .collect();
                if at.len() < 2 {
                    continue;
                }
                let mut members: Vec<usize> = at.iter().map(|&k| layer[k]).collect();
                members.sort_unstable_by_dyn(|&a, &b| {
                    st.nodes[a]
                        .lane_key
                        .cmp(&st.nodes[b].lane_key)
                        .then(a.cmp(&b))
                });
                for (&k, v) in at.iter().zip(members) {
                    layer[k] = v;
                }
            }
        }
    }

    /// Groups sorted by their mean position in the given order. Positions are normalised by layer
    /// width, so the top of a 40-node layer and of a 3-node layer both count as the top.
    fn band_order(&self, layers: &[Vec<usize>]) -> Vec<u32> {
        let mut sum = vec![0.0f64; self.ids.len()];
        let mut count = vec![0.0f64; self.ids.len()];
        for layer in layers {
            let span = layer.len().saturating_sub(1) as f64;
            for (rank, &v) in layer.iter().enumerate() {
                if let Some(k) = self.slot_of(v) {
                    sum[k] += if span > 0.0 {
                        rank as f64 / span
                    } else {
                        0.5 // alone in its layer, neither top nor bottom
                    };
                    count[k] += 1.0;
                }
            }
        }
        let mean = |k: usize| {
            if count[k] > 0.0 {
                sum[k] / count[k]
            } else {
                0.5
            }
        };
        let mut order: Vec<u32> = self.ids.clone();
        order.sort_by_dyn(|a, b| {
            let (ka, kb) = (self.ids.binary_search(a), self.ids.binary_search(b));
            match (ka, kb) {
                (Ok(ka), Ok(kb)) => mean(ka).total_cmp(&mean(kb)).then(a.cmp(b)),
                _ => a.cmp(b),
            }
        });
        order
    }

    /// `None` for an ungrouped node.
    fn slot_of(&self, node: usize) -> Option<usize> {
        self.group
            .get(node)
            .copied()
            .flatten()
            .and_then(|id| self.ids.binary_search(&id).ok())
    }

    /// `None` for a node without a group.
    fn lanes_for(&self, order: &[u32]) -> Vec<Option<usize>> {
        let mut band = vec![0usize; self.ids.len()];
        for (lane, id) in order.iter().enumerate() {
            if let Ok(k) = self.ids.binary_search(id) {
                band[k] = lane;
            }
        }
        (0..self.group.len())
            .map(|i| self.slot_of(i).map(|k| band[k]))
            .collect()
    }

    /// The current order with its bands recorded, for an order already in bands.
    fn keep(
        &self,
        layers: &[Vec<usize>],
        order: &[u32],
        o: &LayoutOptions,
        cross: usize,
    ) -> Banded {
        let lane = self.lanes_for(order);
        let value = self.values(layers, &lane, o);
        Banded {
            stack: layers.to_vec(),
            cross,
            slots: self.slots(&lane, &value),
            order: order.to_vec(),
        }
    }

    /// An ungrouped node joins the band nearest its relaxed value, which is where the order put it.
    fn slots(&self, lane: &[Option<usize>], value: &[f64]) -> Vec<usize> {
        let top = self.ids.len().saturating_sub(1);
        (0..self.group.len())
            .map(|i| match lane.get(i).copied().flatten() {
                Some(k) => k,
                None => (value[i].round().max(0.0) as usize).min(top),
            })
            .collect()
    }

    /// Bands every layer, sweeps inside the bands, then repairs.
    fn build(&self, layers: &[Vec<usize>], order: &[u32], o: &LayoutOptions) -> Banded {
        let lane = self.lanes_for(order);
        let value = self.values(layers, &lane, o);
        let mut stack = layers.to_vec();
        let mut pos = positions(&stack, self.group.len());
        // Ties inside a band keep the current position, so the crossing minimiser's work survives.
        for layer in stack.iter_mut() {
            layer.sort_unstable_by_dyn(|&a, &b| {
                value[a]
                    .total_cmp(&value[b])
                    .then(pos[a].cmp(&pos[b]))
                    .then(a.cmp(&b))
            });
        }
        refresh(&stack, &mut pos);
        self.sweep(&mut stack, &value, &mut pos, o);
        self.repair(&mut stack, &lane, &mut pos, o);
        let cross = count_crossings(&stack, &self.pred, &pos);
        let slots = self.slots(&lane, &value);
        Banded {
            stack,
            cross,
            slots,
            order: order.to_vec(),
        }
    }

    /// Band index, or for an ungrouped node the mean of its neighbours' values, which places it
    /// between the bands it is wired into.
    fn values(&self, layers: &[Vec<usize>], lane: &[Option<usize>], o: &LayoutOptions) -> Vec<f64> {
        let mut value = vec![f64::NAN; self.group.len()];
        for (i, l) in lane.iter().enumerate() {
            if let Some(k) = l {
                value[i] = *k as f64;
            }
        }
        self.relax(&mut value, o);
        // An unreached node takes the band its layer position falls in, so it stays where the
        // crossing minimiser put it instead of being dragged between two unrelated bands.
        let top = self.ids.len().saturating_sub(1) as f64;
        for layer in layers {
            let span = layer.len().saturating_sub(1) as f64;
            for (rank, &v) in layer.iter().enumerate() {
                if value[v].is_nan() {
                    value[v] = if span > 0.0 {
                        rank as f64 / span * top
                    } else {
                        top / 2.0
                    };
                }
            }
        }
        value
    }

    /// Ungrouped nodes take the mean of their neighbours until nothing moves or the budget runs
    /// out. Grouped nodes are anchors, so values stay inside the band range.
    fn relax(&self, value: &mut [f64], o: &LayoutOptions) {
        for _ in 0..o.tuning.order.group_relax_iters {
            let mut changed = false;
            for i in 0..value.len() {
                if self.group.get(i).copied().flatten().is_some() {
                    continue;
                }
                let (mut sum, mut cnt) = (0.0f64, 0.0f64);
                for &j in &self.adj[i] {
                    if !value[j].is_nan() {
                        sum += value[j];
                        cnt += 1.0;
                    }
                }
                if cnt > 0.0 {
                    let m = sum / cnt;
                    if value[i].is_nan() || (value[i] - m).abs() > EPS {
                        value[i] = m;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }

    /// Constrained barycentre sweeps, alternating direction per round. The band value is the first
    /// sort key, so a node only moves inside its band.
    fn sweep(&self, stack: &mut [Vec<usize>], value: &[f64], pos: &mut [usize], o: &LayoutOptions) {
        let depth = stack.len();
        for round in 0..o.tuning.order.group_lane_sweeps {
            let forward = round % 2 == 0;
            for step in 0..depth {
                let li = if forward { step } else { depth - 1 - step };
                // The first layer of a sweep has nothing behind it.
                if (forward && li == 0) || (!forward && li + 1 == depth) {
                    continue;
                }
                let nb = if forward { &self.pred } else { &self.succ };
                let mut keyed: Vec<(f64, f64, usize)> = stack[li]
                    .iter()
                    .map(|&x| (value[x], barycentre(nb, x, pos), x))
                    .collect();
                keyed.sort_unstable_by_dyn(|a, b| {
                    a.0.total_cmp(&b.0)
                        .then(a.1.total_cmp(&b.1))
                        .then(a.2.cmp(&b.2))
                });
                for (slot, &(_, _, x)) in keyed.iter().enumerate() {
                    stack[li][slot] = x;
                }
                refresh(stack, pos);
            }
        }
    }

    /// Adjacent swaps that lower crossings, refusing any that puts two grouped nodes out of band
    /// order. Ungrouped nodes may pass anything.
    fn repair(
        &self,
        stack: &mut [Vec<usize>],
        lane: &[Option<usize>],
        pos: &mut [usize],
        o: &LayoutOptions,
    ) {
        for _ in 0..o.tuning.order.group_lane_repair_rounds {
            let mut moved = false;
            for layer in stack.iter_mut() {
                for k in 0..layer.len().saturating_sub(1) {
                    let (u, v) = (layer[k], layer[k + 1]);
                    if let (Some(a), Some(b)) = (lane[u], lane[v]) {
                        if a != b {
                            continue;
                        }
                    }
                    let delta = adjacent_swap_delta(u, v, &self.pred, pos)
                        + adjacent_swap_delta(u, v, &self.succ, pos);
                    if delta < 0 {
                        layer.swap(k, k + 1);
                        pos[u] = k + 1;
                        pos[v] = k;
                        moved = true;
                    }
                }
            }
            if !moved {
                break;
            }
        }
    }

    /// Grouped node pairs in the wrong band order, summed over layers, `0` exactly when banded.
    /// Quadratic per layer, affordable since it runs twice and only on grouped drawings.
    fn inversions(&self, layers: &[Vec<usize>], lane: &[Option<usize>]) -> usize {
        let mut total = 0;
        let mut seen: Vec<usize> = vec![];
        for layer in layers {
            seen.clear();
            for &v in layer {
                if let Some(k) = lane[v] {
                    total += seen.iter().filter(|&&earlier| earlier > k).count();
                    seen.push(k);
                }
            }
        }
        total
    }
}

fn positions(layers: &[Vec<usize>], n: usize) -> Vec<usize> {
    let mut pos = vec![0usize; n];
    refresh(layers, &mut pos);
    pos
}

fn refresh(layers: &[Vec<usize>], pos: &mut [usize]) {
    for layer in layers {
        for (rank, &v) in layer.iter().enumerate() {
            if let Some(slot) = pos.get_mut(v) {
                *slot = rank;
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::geometry::shape::NodeShape;
    use crate::layered::prepare::{LayeredInput, LayeredInputNode};
    use crate::options::LayoutOptions;

    /// Lanes are off by default.
    fn with_lanes() -> LayoutOptions {
        let mut o = LayoutOptions::layered_default();
        o.order.group_lane_budget_percent = 100;
        o.tuning.order.group_lane_budget_floor = 8;
        o.order.group_lane_area_percent = 50;
        o
    }

    fn node(category: Option<u32>) -> LayeredInputNode {
        LayeredInputNode {
            width: 120.0,
            height: 52.0,
            shape: NodeShape::Box,
            constraint: None,
            category,
            clear_after: 0.0,
            clear_before: 0.0,
            port_sides: None,
            lane_key: None,
            along_before: 0.0,
            along_after: 0.0,
        }
    }

    /// One layer per rank in the given order. All nodes are real, since the pass treats a waypoint
    /// as an ungrouped node anyway.
    fn state(
        groups: &[Option<u32>],
        ranks: &[i32],
        edges: &[(usize, usize)],
        layers: &[Vec<usize>],
    ) -> LayeredState {
        let input = LayeredInput {
            nodes: groups.iter().map(|&g| node(g)).collect(),
            edges: edges.to_vec(),
            weights: vec![],
            thickness: vec![],
            edge_label_sizes: vec![],
            seed: vec![],
            pinned: vec![],
            self_loops: vec![],
            ranked_backwards: Vec::new(),
            options: with_lanes(),
        };
        let mut st = LayeredState::new(&input);
        for (nd, &r) in st.nodes.iter_mut().zip(ranks) {
            nd.rank = r;
        }
        st.rebuild_layers();
        st.layers = layers.to_vec();
        st.reindex();
        st
    }

    fn crossings(st: &LayeredState) -> usize {
        count_crossings(&st.layers, &st.adjacency().0, &st.order_of)
    }

    /// Group 2 leads because the crossing minimiser left it first.
    #[test]
    fn bands_that_cost_nothing_are_taken_in_flow_order() {
        let o = with_lanes();
        let groups = [
            Some(2),
            Some(0),
            Some(1),
            Some(2),
            Some(1),
            Some(0),
            Some(2),
            Some(0),
            Some(1),
        ];
        let mut st = state(
            &groups,
            &[0, 0, 0, 1, 1, 1, 2, 2, 2],
            &[(0, 3), (1, 5), (2, 4), (3, 6), (5, 7), (4, 8)],
            // Layer 1 has groups 1 and 0 the wrong way round for the bands of layers 0 and 2.
            &[vec![0, 1, 2], vec![3, 4, 5], vec![6, 7, 8]],
        );
        assert_eq!(crossings(&st), 2, "the order handed in still crosses");
        enforce_groups(&mut st, &o).unwrap();
        assert_eq!(crossings(&st), 0, "the banding cost nothing - it paid");
        // Bands follow the flow order.
        let bands: Vec<Vec<Option<u32>>> = st
            .layers
            .iter()
            .map(|l| l.iter().map(|&v| groups[v]).collect())
            .collect();
        for layer in &bands {
            assert_eq!(layer, &vec![Some(2), Some(0), Some(1)], "bands {bands:?}");
        }
    }

    /// Banding the middle layer must cross, so the budget decides.
    #[test]
    fn a_banding_that_costs_crossings_is_the_budget_s_decision() {
        let groups = [Some(0), Some(1), Some(1), Some(0)];
        let build = || {
            state(
                &groups,
                &[0, 0, 1, 1],
                &[(0, 2), (1, 3)],
                &[vec![0, 1], vec![2, 3]],
            )
        };

        // The crossing-neutral pass cannot band this without a crossing, so it restores the order.
        let free = vec![vec![0, 1], vec![2, 3]];

        // Percent and floor both zero turn the budget off.
        let mut off = LayoutOptions::layered_default();
        off.order.group_lane_budget_percent = 0;
        off.tuning.order.group_lane_budget_floor = 0;
        let mut st = build();
        enforce_groups(&mut st, &off).unwrap();
        assert_eq!(st.layers, free);
        assert_eq!(crossings(&st), 0, "the free pass pays nothing");

        // One crossing fits a floor of one.
        let mut on = LayoutOptions::layered_default();
        on.order.group_lane_budget_percent = 0;
        on.tuning.order.group_lane_budget_floor = 1;
        let mut st = build();
        enforce_groups(&mut st, &on).unwrap();
        assert_eq!(st.layers, vec![vec![0, 1], vec![3, 2]]);
        assert_eq!(crossings(&st), 1, "the band cost exactly the crossing");

        // A budget one crossing short refuses it again.
        let mut short = on;
        short.tuning.order.group_lane_budget_floor = 0;
        let mut st = build();
        enforce_groups(&mut st, &short).unwrap();
        assert_eq!(st.layers, free);
    }

    /// Ungrouped drawings must stay byte-identical, so no part of the pass may touch them.
    #[test]
    fn a_graph_with_no_group_is_left_exactly_alone() {
        let o = with_lanes();
        let mut st = state(
            &[None, None, None, None, None, None],
            &[0, 0, 0, 1, 1, 1],
            &[(0, 5), (1, 4), (2, 3)],
            &[vec![0, 1, 2], vec![3, 4, 5]],
        );
        let before = st.layers.clone();
        enforce_groups(&mut st, &o).unwrap();
        assert_eq!(st.layers, before);
    }

    /// A shared node wired to the first and last band lands between them, not inside a band.
    #[test]
    fn an_ungrouped_node_sits_between_the_bands_it_touches() {
        let o = with_lanes();
        let groups = [Some(0), Some(1), Some(2), Some(0), None, Some(2)];
        let mut st = state(
            &groups,
            &[0, 0, 0, 1, 1, 1],
            // Node 4 is wired to all three groups. The flow puts group 2 first, so the band order
            // is not the group-id order of the free pass.
            &[(0, 3), (0, 4), (2, 4), (2, 5), (1, 4)],
            &[vec![2, 1, 0], vec![5, 4, 3]],
        );
        enforce_groups(&mut st, &o).unwrap();
        let second: Vec<Option<u32>> = st.layers[1].iter().map(|&v| groups[v]).collect();
        assert_eq!(second, vec![Some(2), None, Some(0)]);
    }

    #[test]
    fn the_pass_is_deterministic() {
        let o = with_lanes();
        let groups = [
            Some(1),
            Some(0),
            Some(1),
            Some(2),
            Some(0),
            None,
            Some(2),
            Some(1),
            Some(0),
            Some(2),
        ];
        let ranks = [0, 0, 0, 0, 1, 1, 1, 2, 2, 2];
        let edges = [
            (0, 4),
            (1, 5),
            (2, 6),
            (3, 6),
            (4, 8),
            (5, 7),
            (6, 9),
            (0, 5),
            (3, 4),
        ];
        let layers = [vec![0, 1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
        let mut first = state(&groups, &ranks, &edges, &layers);
        enforce_groups(&mut first, &o).unwrap();
        for _ in 0..4 {
            let mut again = state(&groups, &ranks, &edges, &layers);
            enforce_groups(&mut again, &o).unwrap();
            assert_eq!(again.layers, first.layers);
        }
        // No grouped node precedes a node of an earlier band.
        let lanes = Lanes::of(&first, false).unwrap();
        let order = lanes.band_order(&first.layers);
        assert_eq!(lanes.inversions(&first.layers, &lanes.lanes_for(&order)), 0);
    }

    /// Spreads each layer `step` apart and shifts layer `li` by `offset(li)`, keeping the order but
    /// drifting between layers as real coordinate phases do.
    fn spread(st: &mut LayeredState, step: f64, offset: impl Fn(usize) -> f64) {
        let layers = st.layers.clone();
        for (li, layer) in layers.iter().enumerate() {
            for (rank, &v) in layer.iter().enumerate() {
                st.nodes[v].cross = rank as f64 * step + offset(li);
            }
        }
    }

    /// Per band, in band order, over grouped nodes.
    fn band_spans(st: &LayeredState) -> Vec<(f64, f64)> {
        let mut span = vec![(f64::INFINITY, f64::NEG_INFINITY); st.band_count];
        for (i, nd) in st.nodes.iter().enumerate().take(st.n_real) {
            if nd.group.is_some() {
                let s = &mut span[st.band_of[i]];
                s.0 = s.0.min(nd.cross - nd.w_cross / 2.0);
                s.1 = s.1.max(nd.cross + nd.w_cross / 2.0);
            }
        }
        span
    }

    fn cross_extent(st: &LayeredState) -> f64 {
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for nd in &st.nodes {
            lo = lo.min(nd.cross - nd.w_cross / 2.0);
            hi = hi.max(nd.cross + nd.w_cross / 2.0);
        }
        hi - lo
    }

    /// A banded order can still be drawn with bands on top of each other, since each layer is
    /// placed on its own.
    #[test]
    fn align_lanes_pulls_each_band_onto_a_strip_of_its_own() {
        let o = with_lanes();
        let groups = [Some(0), Some(1), Some(0), Some(1)];
        let mut st = state(
            &groups,
            &[0, 0, 1, 1],
            &[(0, 2), (1, 3)],
            &[vec![0, 1], vec![2, 3]],
        );
        enforce_groups(&mut st, &o).unwrap();
        assert_eq!(st.band_count, 2);
        // The second layer sits far down the axis, so each band's span covers the other's.
        spread(&mut st, 100.0, |li| li as f64 * 1000.0);
        let before = band_spans(&st);
        assert!(
            before[0].1 > before[1].0,
            "the bands start on top of each other"
        );

        align_lanes(&mut st, &o).unwrap();
        let after = band_spans(&st);
        assert!(
            after[0].1 <= after[1].0,
            "the bands must not overlap any more: {after:?}"
        );
        // No node was moved onto another.
        for layer in st.layers.clone() {
            for pair in layer.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                assert!(
                    st.nodes[b].cross - st.nodes[a].cross >= st.sep(a, b, &o) - 1e-9,
                    "nodes {a} and {b} ended up closer than their separation"
                );
            }
        }
    }

    #[test]
    fn align_lanes_leaves_a_drawing_with_no_bands_alone() {
        let o = with_lanes();
        let mut st = state(
            &[None, None, None, None],
            &[0, 0, 1, 1],
            &[(0, 2), (1, 3)],
            &[vec![0, 1], vec![2, 3]],
        );
        enforce_groups(&mut st, &o).unwrap();
        assert!(st.band_of.is_empty() && st.band_count == 0);
        spread(&mut st, 100.0, |li| li as f64 * 1000.0);
        let before: Vec<f64> = st.nodes.iter().map(|nd| nd.cross).collect();
        align_lanes(&mut st, &o).unwrap();
        let after: Vec<f64> = st.nodes.iter().map(|nd| nd.cross).collect();
        assert_eq!(before, after);
    }

    /// The two bands never share a layer, so stacking them doubles the width. Without allowance
    /// they stay overlapped, with a generous one they separate.
    #[test]
    fn align_lanes_separates_only_as_far_as_the_area_budget_allows() {
        let groups = [
            Some(0),
            Some(0),
            Some(0),
            Some(0),
            Some(1),
            Some(1),
            Some(1),
            Some(1),
        ];
        let build = |percent: u32| {
            let mut o = with_lanes();
            o.order.group_lane_area_percent = percent;
            let mut st = state(
                &groups,
                &[0, 0, 0, 0, 1, 1, 1, 1],
                &[(0, 4), (1, 5), (2, 6), (3, 7)],
                &[vec![0, 1, 2, 3], vec![4, 5, 6, 7]],
            );
            enforce_groups(&mut st, &o).unwrap();
            spread(&mut st, 100.0, |_| 0.0);
            (st, o)
        };

        let (mut tight, o) = build(0);
        let room = cross_extent(&tight);
        align_lanes(&mut tight, &o).unwrap();
        assert!(
            cross_extent(&tight) <= room + 1e-9,
            "nothing may be spent when nothing is allowed"
        );
        let spans = band_spans(&tight);
        assert!(spans[0].1 > spans[1].0, "so the bands still overlap");

        let (mut roomy, o) = build(200);
        align_lanes(&mut roomy, &o).unwrap();
        let spans = band_spans(&roomy);
        assert!(
            spans[0].1 <= spans[1].0,
            "with the room, they separate: {spans:?}"
        );
        assert!(cross_extent(&roomy) <= room * 3.0, "and only just");
    }

    #[test]
    fn align_lanes_is_deterministic() {
        let o = with_lanes();
        let groups = [Some(2), Some(0), Some(1), None, Some(0), Some(2), Some(1)];
        let make = || {
            let mut st = state(
                &groups,
                &[0, 0, 0, 1, 1, 1, 1],
                &[(0, 5), (1, 4), (2, 6), (0, 3), (2, 3)],
                &[vec![0, 1, 2], vec![3, 4, 5, 6]],
            );
            enforce_groups(&mut st, &o).unwrap();
            spread(&mut st, 110.0, |li| li as f64 * 37.0);
            st
        };
        let mut first = make();
        align_lanes(&mut first, &o).unwrap();
        let want: Vec<f64> = first.nodes.iter().map(|nd| nd.cross).collect();
        for _ in 0..4 {
            let mut again = make();
            align_lanes(&mut again, &o).unwrap();
            let got: Vec<f64> = again.nodes.iter().map(|nd| nd.cross).collect();
            assert_eq!(got, want);
        }
    }
}

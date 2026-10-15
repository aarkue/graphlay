//! Draws the heaviest flow through the model as one straight line, since a process map reads best
//! with its main path central and straight (Mennens et al.).
//!
//! [`align_main_path`] pins the path ([`spine`]) to one cross coordinate and re-projects each layer
//! with [`project_gaps_pinned`]. A pin stays only while the drawing's extent stays within
//! `CoordTuning::main_path_room` percent of what it was and no node is pushed into another group's
//! band.

use crate::error::LayoutError;
use crate::geometry::monotone::project_gaps_pinned;
use crate::geometry::{EPS, SLACK};
use crate::options::LayoutOptions;

use super::coord::gaps_hold;
use super::state::LayeredState;
use crate::sort::SortDyn;

/// Pins the main path onto one straight line where the drawing can afford it.
pub fn align_main_path(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    // Mirrored branches want the fork-join line between them, and a line through strict
    // swimlanes would pull nodes out of their lanes.
    if o.tuning.coords.main_path_room == 0
        || st.has_seeds()
        || o.coords.branch_style == crate::options::BranchStyle::Symmetric
        || o.order.group_lanes == crate::options::GroupLanes::Strict
    {
        return Ok(());
    }
    if !has_a_flow(st) {
        return Ok(());
    }
    let spine = spine(st);
    if spine.len() < 3 {
        return Ok(());
    }
    st.reindex();
    let layers = std::mem::take(&mut st.layers);
    let cross: Vec<f64> = st.nodes.iter().map(|n| n.cross).collect();

    // The median height keeps the move small.
    let mut heights: Vec<f64> = spine.iter().map(|&v| cross[v]).collect();
    heights.sort_unstable_by_dyn(f64::total_cmp);
    let line = heights[heights.len() / 2];

    // The budget is on the whole drawing's extent, so a narrow layer can spread for free.
    let widest = spine
        .iter()
        .filter(|&&v| v < st.n_real)
        .map(|&v| st.nodes[v].w_cross)
        .fold(0.0, f64::max);
    // At least one node's width, because on a narrow drawing a percentage is less than a column.
    let room = (extent(&cross) * f64::from(o.tuning.coords.main_path_room) / 100.0).max(widest);
    let budget = extent(&cross) + room + SLACK;
    let on_spine: std::collections::BTreeSet<usize> = spine.iter().copied().collect();
    let mut chains_of: Vec<Vec<usize>> = vec![Vec::new(); st.len()];
    for (&arc, chain) in &st.chains {
        for &v in chain {
            chains_of[v].push(arc);
        }
    }
    // A layer that cannot hold its pin keeps its node where it was.
    let pin = |from: &[f64], nodes: &[usize], line: f64| -> Vec<f64> {
        let mut out = from.to_vec();
        for &v in nodes {
            let Some(layer) = layers.get(st.nodes[v].layer_clamped()) else {
                continue;
            };
            if layer.len() < 2 {
                out[v] = line;
                continue;
            }
            let desired: Vec<f64> = layer.iter().map(|&u| out[u]).collect();
            let gap: Vec<f64> = (0..layer.len() - 1)
                .map(|i| st.sep(layer[i], layer[i + 1], o))
                .collect();
            let fixed: Vec<Option<f64>> = layer.iter().map(|&u| (u == v).then_some(line)).collect();
            let placed = project_gaps_pinned(&desired, &gap, &fixed);
            if gaps_hold(&placed, &gap) {
                for (i, &u) in layer.iter().enumerate() {
                    out[u] = placed[i];
                }
            }
        }
        out
    };
    // A pin pays when the drawing stays in budget and the spine-weighted length does not grow.
    let banded = st.band_count >= 2;
    let pays = |before: &[f64], after: &[f64]| {
        extent(after) <= budget
            && length_cost(st, &chains_of, &on_spine, after, before)
                <= length_cost(st, &chains_of, &on_spine, before, after)
            && (!banded || intruders(st, after) <= intruders(st, before))
    };

    // Try the whole line first. A single pin straightens nothing until its neighbour is on the
    // line too, so pricing pin by pin is myopic. Fall back to node by node only if no line pays.
    let mut placed_cross = cross.clone();
    // Candidate lines are the median and the heights where runs of the path already stand,
    // because the median alone can blow the budget. The cheapest line that pays wins.
    let mut runs: Vec<(usize, f64)> = Vec::new();
    for &h in &heights {
        match runs.last_mut() {
            Some((n, at)) if (h - *at).abs() < 0.5 => *n += 1,
            _ => runs.push((1, h)),
        }
    }
    runs.sort_by_dyn(|a, b| b.0.cmp(&a.0).then(a.1.total_cmp(&b.1)));
    let mut lines = vec![line];
    lines.extend(
        runs.iter()
            .map(|&(_, h)| h)
            .filter(|h| (h - line).abs() >= 0.5)
            .take(LINE_CANDIDATES),
    );
    let added = |after: &[f64]| {
        length_cost(st, &chains_of, &on_spine, after, &cross)
            - length_cost(st, &chains_of, &on_spine, &cross, after)
    };
    let whole = lines
        .iter()
        .map(|&l| pin(&cross, &spine, l))
        .filter(|w| pays(&cross, w))
        .min_by(|a, b| added(a).total_cmp(&added(b)));
    if let Some(whole) = whole {
        placed_cross = whole;
    } else {
        for &v in &spine {
            if (placed_cross[v] - line).abs() < 0.5 {
                continue;
            }
            let trial = pin(&placed_cross, &[v], line);
            if pays(&placed_cross, &trial) {
                placed_cross = trial;
            }
        }
    }
    for (node, &c) in st.nodes.iter_mut().zip(&placed_cross) {
        node.cross = c;
    }
    st.layers = layers;
    Ok(())
}

/// Consecutive node pairs of the main path, for the placement to weigh. Empty when the main path
/// should not be favoured.
pub(super) fn spine_hops(
    st: &LayeredState,
    o: &LayoutOptions,
) -> std::collections::BTreeSet<(usize, usize)> {
    if o.tuning.coords.main_path_room == 0
        || st.has_seeds()
        || o.coords.branch_style == crate::options::BranchStyle::Symmetric
        || !has_a_flow(st)
        || several_flows(st)
        || weighted(st)
    {
        return Default::default();
    }
    spine(st).windows(2).map(|w| (w[0], w[1])).collect()
}

/// Pixels of other edges' length that one pixel of bend removed from the spine is worth. Matches
/// the network-simplex weight on a long edge's inner segment.
const SPINE_WEIGHT: f64 = 8.0;

/// Above this share of backward arcs a drawing has no main flow (see [`has_a_flow`]).
const MAX_BACKWARD_SHARE: f64 = 0.25;

/// How many lines besides the median [`align_main_path`] tries. Each costs one projection per
/// spine layer.
const LINE_CANDIDATES: usize = 4;

/// Weighted cross-axis length of `cross` over every chain touching a node that differs from
/// `other`. Each hop costs its cross distance times the edge weight, times [`SPINE_WEIGHT`] when
/// both ends are on the main path.
fn length_cost(
    st: &LayeredState,
    chains_of: &[Vec<usize>],
    on_spine: &std::collections::BTreeSet<usize>,
    cross: &[f64],
    other: &[f64],
) -> f64 {
    let mut touched: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    for v in 0..cross.len() {
        if (cross[v] - other[v]).abs() > EPS {
            touched.extend(chains_of[v].iter().copied());
        }
    }
    let mut total = 0.0;
    for arc in touched {
        let Some(chain) = st.chains.get(&arc) else {
            continue;
        };
        let w = st.weight.get(arc).copied().unwrap_or(1.0);
        for hop in chain.windows(2) {
            let spine_hop = on_spine.contains(&hop[0]) && on_spine.contains(&hop[1]);
            let factor = if spine_hop { SPINE_WEIGHT } else { 1.0 };
            total += (cross[hop[1]] - cross[hop[0]]).abs() * w * factor;
        }
    }
    total
}

/// Whether the arcs carry different weights. The weights then already favour the main path, and
/// adding spine weight in placement pulls busy nodes off the line that [`align_main_path`] draws.
pub(super) fn weighted(st: &LayeredState) -> bool {
    let mut w = st.weight.iter().copied().filter(|w| w.is_finite());
    let Some(first) = w.next() else {
        return false;
    };
    w.any(|x| (x - first).abs() > EPS * first.abs().max(1.0))
}

/// Whether the nodes carry two or more groups. Each group is its own flow, so a single longest
/// path through them is arbitrary.
pub(super) fn several_flows(st: &LayeredState) -> bool {
    let mut first = None;
    for nd in &st.nodes[..st.n_real] {
        if let Some(g) = nd.group {
            match first {
                None => first = Some(g),
                Some(f) if f != g => return true,
                _ => {}
            }
        }
    }
    false
}

/// Whether the drawing has a direction of flow for a main path to follow, i.e., at most
/// [`MAX_BACKWARD_SHARE`] of the arcs run backwards through the layers.
pub(super) fn has_a_flow(st: &LayeredState) -> bool {
    let edges = st.chains.values().filter(|c| c.first() != c.last()).count();
    if edges == 0 {
        return false;
    }
    let turned_by_the_cycle_breaker: std::collections::BTreeSet<usize> = st
        .arcs
        .iter()
        .filter(|a| a.reversed)
        .map(|a| a.edge)
        .collect();
    // Caller-requested backward arcs are counted, but cycle-breaker reversals are weighed,
    // because a process map's rework loops are its lightest arcs.
    let weight = |e: usize| st.weight.get(e).copied().unwrap_or(1.0);
    let total: f64 = st
        .chains
        .iter()
        .filter(|(_, c)| c.first() != c.last())
        .map(|(&e, _)| weight(e))
        .sum();
    let turned: f64 = turned_by_the_cycle_breaker.iter().map(|&e| weight(e)).sum();
    let share = st.ranked_backwards.iter().filter(|&&r| r).count() as f64 / edges as f64
        + if total > 0.0 { turned / total } else { 0.0 };
    share <= MAX_BACKWARD_SHARE
}

/// How many grouped real nodes sit inside another band's span in `cross`, as in
/// `LayoutMetrics::lane_intruders`.
fn intruders(st: &LayeredState, cross: &[f64]) -> usize {
    let mut span = vec![(f64::INFINITY, f64::NEG_INFINITY); st.band_count];
    let grouped = |v: usize| st.nodes[v].group.is_some() && v < st.band_of.len();
    for v in (0..st.n_real).filter(|&v| grouped(v)) {
        if let Some(sp) = span.get_mut(st.band_of[v]) {
            let h = st.nodes[v].w_cross / 2.0;
            sp.0 = sp.0.min(cross[v] - h);
            sp.1 = sp.1.max(cross[v] + h);
        }
    }
    (0..st.n_real)
        .filter(|&v| grouped(v))
        .filter(|&v| {
            span.iter()
                .enumerate()
                .any(|(k, sp)| k != st.band_of[v] && cross[v] > sp.0 && cross[v] < sp.1)
        })
        .count()
}

/// Lexicographic `(bottleneck, total)` order.
fn order_of(a: (f64, f64), b: (f64, f64)) -> std::cmp::Ordering {
    a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1))
}

/// Whether `a` beats `b` by more than rounding.
fn better(a: (f64, f64), b: (f64, f64)) -> bool {
    a.0 > b.0 + EPS || ((a.0 - b.0).abs() <= EPS && a.1 > b.1 + EPS)
}

/// The cross-axis extent of a placement.
fn extent(cross: &[f64]) -> f64 {
    let (lo, hi) = cross
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), &c| {
            (l.min(c), h.max(c))
        });
    if hi >= lo {
        hi - lo
    } else {
        0.0
    }
}

/// Every node on the main path in rank order, including its waypoints. The path is the widest
/// source-to-sink path (heaviest lightest arc), then the heaviest in total, since a max-sum path
/// prefers a detour of two light arcs over the heavy arc that skips it. Ties go to the lower node
/// index.
pub(super) fn spine(st: &LayeredState) -> Vec<usize> {
    let n = st.n_real;
    // (target, weight, arc) per real source.
    let mut out: Vec<Vec<(usize, f64, usize)>> = vec![Vec::new(); n];
    for (&arc, chain) in &st.chains {
        let (Some(&first), Some(&last)) = (chain.first(), chain.last()) else {
            continue;
        };
        // Follow the layering, so a reversed arc runs from its lower-ranked end.
        let (a, b) = if st.nodes[last].rank >= st.nodes[first].rank {
            (first, last)
        } else {
            (last, first)
        };
        if a == b || a >= n || b >= n || st.nodes[b].rank == st.nodes[a].rank {
            continue;
        }
        let w = st.weight.get(arc).copied().unwrap_or(1.0);
        out[a].push((b, w, arc));
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key_dyn(|&v| (st.nodes[v].rank, v));
    let mut indegree = vec![0usize; n];
    for arcs in &out {
        for &(b, _, _) in arcs {
            indegree[b] += 1;
        }
    }
    // Only sources start a path, so a lone heavy arc mid-model is not one.
    let unreached = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    let mut best: Vec<(f64, f64)> = (0..n)
        .map(|v| {
            if indegree[v] == 0 {
                (f64::INFINITY, 0.0)
            } else {
                unreached
            }
        })
        .collect();
    let mut pred: Vec<Option<(usize, usize)>> = vec![None; n];
    for &v in &order {
        if best[v] == unreached {
            continue;
        }
        for &(b, w, arc) in &out[v] {
            let candidate = (best[v].0.min(w), best[v].1 + w);
            if better(candidate, best[b]) {
                best[b] = candidate;
                pred[b] = Some((v, arc));
            }
        }
    }
    // Prefer a true sink in the caller's direction. A node whose only way on is a reversed loop
    // looks like a sink to the layering, and ending there would leave the rest off the line.
    let mut leaves = vec![0usize; n];
    let mut turned = vec![false; st.routes.len().max(1)];
    for arc in &st.arcs {
        if arc.reversed {
            if let Some(t) = turned.get_mut(arc.edge) {
                *t = true;
            }
        }
    }
    for (&edge, chain) in &st.chains {
        let (Some(&first), Some(&last)) = (chain.first(), chain.last()) else {
            continue;
        };
        if first == last {
            continue;
        }
        let source = if turned.get(edge).copied().unwrap_or(false) {
            last
        } else {
            first
        };
        if source < n {
            leaves[source] += 1;
        }
    }
    let reached = |v: &usize| pred[*v].is_some();
    let best_of = |nodes: &mut dyn Iterator<Item = usize>| {
        nodes.max_by(|&a, &b| order_of(best[a], best[b]).then(b.cmp(&a)))
    };
    let end = best_of(&mut (0..n).filter(reached).filter(|&v| leaves[v] == 0))
        .or_else(|| best_of(&mut (0..n).filter(reached).filter(|&v| out[v].is_empty())))
        .or_else(|| best_of(&mut (0..n).filter(reached)));
    let Some(end) = end else {
        return Vec::new();
    };
    let mut nodes = vec![end];
    let mut v = end;
    while let Some((u, arc)) = pred[v] {
        if let Some(chain) = st.chains.get(&arc) {
            let inner = &chain[1..chain.len() - 1];
            // The chain may be stored in either direction.
            if chain.last() == Some(&v) {
                nodes.extend(inner.iter().rev().copied());
            } else {
                nodes.extend(inner.iter().copied());
            }
        }
        nodes.push(u);
        v = u;
    }
    nodes.reverse();
    nodes
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use crate::geometry::Size;
    use crate::graph::{Edge, Graph, Node, NodeId};
    use crate::options::{Direction, LayoutOptions};

    /// The heavy side of a diamond is the main path, and its nodes share one cross coordinate.
    #[test]
    fn the_heaviest_path_is_drawn_on_one_line() {
        let node = || Node {
            size: Size::new(80.0, 40.0),
            ..Node::default()
        };
        let edge = |a: u32, b: u32, w: f64| Edge {
            source: NodeId(a),
            target: NodeId(b),
            weight: w,
            ..Edge::default()
        };
        // 0 -> 1 -> 3 is heavy and 0 -> 2 -> 3 light. Node 4 crowds the layer of 1.
        let g = Graph {
            nodes: vec![node(), node(), node(), node(), node()],
            edges: vec![
                edge(0, 1, 50.0),
                edge(1, 3, 50.0),
                edge(0, 2, 1.0),
                edge(2, 3, 1.0),
                edge(0, 4, 1.0),
            ],
        };
        let mut o = LayoutOptions {
            direction: Direction::TopBottom,
            ..LayoutOptions::default()
        };
        o.tuning.coords.main_path_room = 100;
        let l = crate::layout(&g, &o).unwrap();
        let x = |i: usize| l.nodes[i].center.x;
        assert!(
            (x(0) - x(1)).abs() < 0.5 && (x(1) - x(3)).abs() < 0.5,
            "{:?}",
            l.nodes
        );
    }
}

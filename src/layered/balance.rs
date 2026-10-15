//! Phase `BalanceSides`: hangs side branches on both sides of the main path.
//!
//! Crossing minimisation does not care which side a branch lands on, so block-structured models
//! often get every branch on one side. Each lobe left after removing the path can be mirrored as
//! a unit without changing its own crossings, so a flip is kept only if the exact crossing count
//! of the layers it touches does not rise. Lobes go biggest first, aiming for equal room on both
//! sides in each layer. Only the layer order changes.

use super::mainpath;
use super::order::count_crossings;
use super::state::LayeredState;
use crate::error::LayoutError;
use crate::geometry::SLACK;
use crate::options::LayoutOptions;
use crate::sort::SortDyn;

/// Which side of the main path a node is drawn on, in order-axis terms.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    /// Above the path in a left-to-right drawing.
    Before,
    After,
}

struct Lobe {
    nodes: Vec<usize>,
    span: (usize, usize),
}

/// Mirrors lobes across the main path while it improves balance without adding crossings.
pub fn balance_sides(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    if o.tuning.order.balance_rounds == 0 || st.keeps_structure() {
        return Ok(());
    }
    st.reindex();
    let Some(frame) = Frame::of(st) else {
        return Ok(());
    };
    let Frame {
        layer_of,
        spine_at,
        pred,
        lobes,
        ..
    } = frame;
    let n = st.len();
    let n_layers = st.layers.len();
    // Extent plus following gap, matching what `LayeredState::sep` adds between neighbours.
    let room: Vec<f64> = (0..n)
        .map(|v| st.nodes[v].w_cross + st.gap(v, v, o))
        .collect();

    let mut layers = std::mem::take(&mut st.layers);
    let mut pos = vec![0usize; n];
    for layer in &layers {
        for (i, &v) in layer.iter().enumerate() {
            pos[v] = i;
        }
    }
    // Biggest first, so the small lobes fill in around the big ones.
    let mut by_size: Vec<usize> = (0..lobes.len()).collect();
    by_size.sort_by_key_dyn(|&k| (std::cmp::Reverse(lobes[k].nodes.len()), k));

    for _ in 0..o.tuning.order.balance_rounds {
        let mut moved = false;
        for &k in &by_size {
            let lobe = &lobes[k];
            let Some(side) = side_of(lobe, &layer_of, &spine_at, &pos) else {
                continue;
            };
            let (l0, l1) = lobe.span;
            let (c0, c1) = (l0.saturating_sub(1), (l1 + 1).min(n_layers - 1));
            let before_crossings = count_crossings(&layers[c0..=c1], &pred, &pos);
            // Judge over all layers, since the extent is a maximum over the whole drawing.
            let (before_balance, before_extent) = imbalance(&layers, &spine_at, &room);
            let mut best: Option<(usize, f64, Vec<Vec<usize>>)> = None;
            for outer in [false, true] {
                let trial = flipped(&layers, lobe, &layer_of, &spine_at, side, outer);
                let mut trial_pos = pos.clone();
                for layer in &trial[c0..=c1] {
                    for (i, &v) in layer.iter().enumerate() {
                        trial_pos[v] = i;
                    }
                }
                let crossings = count_crossings(&trial[c0..=c1], &pred, &trial_pos);
                let (balance, extent) = imbalance(&trial, &spine_at, &room);
                if crossings > before_crossings
                    || extent > before_extent + SLACK
                    || balance >= before_balance - SLACK
                {
                    continue;
                }
                if best
                    .as_ref()
                    .is_none_or(|(c, b, _)| (crossings, balance) < (*c, *b))
                {
                    best = Some((crossings, balance, trial));
                }
            }
            if let Some((_, _, trial)) = best {
                for li in l0..=l1 {
                    layers[li] = trial[li].clone();
                    for (i, &v) in layers[li].iter().enumerate() {
                        pos[v] = i;
                    }
                }
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    st.layers = layers;
    st.reindex();
    Ok(())
}

struct Frame {
    layer_of: Vec<usize>,
    spine_at: Vec<Option<usize>>,
    pred: Vec<Vec<usize>>,
    lobes: Vec<Lobe>,
}

impl Frame {
    /// `None` without a single usable main path, or when seeds or group lanes fix the order.
    fn of(st: &LayeredState) -> Option<Frame> {
        if st.has_seeds() || st.band_count >= 2 || st.n_real < 4 {
            return None;
        }
        if !mainpath::has_a_flow(st) || mainpath::several_flows(st) {
            return None;
        }
        let spine = mainpath::spine(st);
        if spine.len() < 3 {
            return None;
        }
        let n = st.len();
        let mut layer_of = vec![usize::MAX; n];
        for (li, layer) in st.layers.iter().enumerate() {
            for &v in layer {
                layer_of[v] = li;
            }
        }
        let mut on_spine = vec![false; n];
        let mut spine_at: Vec<Option<usize>> = vec![None; st.layers.len()];
        for &v in &spine {
            if layer_of[v] == usize::MAX || spine_at[layer_of[v]].is_some() {
                return None;
            }
            on_spine[v] = true;
            spine_at[layer_of[v]] = Some(v);
        }
        let (pred, succ) = st.adjacency();
        let lobes = lobes(n, &on_spine, &pred, &succ, &layer_of);
        Some(Frame {
            layer_of,
            spine_at,
            pred,
            lobes,
        })
    }
}

/// Whether placement lays the drawing out around its main path. Unweighted models only, since
/// varying weights already decide what runs straight.
pub(super) fn frames_the_drawing(st: &LayeredState, o: &LayoutOptions) -> bool {
    o.coords.branch_bands && !mainpath::weighted(st) && Frame::of(st).is_some()
}

/// `(before, after)` node sets that placement must keep apart, one pair per two branches on the
/// same side of the main path. Layer order separates branches within a layer but not across
/// layers. Pairs that interleave or swap sides in a shared layer are left out.
pub(super) fn branch_bands(st: &LayeredState, o: &LayoutOptions) -> Vec<(Vec<usize>, Vec<usize>)> {
    if !frames_the_drawing(st, o) {
        return Vec::new();
    }
    let Some(frame) = Frame::of(st) else {
        return Vec::new();
    };
    let pos = &st.order_of;
    if pos.len() != st.len() {
        return Vec::new();
    }
    // Distances in order steps from the path, so both sides compare the same way.
    struct Seen {
        side: Side,
        near_far: Vec<Option<(usize, usize)>>,
    }
    let mut seen: Vec<Option<Seen>> = Vec::with_capacity(frame.lobes.len());
    for lobe in &frame.lobes {
        let Some(side) = side_of(lobe, &frame.layer_of, &frame.spine_at, pos) else {
            seen.push(None);
            continue;
        };
        let (l0, l1) = lobe.span;
        let mut near_far: Vec<Option<(usize, usize)>> = vec![None; l1 - l0 + 1];
        for &v in &lobe.nodes {
            let li = frame.layer_of[v];
            let Some(anchor) = frame.spine_at[li] else {
                continue;
            };
            let d = pos[v].abs_diff(pos[anchor]);
            let slot = &mut near_far[li - l0];
            *slot = Some(slot.map_or((d, d), |(a, b)| (a.min(d), b.max(d))));
        }
        seen.push(Some(Seen { side, near_far }));
    }
    let mut out = Vec::new();
    for a in 0..frame.lobes.len() {
        for b in a + 1..frame.lobes.len() {
            let (Some(sa), Some(sb)) = (&seen[a], &seen[b]) else {
                continue;
            };
            if sa.side != sb.side {
                continue;
            }
            let (la, lb) = (&frame.lobes[a], &frame.lobes[b]);
            let (s0, s1) = (la.span.0.max(lb.span.0), la.span.1.min(lb.span.1));
            if s0 > s1 {
                continue;
            }
            // The nearer lobe must be the same in every shared layer, or there is no band.
            let mut a_inner: Option<bool> = None;
            let mut consistent = true;
            for li in s0..=s1 {
                let (Some((an, af)), Some((bn, bf))) =
                    (sa.near_far[li - la.span.0], sb.near_far[li - lb.span.0])
                else {
                    continue;
                };
                let here = if af < bn {
                    true
                } else if bf < an {
                    false
                } else {
                    consistent = false;
                    break;
                };
                if a_inner.is_some_and(|x| x != here) {
                    consistent = false;
                    break;
                }
                a_inner = Some(here);
            }
            let (true, Some(a_inner)) = (consistent, a_inner) else {
                continue;
            };
            let within = |lobe: &Lobe| -> Vec<usize> {
                lobe.nodes
                    .iter()
                    .copied()
                    .filter(|&v| (s0..=s1).contains(&frame.layer_of[v]))
                    .collect()
            };
            let (inner, outer) = if a_inner {
                (within(la), within(lb))
            } else {
                (within(lb), within(la))
            };
            out.push(match sa.side {
                Side::Before => (outer, inner),
                Side::After => (inner, outer),
            });
        }
    }
    out
}

/// Connected components off the path, by union-find.
fn lobes(
    n: usize,
    on_spine: &[bool],
    pred: &[Vec<usize>],
    succ: &[Vec<usize>],
    layer_of: &[usize],
) -> Vec<Lobe> {
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], mut v: usize) -> usize {
        while parent[v] != v {
            parent[v] = parent[parent[v]];
            v = parent[v];
        }
        v
    }
    for v in 0..n {
        if on_spine[v] {
            continue;
        }
        for &u in succ[v].iter().chain(&pred[v]) {
            if !on_spine[u] {
                let (a, b) = (find(&mut parent, u), find(&mut parent, v));
                if a != b {
                    parent[a.max(b)] = a.min(b);
                }
            }
        }
    }
    let mut members: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for v in (0..n).filter(|&v| !on_spine[v] && layer_of[v] != usize::MAX) {
        let root = find(&mut parent, v);
        members.entry(root).or_default().push(v);
    }
    members
        .into_values()
        .map(|nodes| {
            let lo = nodes.iter().map(|&v| layer_of[v]).min().unwrap_or(0);
            let hi = nodes.iter().map(|&v| layer_of[v]).max().unwrap_or(0);
            Lobe {
                nodes,
                span: (lo, hi),
            }
        })
        .collect()
}

/// The side `lobe` is on, or `None` when it straddles the path or reaches past its ends.
fn side_of(
    lobe: &Lobe,
    layer_of: &[usize],
    spine_at: &[Option<usize>],
    pos: &[usize],
) -> Option<Side> {
    let mut side = None;
    for &v in &lobe.nodes {
        let anchor = spine_at[layer_of[v]]?;
        let here = if pos[v] < pos[anchor] {
            Side::Before
        } else {
            Side::After
        };
        match side {
            None => side = Some(here),
            Some(s) if s != here => return None,
            _ => {}
        }
    }
    side
}

/// Sum of squared before/after room differences, and the max room before plus the max room
/// after, i.e., the cross extent once the path is straight.
fn imbalance(layers: &[Vec<usize>], spine_at: &[Option<usize>], room: &[f64]) -> (f64, f64) {
    let mut total = 0.0;
    let (mut most_before, mut most_after) = (0.0f64, 0.0f64);
    for (layer, anchor) in layers.iter().zip(spine_at) {
        let Some(anchor) = *anchor else {
            continue;
        };
        let (mut before, mut after, mut past) = (0.0, 0.0, false);
        for &v in layer {
            if v == anchor {
                past = true;
            } else if past {
                after += room[v];
            } else {
                before += room[v];
            }
        }
        total += (before - after) * (before - after);
        most_before = most_before.max(before);
        most_after = most_after.max(after);
    }
    (total, most_before + most_after)
}

/// `layers` with `lobe` reversed onto the other side of the path, next to the path's node or, if
/// `outer`, at the far end.
fn flipped(
    layers: &[Vec<usize>],
    lobe: &Lobe,
    layer_of: &[usize],
    spine_at: &[Option<usize>],
    from: Side,
    outer: bool,
) -> Vec<Vec<usize>> {
    let mut out = layers.to_vec();
    let (l0, l1) = lobe.span;
    let mut mine = vec![false; layer_of.len()];
    for &v in &lobe.nodes {
        mine[v] = true;
    }
    for li in l0..=l1 {
        let Some(anchor) = spine_at[li] else {
            continue;
        };
        let moving: Vec<usize> = layers[li].iter().copied().filter(|&v| mine[v]).collect();
        if moving.is_empty() {
            continue;
        }
        let rest: Vec<usize> = layers[li].iter().copied().filter(|&v| !mine[v]).collect();
        let at = rest.iter().position(|&v| v == anchor).unwrap_or(0);
        let mirrored = moving.iter().rev().copied();
        let mut layer = Vec::with_capacity(layers[li].len());
        match (from, outer) {
            (Side::Before, false) => {
                layer.extend_from_slice(&rest[..=at]);
                layer.extend(mirrored);
                layer.extend_from_slice(&rest[at + 1..]);
            }
            (Side::Before, true) => {
                layer.extend_from_slice(&rest);
                layer.extend(mirrored);
            }
            (Side::After, false) => {
                layer.extend_from_slice(&rest[..at]);
                layer.extend(mirrored);
                layer.extend_from_slice(&rest[at..]);
            }
            (Side::After, true) => {
                layer.extend(mirrored);
                layer.extend_from_slice(&rest);
            }
        }
        out[li] = layer;
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use crate::geometry::Size;
    use crate::graph::{Edge, Graph, Node, NodeId};
    use crate::options::{Direction, LayoutOptions};

    #[test]
    fn a_four_way_split_hangs_branches_on_both_sides() {
        let node = || Node {
            size: Size::new(60.0, 40.0),
            ..Node::default()
        };
        let edge = |a: u32, b: u32| Edge {
            source: NodeId(a),
            target: NodeId(b),
            ..Edge::default()
        };
        // 0 -> 1 -> {2, 3, 4, 5} -> 6 -> 7, with branch 2 one longer (2 -> 8 -> 6) so the main
        // path runs through it.
        let g = Graph {
            nodes: (0..9).map(|_| node()).collect(),
            edges: vec![
                edge(0, 1),
                edge(1, 2),
                edge(1, 3),
                edge(1, 4),
                edge(1, 5),
                edge(2, 8),
                edge(8, 6),
                edge(3, 6),
                edge(4, 6),
                edge(5, 6),
                edge(6, 7),
            ],
        };
        let o = LayoutOptions {
            direction: Direction::LeftRight,
            ..LayoutOptions::layered_default()
        };
        let layout = crate::layout(&g, &o).unwrap();
        let y = |v: usize| layout.nodes[v].center.y;
        let line = y(1);
        let above = [3, 4, 5].iter().filter(|&&v| y(v) < line - 1.0).count();
        let below = [3, 4, 5].iter().filter(|&&v| y(v) > line + 1.0).count();
        assert!(
            above >= 1 && below >= 1,
            "above {above}, below {below}, line {line}"
        );
        assert!((y(2) - line).abs() < 1.0 && (y(6) - line).abs() < 1.0);
    }
}

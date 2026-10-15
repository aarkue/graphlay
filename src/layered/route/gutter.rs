//! The spine of every route and its lanes through the gutters.
//!
//! [`waypoints`] lays each arc's spine through its routing dummies. [`assign_lanes`] gives every
//! arc that changes height in a gutter its own vertical lane there, ordered by
//! [`order_gutter_lanes`] to cross as little as possible.

use std::collections::BTreeMap;

use super::{PortMap, RouteCtx};
use crate::geometry::monotone::monotone_project;
use crate::sort::SortDyn;

/// One arc's passage through a gutter, arriving at cross-position `from` and leaving at `to`.
#[derive(Clone, Copy)]
pub(super) struct GutterSeg {
    pub(super) arc: usize,
    pub(super) from: f64,
    pub(super) to: f64,
}

/// Spine waypoints per arc: source stub, dummy centres, target stub. The dummies are made
/// monotone where that keeps them clear of their layers' nodes.
pub(super) fn waypoints(ctx: &RouteCtx, port: &PortMap) -> BTreeMap<usize, Vec<(f64, f64)>> {
    let (st, b) = (ctx.st, ctx.b);
    let (cx, cy, layers) = (&b.cx, &b.cy, &st.layers);
    let mut waypoints: BTreeMap<usize, Vec<(f64, f64)>> = BTreeMap::new();
    for (arc, chain) in ctx.ordered() {
        let m = chain.len();
        if m < 2 {
            continue;
        }
        let plo = port[&(arc, true)];
        let phi = port[&(arc, false)];
        let mut v = Vec::with_capacity(m);
        for (i, &node) in chain.iter().enumerate() {
            if i == 0 {
                v.push(plo.stub);
            } else if i == m - 1 {
                v.push(phi.stub);
            } else {
                v.push((cx[node], cy[node]));
            }
        }
        if m > 2 {
            let ys: Vec<f64> = v[1..m - 1].iter().map(|p| p.1).collect();
            let mono = monotone_project(&ys, v[m - 1].1 >= v[0].1);
            // Pooling waypoints to their mean height can land inside a node of their layer, so
            // the fit applies only if every pooled height clears it by half the stroke width.
            let half = st.thickness.get(arc).copied().unwrap_or(0.0) / 2.0;
            let clear = chain[1..m - 1].iter().zip(&mono).all(|(&node, &y)| {
                let rank = st.nodes[node].layer_clamped();
                layers.get(rank).is_none_or(|layer| {
                    layer.iter().all(|&u| {
                        u >= b.n_real
                            || u == chain[0]
                            || u == chain[m - 1]
                            || (y - b.cy[u]).abs() >= b.order_w[u] / 2.0 + half + 1.0
                    })
                })
            });
            if clear {
                for (p, &y) in v[1..m - 1].iter_mut().zip(&mono) {
                    p.1 = y;
                }
            }
        }
        waypoints.insert(arc, v);
    }
    waypoints
}

/// Give each arc with a riser in a gutter its own vertical lane, ordered by
/// [`order_gutter_lanes`]. One map per layer index from arc to lane x, so a last-layer waypoint
/// cannot over-index.
pub(super) fn assign_lanes(
    ctx: &RouteCtx,
    waypoints: &BTreeMap<usize, Vec<(f64, f64)>>,
) -> Vec<BTreeMap<usize, f64>> {
    let (cx, lw) = (&ctx.b.cx, &ctx.b.layer_w);
    let layers = &ctx.st.layers;
    let n_layers = layers.len();
    let layer_x: Vec<f64> = layers
        .iter()
        .map(|l| l.first().map(|&n| cx[n]).unwrap_or(0.0))
        .collect();
    let layer_half: Vec<f64> = layers
        .iter()
        .map(|l| l.iter().map(|&n| lw[n] / 2.0).fold(0.0, f64::max))
        .collect();
    let n_gutters = n_layers.max(1);
    let mut gutter_segs: Vec<Vec<GutterSeg>> = (0..n_gutters).map(|_| vec![]).collect();
    for (arc, chain) in ctx.ordered() {
        if chain.len() < 2 {
            continue;
        }
        let v = &waypoints[&arc];
        for i in 0..chain.len() - 1 {
            let l = ctx.layer_of[chain[i]] as usize;
            gutter_segs[l].push(GutterSeg {
                arc,
                from: v[i].1,
                to: v[i + 1].1,
            });
        }
    }
    let mut lane: Vec<BTreeMap<usize, f64>> = (0..n_gutters).map(|_| BTreeMap::new()).collect();
    for (l, segs) in gutter_segs.iter_mut().enumerate() {
        let gl = layer_x[l] + layer_half[l];
        let gr = if l + 1 < n_layers {
            layer_x[l + 1] - layer_half[l + 1]
        } else {
            gl + ctx.o.spacing.layer_gap
        };
        // A passage without a riser needs no lane of its own, so it takes the middle.
        let (straight, risers): (Vec<GutterSeg>, Vec<GutterSeg>) =
            segs.iter().partition(|s| (s.from - s.to).abs() < 0.5);
        for seg in &straight {
            lane[l].insert(seg.arc, (gl + gr) / 2.0);
        }
        let mut risers = risers;
        order_gutter_lanes(&mut risers);
        let k = risers.len();
        for (i, seg) in risers.iter().enumerate() {
            let f = (i as f64 + 1.0) / (k as f64 + 1.0);
            lane[l].insert(seg.arc, gl + (gr - gl) * f);
        }
    }
    lane
}

/// Whether putting `a` on the earlier lane and `b` on the later one makes them cross. A run at
/// exactly a riser's end height counts, since the shared column draws as an overlap.
pub(super) fn lanes_cross(a: &GutterSeg, b: &GutterSeg) -> bool {
    let between = |c: f64, s: &GutterSeg| {
        let (lo, hi) = (s.from.min(s.to), s.from.max(s.to));
        lo - 0.5 <= c && c <= hi + 0.5
    };
    between(b.from, a) || between(a.to, b)
}

/// Whether putting `a` on the earlier lane and `b` on the later one draws a run of each on one
/// line. The crossing metric misses this overlap, and a reader cannot tell the edges apart.
fn lanes_share_a_column(a: &GutterSeg, b: &GutterSeg) -> bool {
    (a.to - b.from).abs() < 0.5
}

/// Order a gutter's passages onto its lanes. A pair that crosses in only one order becomes a
/// constraint, honoured topologically. Pairs that cross either way avoid a shared column.
fn order_gutter_lanes(segs: &mut [GutterSeg]) {
    let n = segs.len();
    if n < 2 {
        return;
    }
    let mid = |s: &GutterSeg| (s.from + s.to) / 2.0;
    segs.sort_unstable_by_dyn(|a, b| mid(a).total_cmp(&mid(b)).then(a.arc.cmp(&b.arc)));
    let mut after: Vec<Vec<usize>> = vec![vec![]; n]; // after[i]: the j that must follow i
    let mut indeg = vec![0usize; n];
    let mut prefer_after: Vec<Vec<usize>> = vec![vec![]; n]; // the j that should follow i
    let mut prefer_indeg = vec![0usize; n];
    for i in 0..n {
        for j in i + 1..n {
            let ij = lanes_cross(&segs[i], &segs[j]);
            let ji = lanes_cross(&segs[j], &segs[i]);
            if ij && !ji {
                after[j].push(i);
                indeg[i] += 1;
            } else if ji && !ij {
                after[i].push(j);
                indeg[j] += 1;
            } else if ij && ji {
                let sij = lanes_share_a_column(&segs[i], &segs[j]);
                let sji = lanes_share_a_column(&segs[j], &segs[i]);
                if sij && !sji {
                    prefer_after[j].push(i);
                    prefer_indeg[i] += 1;
                } else if sji && !sij {
                    prefer_after[i].push(j);
                    prefer_indeg[j] += 1;
                }
            }
        }
    }
    // Kahn's algorithm. On a cycle nothing is ready, so take the lowest base index left.
    let mut done = vec![false; n];
    let mut order = Vec::with_capacity(n);
    while order.len() < n {
        let pick = (0..n)
            .find(|&i| !done[i] && indeg[i] == 0 && prefer_indeg[i] == 0)
            .or_else(|| (0..n).find(|&i| !done[i] && indeg[i] == 0))
            .or_else(|| (0..n).find(|&i| !done[i]));
        let Some(i) = pick else { break };
        done[i] = true;
        order.push(i);
        for &j in &after[i] {
            if !done[j] {
                indeg[j] = indeg[j].saturating_sub(1);
            }
        }
        for &j in &prefer_after[i] {
            if !done[j] {
                prefer_indeg[j] = prefer_indeg[j].saturating_sub(1);
            }
        }
    }
    let placed: Vec<GutterSeg> = order.iter().map(|&i| segs[i]).collect();
    segs.copy_from_slice(&placed);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Arcs 0 and 1 cross in either order, but arc 0 first would share a column at 100. A hard
    /// constraint still outranks that preference.
    #[test]
    fn the_lane_order_keeps_a_forced_crossing_off_a_shared_column() {
        let seg = |arc: usize, from: f64, to: f64| GutterSeg { arc, from, to };
        let mut segs = vec![seg(0, 0.0, 100.0), seg(1, 100.0, 40.0)];
        order_gutter_lanes(&mut segs);
        assert_eq!([segs[0].arc, segs[1].arc], [1, 0]);
        // Arc 2 after arc 1 would cut arc 1's riser, so arc 2 is forced first.
        let mut segs = vec![seg(0, 0.0, 100.0), seg(1, 100.0, 40.0), seg(2, 60.0, 20.0)];
        order_gutter_lanes(&mut segs);
        let order: Vec<usize> = segs.iter().map(|s| s.arc).collect();
        assert_eq!(order, vec![2, 1, 0], "{order:?}");
    }
}

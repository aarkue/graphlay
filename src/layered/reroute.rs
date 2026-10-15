//! Position-driven re-route. Lays the graph out over coordinates the caller already has (e.g., a
//! dragged node or a saved diagram). Ranks are clustered from the seeded layer-axis coordinates,
//! then the usual dummy, coordinate, port and routing phases run.
//! [`super::pipeline::Pipeline::reroute`] lists the phases.

use crate::error::LayoutError;
use crate::geometry::hit::nearest_clear_lane;
use crate::options::{LayoutOptions, RoutingStyle};

use super::finish::LayeredOutput;
use super::pipeline::Pipeline;
use super::prepare::{EdgePortHints, LayeredInput};

use super::coord::{brandes_koepf, seeded_coords, straighten_long_edges};
use super::dummy::{build_edge_chains, insert_dummies};
use super::state::LayeredState;
use crate::sort::SortDyn;

/// Routes edges over the caller's final node positions (seeds). Every node must have a seed.
/// `hints` holds the caller's port hints per edge, or is empty (see [`super::layout_layered`]).
pub(crate) fn reroute_from_positions(
    input: &LayeredInput,
    hints: &[EdgePortHints],
) -> Result<LayeredOutput, LayoutError> {
    input.validate()?;
    let n = input.nodes.len();
    if n == 0 {
        return Ok(LayeredOutput::empty(0, input.edges.len()));
    }
    // Compaction spacing assumes a later pass removes the slack, which a reroute never runs.
    let mut options = input.options;
    options.spacing.node_gap = options.spacing.reroute_node_gap;
    let mut st = LayeredState::new(input);
    st.port_hints = hints.to_vec();
    Pipeline::reroute().run(&mut st, &options)?;
    let mut out = super::output(&mut st, n, input.edges.len());
    if options.routing.style != RoutingStyle::Diagonal {
        // The relief moves the end at the arc's `to`, which is the caller's source when the edge
        // is ranked backwards. A hinted end stays where the caller put it.
        let tail_hinted: Vec<bool> = (0..input.edges.len())
            .map(|e| {
                let backwards = input.ranked_backwards.get(e).copied().unwrap_or(false);
                hints
                    .get(e)
                    .is_some_and(|h| h[usize::from(!backwards)].is_some())
            })
            .collect();
        relieve_end_approaches(&mut out.routes, &out.centers, input, &tail_hinted);
    }
    Ok(out)
}

/// [`super::pipeline::Phase::RankFromPositions`]. Reads layers, waypoints and within-layer order
/// off the caller's positions.
pub(super) fn rank_from_positions(
    st: &mut LayeredState,
    o: &LayoutOptions,
) -> Result<(), LayoutError> {
    // Seeds are already in flow space.
    let n = st.n_real;
    for i in 0..n {
        let (along, cross) = st.nodes[i].seed.unwrap_or((0.0, 0.0));
        st.nodes[i].along = along;
        st.nodes[i].cross = cross;
    }

    // A new layer opens wherever consecutive boxes stop overlapping on the layer axis.
    let mut by_layer: Vec<usize> = (0..n).collect();
    // Node index breaks ties so the walk over consecutive pairs is deterministic.
    by_layer.sort_unstable_by_dyn(|&a, &b| {
        st.nodes[a]
            .along
            .total_cmp(&st.nodes[b].along)
            .then(a.cmp(&b))
    });
    let mut cur = 0i32;
    for w in 1..n {
        let (a, b) = (by_layer[w - 1], by_layer[w]);
        if st.nodes[b].along - st.nodes[a].along
            >= 0.5 * (st.nodes[a].w_along + st.nodes[b].w_along)
        {
            cur += 1;
        }
        st.nodes[b].rank = cur;
    }
    let n_layers = (cur + 1) as usize;

    // The router emits layer-ascending polylines, so a flipped arc is reversed on output.
    for i in 0..st.arcs.len() {
        let (s, t) = (st.arcs[i].from, st.arcs[i].to);
        let rev = st.nodes[s].rank > st.nodes[t].rank;
        if rev {
            st.arcs[i].from = t;
            st.arcs[i].to = s;
        }
        st.arcs[i].reversed = rev;
    }
    super::pipeline::check_dummy_limit(st, o)?;
    insert_dummies(st)?;
    build_edge_chains(st)?;
    let n_all = st.len();

    // Mean layer coordinate per row, for dummies and the gutter estimate only. Real-node ports
    // still use each node's own coordinate.
    let mut lay_sum = vec![0.0f64; n_layers];
    let mut lay_cnt = vec![0.0f64; n_layers];
    for i in 0..n {
        lay_sum[st.nodes[i].layer()] += st.nodes[i].along;
        lay_cnt[st.nodes[i].layer()] += 1.0;
    }
    let lay_rep: Vec<f64> = (0..n_layers)
        .map(|l| {
            if lay_cnt[l] > 0.0 {
                lay_sum[l] / lay_cnt[l]
            } else {
                0.0
            }
        })
        .collect();

    // The interpolated cross coordinate only decides the within-layer order.
    let chains = std::mem::take(&mut st.chains);
    for chain in chains.values() {
        if chain.len() < 3 {
            continue;
        }
        let (lo, hi) = (chain[0], chain[chain.len() - 1]);
        let span = st.nodes[hi].along - st.nodes[lo].along;
        for &d in &chain[1..chain.len() - 1] {
            let l = st.nodes[d].layer();
            st.nodes[d].along = lay_rep[l];
            let frac = if span.abs() > 1e-6 {
                (lay_rep[l] - st.nodes[lo].along) / span
            } else {
                0.0
            };
            st.nodes[d].cross =
                st.nodes[lo].cross + (st.nodes[hi].cross - st.nodes[lo].cross) * frac;
        }
    }
    st.chains = chains;

    // On a tie the real node sorts first (dummies have higher indices), so separation pushes the
    // dummy clear.
    let mut layers: Vec<Vec<usize>> = vec![vec![]; n_layers];
    for i in 0..n_all {
        layers[st.nodes[i].layer()].push(i);
    }
    for l in &mut layers {
        l.sort_unstable_by_dyn(|&a, &b| {
            st.nodes[a]
                .cross
                .total_cmp(&st.nodes[b].cross)
                .then(a.cmp(&b))
        });
    }
    st.layers = layers;
    st.reindex();
    Ok(())
}

/// [`super::pipeline::Phase::PlaceOverPositions`]. Pins every real node where it was dropped,
/// places waypoints with the fresh layout's optimiser and snaps each long edge onto one clear lane.
pub(super) fn place_over_positions(
    st: &mut LayeredState,
    o: &LayoutOptions,
) -> Result<(), LayoutError> {
    // Straight interpolation would rake waypoints through skipped boxes, so use the optimiser.
    let n = st.n_real;
    for node in st.nodes.iter_mut().take(n) {
        node.pinned = true;
    }
    // Same optimiser as a fresh layout, so a node dropped in place leaves its long edges alone.
    if !(o.coords.placement == crate::options::Placement::NetworkSimplex
        && super::simplex::place_cross_simplex(st, o)?)
    {
        brandes_koepf(st, o)?;
    }
    seeded_coords(st, o)?;
    straighten_long_edges(st, o)?;

    // Per-layer separation can zigzag a chain's dummies around dragged nodes, so each chain gets
    // one clear lane.
    let chains = std::mem::take(&mut st.chains);
    let mut layers = std::mem::take(&mut st.layers);
    for chain in chains.values() {
        if chain.len() < 3 {
            continue;
        }
        let (lo, hi) = (chain[0], chain[chain.len() - 1]);
        let dummies: Vec<usize> = chain[1..chain.len() - 1]
            .iter()
            .copied()
            .filter(|&d| st.nodes[d].is_dummy())
            .collect();
        if dummies.is_empty() {
            continue;
        }
        let mut ys: Vec<f64> = dummies.iter().map(|&d| st.nodes[d].cross).collect();
        ys.sort_unstable_by_dyn(f64::total_cmp);
        let desired = ys[ys.len() / 2];
        let mut intervals: Vec<(f64, f64)> = vec![];
        for &d in &dummies {
            for &r in &layers[st.nodes[d].layer()] {
                if r < n && r != lo && r != hi {
                    let h = st.nodes[r].w_cross / 2.0 + o.spacing.dummy_gap;
                    intervals.push((st.nodes[r].cross - h, st.nodes[r].cross + h));
                }
            }
        }
        let lane = nearest_clear_lane(desired, &mut intervals);
        for &d in &dummies {
            st.nodes[d].cross = lane;
        }
    }
    // Port spreading reads the within-layer order, which the lane snap may have changed.
    for l in &mut layers {
        l.sort_unstable_by_dyn(|&a, &b| {
            st.nodes[a]
                .cross
                .total_cmp(&st.nodes[b].cross)
                .then(a.cmp(&b))
        });
    }
    st.chains = chains;
    st.layers = layers;
    st.reindex();
    Ok(())
}

/// Re-attaches an orthogonal route's target end to a perpendicular border when its approach rakes
/// a node and the new end does not, as happens once dragging breaks the columns. Works in output
/// space and skips edges in `tail_hinted`.
fn relieve_end_approaches(
    routes: &mut [Vec<(f64, f64)>],
    centers: &[(f64, f64)],
    input: &LayeredInput,
    tail_hinted: &[bool],
) {
    let n = input.nodes.len();
    let box_of = |i: usize| {
        (
            centers[i].0,
            centers[i].1,
            input.nodes[i].width / 2.0,
            input.nodes[i].height / 2.0,
        )
    };
    let seg_hits = |p: (f64, f64), q: (f64, f64), c: (f64, f64, f64, f64)| -> bool {
        let inset = 1.5;
        let (l, r, t, b) = (
            c.0 - c.2 + inset,
            c.0 + c.2 - inset,
            c.1 - c.3 + inset,
            c.1 + c.3 - inset,
        );
        (0..=48).any(|k| {
            let f = k as f64 / 48.0;
            let (x, y) = (p.0 + (q.0 - p.0) * f, p.1 + (q.1 - p.1) * f);
            x > l && x < r && y > t && y < b
        })
    };
    let route_rakes = |pts: &[(f64, f64)], s: usize, t: usize| -> bool {
        pts.windows(2)
            .any(|w| (0..n).any(|i| i != s && i != t && seg_hits(w[0], w[1], box_of(i))))
    };
    for (arc, &(s, t)) in input.edges.iter().enumerate() {
        let rlen = routes[arc].len();
        if tail_hinted.get(arc).copied().unwrap_or(false) {
            continue;
        }
        if rlen < 2 || !route_rakes(&routes[arc], s, t) {
            continue;
        }
        let (tcx, tcy, _, thh) = box_of(t);
        let lo = rlen.saturating_sub(4).max(1);
        for j in (lo..=rlen - 2).rev() {
            let a = routes[arc][j];
            let e = (tcx, if a.1 < tcy { tcy - thh } else { tcy + thh });
            let mut cand = routes[arc][..=j].to_vec();
            if (tcx - a.0).abs() > 1e-6 {
                cand.push((tcx, a.1));
            }
            cand.push(e);
            if !route_rakes(&cand, s, t) {
                routes[arc] = cand;
                break;
            }
        }
    }
}

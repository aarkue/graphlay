//! Position-driven re-route. Lays the graph out over coordinates the caller already has (e.g., a
//! dragged node or a saved diagram). Ranks are clustered from the seeded layer-axis coordinates,
//! then the usual dummy, coordinate, port and routing phases run.
//! [`super::pipeline::Pipeline::reroute`] lists the phases.

use crate::error::LayoutError;
use crate::geometry::hit::nearest_clear_lane;
use crate::geometry::shape::NodeShape;
use crate::graph::PortSides;
use crate::options::{Direction, LayoutOptions, RoutingStyle};
use std::collections::BTreeMap;

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
        // Which of each route's ends, its first and its last point, the caller hinted. The last is
        // at the arc's `to`, which is the caller's source when the edge is ranked backwards. A
        // hinted end stays where the caller put it.
        let hinted: Vec<[bool; 2]> = (0..input.edges.len())
            .map(|e| {
                let backwards = input.ranked_backwards.get(e).copied().unwrap_or(false);
                let hinted = |end: usize| hints.get(e).is_some_and(|h| h[end].is_some());
                [
                    hinted(usize::from(backwards)),
                    hinted(usize::from(!backwards)),
                ]
            })
            .collect();
        join_stacked_nodes(&mut out.routes, &out.centers, input, &hinted);
        relieve_end_approaches(&mut out.routes, &out.centers, input, &hinted);
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

/// A node's box as `(centre x, centre y, half width, half height)`.
type BoxHalf = (f64, f64, f64, f64);

/// Whether the segment `p`-`q` runs through the inside of `c`, 1.5px in from its border: of its
/// box, or of the ellipse inscribed in it.
fn seg_inside(p: (f64, f64), q: (f64, f64), c: BoxHalf, ellipse: bool) -> bool {
    let (hw, hh) = (c.2 - 1.5, c.3 - 1.5);
    if hw <= 0.0 || hh <= 0.0 {
        return false;
    }
    // In units of the half extents, so the inside is the unit square or the unit disc.
    let (px, py) = ((p.0 - c.0) / hw, (p.1 - c.1) / hh);
    let (dx, dy) = ((q.0 - p.0) / hw, (q.1 - p.1) / hh);
    if ellipse {
        // |p + t d|^2 < 1 for some t in [0, 1]: the closest point of the segment to the centre.
        let len2 = dx * dx + dy * dy;
        let t = if len2 > 0.0 {
            (-(px * dx + py * dy) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        return (px + t * dx).powi(2) + (py + t * dy).powi(2) < 1.0;
    }
    // Clip the segment against the open square, axis by axis.
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for (start, delta) in [(px, dx), (py, dy)] {
        if delta.abs() < 1e-12 {
            if start.abs() >= 1.0 {
                return false;
            }
            continue;
        }
        let (a, b) = ((-1.0 - start) / delta, (1.0 - start) / delta);
        lo = lo.max(a.min(b));
        hi = hi.min(a.max(b));
    }
    lo < hi
}

/// Whether node `i` may only use the borders facing the flow.
fn facing_only(input: &LayeredInput, i: usize) -> bool {
    input.nodes[i]
        .port_sides
        .unwrap_or(input.options.routing.port_sides)
        == PortSides::FacingOnly
}

/// Joins two nodes stacked across the flow, their boxes overlapping along it, by one straight
/// segment between the borders that face each other, where it runs clear of every other box. The
/// router only knows the borders facing the flow, so it would leave one node and loop round into
/// the other. Several edges between one pair spread over the overlap. Skips edges with a hinted
/// end and works in output space.
fn join_stacked_nodes(
    routes: &mut [Vec<(f64, f64)>],
    centers: &[(f64, f64)],
    input: &LayeredInput,
    hinted: &[[bool; 2]],
) {
    let n = input.nodes.len();
    // Coordinates as `(along the flow, across it)`. The swap is its own inverse.
    let flow_x = input.options.direction == Direction::LeftRight;
    let to_flow = |p: (f64, f64)| if flow_x { p } else { (p.1, p.0) };
    let box_of = |i: usize| -> BoxHalf {
        let (a, c) = to_flow(centers[i]);
        let (wa, wc) = to_flow((input.nodes[i].width / 2.0, input.nodes[i].height / 2.0));
        (a, c, wa, wc)
    };
    let mut pairs: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for (arc, &(s, t)) in input.edges.iter().enumerate() {
        if s != t && hinted.get(arc).is_none_or(|h| !h[0] && !h[1]) {
            pairs.entry((s.min(t), s.max(t))).or_default().push(arc);
        }
    }
    let margin = input.options.spacing.port_gap;
    for ((a, b), arcs) in pairs {
        if facing_only(input, a) || facing_only(input, b) {
            continue;
        }
        let (ba, bb) = (box_of(a), box_of(b));
        let (lo, hi) = (
            (ba.0 - ba.2).max(bb.0 - bb.2),
            (ba.0 + ba.2).min(bb.0 + bb.2),
        );
        // `a` above `b` across the flow, or the other way round.
        let (upper, lower) = if ba.1 < bb.1 { (a, b) } else { (b, a) };
        let (bu, bl) = (box_of(upper), box_of(lower));
        if hi - lo < 2.0 * margin || bl.1 - bl.3 <= bu.1 + bu.3 {
            continue;
        }
        // The border across the flow at `along`: the box's, or the ellipse's outline.
        let reach = |i: usize, c: BoxHalf, along: f64| -> f64 {
            if input.nodes[i].shape == NodeShape::Ellipse {
                c.3 * (1.0 - ((along - c.0) / c.2).powi(2)).max(0.0).sqrt()
            } else {
                c.3
            }
        };
        let step = (hi - lo) / (arcs.len() + 1) as f64;
        for (k, &arc) in arcs.iter().enumerate() {
            let along = lo + step * (k + 1) as f64;
            let (from, to) = (
                bu.1 + reach(upper, bu, along),
                bl.1 - reach(lower, bl, along),
            );
            let (p, q) = ((along, from), (along, to));
            if (0..n).any(|i| i != a && i != b && seg_inside(p, q, box_of(i), false)) {
                continue;
            }
            let s_upper = input.edges[arc].0 == upper;
            let (start, end) = if s_upper { (p, q) } else { (q, p) };
            routes[arc] = vec![to_flow(start), to_flow(end)];
        }
    }
}

/// Re-attaches an orthogonal route's end to another border of its node when the route rakes a box,
/// its own source and target included, and the new end does not, as happens once dragging breaks
/// the columns. The last point goes first, then the first. Works in output space and skips the
/// ends in `hinted`.
fn relieve_end_approaches(
    routes: &mut [Vec<(f64, f64)>],
    centers: &[(f64, f64)],
    input: &LayeredInput,
    hinted: &[[bool; 2]],
) {
    let n = input.nodes.len();
    let stub = input.options.spacing.stub;
    let box_of = |i: usize| -> BoxHalf {
        (
            centers[i].0,
            centers[i].1,
            input.nodes[i].width / 2.0,
            input.nodes[i].height / 2.0,
        )
    };
    // Other nodes count by their box. A route meets its own end nodes on their borders, so those
    // count by their shape: an end on a circle lies inside the circle's box.
    let seg_hits = |p: (f64, f64), q: (f64, f64), i: usize, by_shape: bool| -> bool {
        seg_inside(
            p,
            q,
            box_of(i),
            by_shape && input.nodes[i].shape == NodeShape::Ellipse,
        )
    };
    for (arc, &(s, t)) in input.edges.iter().enumerate() {
        let rakes = |pts: &[(f64, f64)]| -> bool {
            pts.windows(2)
                .any(|w| (0..n).any(|i| seg_hits(w[0], w[1], i, i == s || i == t)))
        };
        // A self-loop leaves and re-enters its own node by design.
        if s == t || routes[arc].len() < 2 || !rakes(&routes[arc]) {
            continue;
        }
        let [first_hinted, last_hinted] = hinted.get(arc).copied().unwrap_or([false, false]);
        let (last_hinted, first_hinted) = (
            last_hinted || facing_only(input, t),
            first_hinted || facing_only(input, s),
        );
        if !last_hinted {
            if let Some(r) = reattach_end(&routes[arc], box_of(t), stub, first_hinted, rakes) {
                routes[arc] = r;
            }
        }
        if !first_hinted && rakes(&routes[arc]) {
            let reversed: Vec<(f64, f64)> = routes[arc].iter().rev().copied().collect();
            if let Some(mut r) = reattach_end(&reversed, box_of(s), stub, last_hinted, rakes) {
                r.reverse();
                routes[arc] = r;
            }
        }
    }
}

/// `route` with its last point moved onto a border of `node`: from one of its last waypoints, by
/// at most two bends to a point `stub` out from that border, then straight in. Tries the border the route
/// comes from first and keeps the first end that leaves the route clear of every box without
/// doubling back on itself. `keep_first` keeps the route's first segment, so a hinted first end
/// still leaves its border as hinted.
fn reattach_end(
    route: &[(f64, f64)],
    node: BoxHalf,
    stub: f64,
    keep_first: bool,
    rakes: impl Fn(&[(f64, f64)]) -> bool,
) -> Option<Vec<(f64, f64)>> {
    let (cx, cy, hw, hh) = node;
    let rlen = route.len();
    let first = usize::from(keep_first);
    for j in (rlen.saturating_sub(4).max(first)..=rlen - 2).rev() {
        let a = route[j];
        // Each border as (where the end goes, outward direction).
        let horizontal = |top: bool| {
            let s = if top { -1.0 } else { 1.0 };
            ((cx, cy + s * hh), (0.0, s))
        };
        let vertical = |east: bool| {
            let s = if east { 1.0 } else { -1.0 };
            ((cx + s * hw, cy), (s, 0.0))
        };
        let above = a.1 < cy;
        let east = a.0 > cx;
        for (end, out) in [
            horizontal(above),
            vertical(east),
            horizontal(!above),
            vertical(!east),
        ] {
            let outside = (end.0 + out.0 * stub, end.1 + out.1 * stub);
            for corner in [(outside.0, a.1), (a.0, outside.1)] {
                let mut cand = route[..=j].to_vec();
                for p in [corner, outside, end] {
                    push_orthogonal(&mut cand, p);
                }
                if !doubles_back(&cand) && !rakes(&cand) {
                    return Some(cand);
                }
            }
        }
    }
    None
}

/// Appends `p`, dropping a repeated point and merging a straight continuation into one segment.
fn push_orthogonal(pts: &mut Vec<(f64, f64)>, p: (f64, f64)) {
    let same = |a: f64, b: f64| (a - b).abs() < 1e-6;
    if let Some(&last) = pts.last() {
        if same(last.0, p.0) && same(last.1, p.1) {
            return;
        }
        if pts.len() >= 2 {
            let prev = pts[pts.len() - 2];
            let on_line = (same(prev.0, last.0) && same(last.0, p.0))
                || (same(prev.1, last.1) && same(last.1, p.1));
            let onward =
                (last.0 - prev.0) * (p.0 - last.0) + (last.1 - prev.1) * (p.1 - last.1) > 0.0;
            if on_line && onward {
                pts.pop();
            }
        }
    }
    pts.push(p);
}

/// Whether a segment of `pts` runs straight back along the one before it.
fn doubles_back(pts: &[(f64, f64)]) -> bool {
    pts.windows(3).any(|w| {
        let (u, v) = (
            (w[1].0 - w[0].0, w[1].1 - w[0].1),
            (w[2].0 - w[1].0, w[2].1 - w[1].1),
        );
        let cross = u.0 * v.1 - u.1 * v.0;
        cross.abs() < 1e-6 && u.0 * v.0 + u.1 * v.1 < 0.0
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn a_long_segment_does_not_step_over_a_thin_node() {
        assert!(seg_inside(
            (0.0, 0.0),
            (600.0, 0.0),
            (256.25, 0.0, 7.0, 26.0),
            false
        ));
        assert!(seg_inside(
            (0.0, 0.0),
            (600.0, 0.0),
            (256.25, 0.0, 7.0, 26.0),
            true
        ));
        assert!(!seg_inside(
            (0.0, 30.0),
            (600.0, 30.0),
            (256.25, 0.0, 7.0, 26.0),
            false
        ));
    }

    #[test]
    fn an_end_may_move_to_the_far_side_border() {
        // The route arrives from the west. Only an end on the east border is acceptable here.
        let route = [
            (-100.0, 0.0),
            (-50.0, 0.0),
            (-50.0, 0.0001),
            (-20.0, 0.0001),
        ];
        let only_east = |pts: &[(f64, f64)]| pts.last().is_some_and(|p| (p.0 - 20.0).abs() > 1e-6);
        let r = reattach_end(&route, (0.0, 0.0, 20.0, 10.0), 15.0, false, only_east).unwrap();
        assert_eq!(*r.last().unwrap(), (20.0, 0.0));
    }
}

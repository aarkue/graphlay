//! First drawing of every arc. A direct edge takes the simplest path that clears the boxes, and a
//! long edge a staircase through its gutter lanes. Flow style tries [`super::diagonal`] first.

use std::collections::BTreeMap;

use super::{diagonal, PortMap, RouteCtx};
use crate::geometry::polyline::simplify_collinear;
use crate::layered::ports::Port;

/// One polyline per arc, empty for an arc without a chain.
pub(super) fn emit_routes(
    ctx: &RouteCtx,
    port: &PortMap,
    waypoints: &BTreeMap<usize, Vec<(f64, f64)>>,
    lane: &[BTreeMap<usize, f64>],
    side_port_used: &mut [bool],
) -> Vec<Vec<(f64, f64)>> {
    let mut routes: Vec<Vec<(f64, f64)>> = vec![vec![]; ctx.st.routes.len()];
    for (arc, chain) in ctx.ordered() {
        let m = chain.len();
        if m < 2 {
            continue;
        }
        let lo = chain[0];
        let ps = port[&(arc, true)];
        let pt = port[&(arc, false)];
        // Every chain registered a gutter segment in its source layer, so the lane exists.
        let Some(&lx) = lane[ctx.layer_of[lo] as usize].get(&arc) else {
            continue;
        };
        routes[arc] = if m == 2 {
            direct_route(ctx, chain, (ps, pt), lx)
        } else if let Some(r) = ctx
            .flow_edges
            .then(|| {
                diagonal::long_route(ctx, arc, chain, (ps, pt), &waypoints[&arc], side_port_used)
            })
            .flatten()
        {
            r
        } else {
            staircase(ctx, arc, chain, (ps, pt), &waypoints[&arc], lane)
        };
    }
    routes
}

/// The first clear path of the flow diagonal and the single-bend L, else the Z through gutter
/// lane `lx`.
fn direct_route(
    ctx: &RouteCtx,
    chain: &[usize],
    (ps, pt): (Port, Port),
    lx: f64,
) -> Vec<(f64, f64)> {
    let (lo, hi) = (chain[0], chain[1]);
    let (a_s, s_s) = (ps.anchor, ps.stub);
    let (a_t, s_t) = (pt.anchor, pt.stub);
    let gutter = vec![a_s, s_s, (lx, s_s.1), (lx, s_t.1), s_t, a_t];
    let mut cands: Vec<Vec<(f64, f64)>> = vec![];
    if ctx.flow_edges {
        cands.push(vec![a_s, a_t]);
    }
    match (ps.side.horizontal(), pt.side.horizontal()) {
        (true, true) if (s_s.1 - s_t.1).abs() < 0.5 => cands.push(vec![a_s, s_s, s_t, a_t]),
        (true, true) => {}
        (true, false) => cands.push(vec![a_s, s_s, (s_t.0, s_s.1), s_t, a_t]),
        (false, _) => cands.push(vec![a_s, s_s, (s_s.0, s_t.1), s_t, a_t]),
    }
    let chosen = cands
        .into_iter()
        .find(|c| !ctx.poly_hits(c, lo, hi))
        .unwrap_or(gutter);
    simplify_collinear(chosen)
}

/// Draws a long edge as a staircase through its gutter lanes. Separate risers per arc avoid
/// crossings.
fn staircase(
    ctx: &RouteCtx,
    arc: usize,
    chain: &[usize],
    (ps, pt): (Port, Port),
    v: &[(f64, f64)],
    lane: &[BTreeMap<usize, f64>],
) -> Vec<(f64, f64)> {
    let m = chain.len();
    let mut pts: Vec<(f64, f64)> = vec![ps.anchor, v[0]];
    if !ps.side.horizontal() {
        pts.push((v[0].0, v[1].1));
    }
    for i in 0..m - 1 {
        let l = ctx.layer_of[chain[i]] as usize;
        let last_perp = i == m - 2 && !pt.side.horizontal();
        let glx = match (last_perp, lane[l].get(&arc)) {
            (true, _) => v[m - 1].0,
            (false, Some(&x)) => x,
            (false, None) => continue, // unreachable: every chain segment has a lane
        };
        // `pts` was seeded with two points and is only ever pushed to.
        let cur = pts.last().copied().unwrap_or(v[0]);
        let cur_y = cur.1;
        let y_there = v[i + 1].1;
        if (glx - cur.0).abs() > 0.5 {
            pts.push((glx, cur_y));
        }
        if (y_there - cur_y).abs() > 0.5 {
            pts.push((glx, y_there));
        }
    }
    if !pt.side.horizontal() {
        let last = pts.last().copied().unwrap_or(v[m - 1]);
        if (last.0 - v[m - 1].0).abs() > 0.5 {
            pts.push((v[m - 1].0, last.1));
        }
    }
    pts.push(v[m - 1]);
    pts.push(pt.anchor);
    simplify_collinear(pts)
}

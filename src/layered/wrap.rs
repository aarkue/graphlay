//! Wraps a long drawing into rows, the way a page wraps a line of text.
//!
//! Cuts the layer sequence at gaps few edges cross and stacks the pieces as rows. An edge over a
//! cut becomes a U on its own lane between the rows, and the edge highest in the row turns
//! outermost so the lanes nest without crossing. Runs after routing and wraps only when the result
//! fills [`crate::WrapOptions::viewport_aspect`] at a clearly larger scale.

use super::state::LayeredState;
use crate::error::LayoutError;
use crate::geometry::polyline::simplify_collinear;
use crate::options::{LayoutOptions, RoutingStyle};
use crate::sort::SortDyn;

/// Minimum row width as a share of the ideal row width.
const MIN_ROW_SHARE: f64 = 0.25;

/// Cost of a cut per ideal row width of drift, against a cost of 1 per edge it carries.
const CUT_DRIFT_COST: f64 = 3.0;

/// A cut further than this share of the ideal row width from its ideal position is not taken.
const MAX_CUT_DRIFT: f64 = 0.6;

/// A candidate cut on the layer axis and the routes that cross it.
#[derive(Clone)]
struct Cut {
    at: f64,
    /// `(index into route_ids, cross coordinate of the crossing)`.
    crossing: Vec<(usize, f64)>,
}

/// `(edge, None)` is the edge's route and `(edge, Some(k))` is lane `k` of its bundle.
type RouteId = (usize, Option<usize>);

/// Every drawn polyline, using an edge's lanes where it has them.
fn route_ids(st: &LayeredState) -> Vec<RouteId> {
    let mut out = Vec::new();
    for e in 0..st.routes.len() {
        match st.lane_routes.get(e).filter(|l| !l.is_empty()) {
            Some(lanes) => out.extend((0..lanes.len()).map(|k| (e, Some(k)))),
            None if st.routes[e].len() >= 2 => out.push((e, None)),
            None => {}
        }
    }
    out
}

fn route_of(st: &LayeredState, id: RouteId) -> &[(f64, f64)] {
    match id.1 {
        Some(k) => &st.lane_routes[id.0][k],
        None => &st.routes[id.0],
    }
}

fn row_of(x: f64, cuts: &[f64]) -> usize {
    cuts.iter().filter(|&&c| x > c).count()
}

/// `(along min, along max, cross min, cross max)` per row, before any shift.
fn row_extents(st: &LayeredState, cuts: &[f64]) -> Vec<(f64, f64, f64, f64)> {
    let mut ext = vec![
        (
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY
        );
        cuts.len() + 1
    ];
    let mut grow = |r: usize, a0: f64, a1: f64, c0: f64, c1: f64| {
        let e = &mut ext[r];
        e.0 = e.0.min(a0);
        e.1 = e.1.max(a1);
        e.2 = e.2.min(c0);
        e.3 = e.3.max(c1);
    };
    for nd in st.nodes.iter().take(st.n_real) {
        let r = row_of(nd.along, cuts);
        grow(
            r,
            nd.along - nd.w_along / 2.0,
            nd.along + nd.w_along / 2.0,
            nd.cross - nd.w_cross / 2.0,
            nd.cross + nd.w_cross / 2.0,
        );
    }
    for id in route_ids(st) {
        for &(a, c) in route_of(st, id) {
            grow(row_of(a, cuts), a, a, c, c);
        }
    }
    for pts in st.loop_routes.values() {
        for &(a, c) in pts {
            grow(row_of(a, cuts), a, a, c, c);
        }
    }
    ext
}

/// The `(d along, d cross)` shift per row and the wrapped drawing's size, where `n_cut[j]` routes
/// cross cut `j`.
fn row_shifts(
    ext: &[(f64, f64, f64, f64)],
    cuts: &[f64],
    n_cut: &[usize],
    margin: f64,
    lane: f64,
    row_gap: f64,
) -> (Vec<(f64, f64)>, (f64, f64)) {
    let left = ext[0].0;
    let mut shifts = vec![(0.0, 0.0); ext.len()];
    let mut bottom = ext[0].3;
    let mut width = ext[0].1 - left;
    for j in 1..ext.len() {
        let n = n_cut[j - 1] as f64;
        // The descents into this row run left of it.
        let da = left + margin + n * lane - cuts[j - 1];
        let band = (2.0 * margin + n * lane).max(row_gap);
        let dc = bottom + band - ext[j].2;
        shifts[j] = (da, dc);
        bottom = ext[j].3 + dc;
        width = width.max(ext[j].1 + da - left);
    }
    // The previous rows' descents run right of them.
    for j in 0..cuts.len() {
        let right = cuts[j] + shifts[j].0 + margin + n_cut[j] as f64 * lane;
        width = width.max(right - left);
    }
    (shifts, (width, bottom - ext[0].2))
}

/// Wraps the drawing into rows when that fits the viewport clearly better.
pub fn wrap_rows(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    let (w, wt) = (&o.wrap, &o.tuning.wrap);
    if w.viewport_aspect <= 0.0 || wt.max_rows < 2 || o.routing.style != RoutingStyle::Orthogonal {
        return Ok(());
    }
    // Rows would cut every swimlane apart.
    if o.order.group_lanes == crate::options::GroupLanes::Strict {
        return Ok(());
    }
    if st.layers.len() < 4 {
        return Ok(());
    }
    // Candidate cuts sit in the middle of each gap between layers.
    let span = |l: &Vec<usize>| {
        l.iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &v| {
                let nd = &st.nodes[v];
                (
                    lo.min(nd.along - nd.w_along / 2.0),
                    hi.max(nd.along + nd.w_along / 2.0),
                )
            })
    };
    let spans: Vec<(f64, f64)> = st.layers.iter().map(span).collect();
    let ids = route_ids(st);
    let mut cands: Vec<Cut> = Vec::new();
    for l in 0..spans.len() - 1 {
        let at = (spans[l].1 + spans[l + 1].0) / 2.0;
        let mut crossing = Vec::new();
        let mut ok = true;
        for (i, &id) in ids.iter().enumerate() {
            let pts = route_of(st, id);
            let hits: Vec<f64> = pts
                .windows(2)
                .filter(|s| (s[0].0 - at) * (s[1].0 - at) < 0.0)
                .map(|s| s[0].1)
                .collect();
            // The U needs one crossing, on a run along the layer axis.
            let level = pts
                .windows(2)
                .filter(|s| (s[0].0 - at) * (s[1].0 - at) < 0.0)
                .all(|s| (s[0].1 - s[1].1).abs() < 0.5);
            if hits.len() > 1 || !level {
                ok = false;
                break;
            }
            if let Some(&c) = hits.first() {
                crossing.push((i, c));
            }
            // A point on the cut has no row.
            if pts.iter().any(|p| (p.0 - at).abs() < 0.5) {
                ok = false;
                break;
            }
        }
        if ok && crossing.len() <= wt.max_cut_edges {
            cands.push(Cut { at, crossing });
        }
    }
    if cands.is_empty() {
        return Ok(());
    }
    let (margin, lane) = (
        o.spacing.layer_gap / 2.0,
        o.routing.lane_gap.max(o.spacing.port_gap / 2.0),
    );
    let fit = |(wd, ht): (f64, f64)| (w.viewport_aspect / wd).min(1.0 / ht);
    let one = row_extents(st, &[]);
    let base = fit((one[0].1 - one[0].0, one[0].3 - one[0].2));
    let (x0, x1) = (one[0].0, one[0].1);
    let mut best: Option<(f64, Vec<Cut>)> = None;
    for k in 2..=wt.max_rows as usize {
        // Each boundary greedily takes the fewest crossings, weighed against drift from the ideal.
        let mut chosen: Vec<Cut> = Vec::new();
        let row_w = (x1 - x0) / k as f64;
        for j in 1..k {
            let ideal = x0 + row_w * j as f64;
            let after = chosen.last().map_or(f64::NEG_INFINITY, |c| c.at);
            let pick = cands
                .iter()
                .filter(|c| c.at > after + row_w * MIN_ROW_SHARE)
                .min_by(|a, b| {
                    let cost = |c: &Cut| {
                        c.crossing.len() as f64 + CUT_DRIFT_COST * (c.at - ideal).abs() / row_w
                    };
                    cost(a).total_cmp(&cost(b))
                });
            match pick {
                Some(c) if (c.at - ideal).abs() < row_w * MAX_CUT_DRIFT => chosen.push(c.clone()),
                _ => break,
            }
        }
        if chosen.len() != k - 1 {
            continue;
        }
        let cuts: Vec<f64> = chosen.iter().map(|c| c.at).collect();
        let n_cut: Vec<usize> = chosen.iter().map(|c| c.crossing.len()).collect();
        let ext = row_extents(st, &cuts);
        let (_, size) = row_shifts(&ext, &cuts, &n_cut, margin, lane, o.spacing.layer_gap);
        let scale = fit(size);
        if best.as_ref().is_none_or(|b| scale > b.0) {
            best = Some((scale, chosen));
        }
    }
    let Some((scale, chosen)) = best else {
        return Ok(());
    };
    if scale < base * wt.min_gain {
        return Ok(());
    }
    apply(st, &ids, &chosen, margin, lane, o.spacing.layer_gap);
    Ok(())
}

/// Moves every row into place and routes the edges crossing a cut through their U.
fn apply(
    st: &mut LayeredState,
    ids: &[RouteId],
    chosen: &[Cut],
    margin: f64,
    lane: f64,
    row_gap: f64,
) {
    let cuts: Vec<f64> = chosen.iter().map(|c| c.at).collect();
    let n_cut: Vec<usize> = chosen.iter().map(|c| c.crossing.len()).collect();
    let ext = row_extents(st, &cuts);
    let (shifts, _) = row_shifts(&ext, &cuts, &n_cut, margin, lane, row_gap);
    let mv = |p: (f64, f64)| {
        let (da, dc) = shifts[row_of(p.0, &cuts)];
        (p.0 + da, p.1 + dc)
    };
    // The route highest in the row gets the outermost lane.
    let mut lane_of: std::collections::BTreeMap<(usize, usize), usize> = Default::default();
    for (j, c) in chosen.iter().enumerate() {
        let mut order: Vec<(usize, f64)> = c.crossing.clone();
        order.sort_by_dyn(|a, b| a.1.total_cmp(&b.1));
        for (i, &(r, _)) in order.iter().enumerate() {
            lane_of.insert((r, j), i);
        }
    }
    // The U over cut `j` from cross coordinate `c`, left to right.
    let u_turn = |j: usize, c: f64, i: usize| -> Vec<(f64, f64)> {
        let n = n_cut[j];
        let rows_bottom = ext[j].3 + shifts[j].1;
        let at = cuts[j];
        let (from, to) = (
            (at + shifts[j].0, c + shifts[j].1),
            (at + shifts[j + 1].0, c + shifts[j + 1].1),
        );
        let xr = from.0 + margin + (n - 1 - i) as f64 * lane;
        let yl = rows_bottom + margin + (n - 1 - i) as f64 * lane;
        let xl = to.0 - margin - i as f64 * lane;
        vec![from, (xr, from.1), (xr, yl), (xl, yl), (xl, to.1), to]
    };
    let mut new_routes: Vec<(RouteId, Vec<(f64, f64)>)> = Vec::new();
    for (r, &id) in ids.iter().enumerate() {
        let pts = route_of(st, id).to_vec();
        let mut out: Vec<(f64, f64)> = Vec::with_capacity(pts.len() + 6);
        for (k, &p) in pts.iter().enumerate() {
            if k > 0 {
                let q = pts[k - 1];
                let (ra, rb) = (row_of(q.0, &cuts), row_of(p.0, &cuts));
                if ra != rb {
                    let j = ra.min(rb);
                    let i = lane_of.get(&(r, j)).copied().unwrap_or(0);
                    let mut u = u_turn(j, q.1, i);
                    if ra > rb {
                        u.reverse();
                    }
                    out.extend(u);
                }
            }
            out.push(mv(p));
        }
        new_routes.push((id, simplify_collinear(out)));
    }
    for (id, pts) in new_routes {
        match id.1 {
            Some(k) => st.lane_routes[id.0][k] = pts,
            None => st.routes[id.0] = pts,
        }
    }
    // A laned edge's corridor still has to move with its rows.
    for e in 0..st.routes.len() {
        if st.lane_routes.get(e).is_some_and(|l| !l.is_empty()) {
            let pts: Vec<(f64, f64)> = st.routes[e].iter().map(|&p| mv(p)).collect();
            st.routes[e] = pts;
        }
    }
    for pts in st.loop_routes.values_mut() {
        // A loop moves with its node's row, taken from its first point.
        let Some(&first) = pts.first() else { continue };
        let (da, dc) = shifts[row_of(first.0, &cuts)];
        for p in pts.iter_mut() {
            *p = (p.0 + da, p.1 + dc);
        }
    }
    for a in st.loop_label_anchors.values_mut() {
        *a = mv(*a);
    }
    for j in st.junctions.iter_mut() {
        *j = mv(*j);
    }
    for nd in st.nodes.iter_mut() {
        let (da, dc) = shifts[row_of(nd.along, &cuts)];
        nd.along += da;
        nd.cross += dc;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use crate::{layout, GraphBuilder, LayoutOptions, Node, Size};

    /// A 16-box chain wraps into rows for a 16:9 viewport, fits it at a larger scale and keeps
    /// every segment axis-aligned.
    #[test]
    fn a_long_chain_wraps_into_rows_for_a_wide_viewport() {
        let mut b = GraphBuilder::new();
        let nodes: Vec<_> = (0..16)
            .map(|_| {
                b.add_node(Node {
                    size: Size::new(120.0, 52.0),
                    ..Node::default()
                })
            })
            .collect();
        for w in nodes.windows(2) {
            b.connect(w[0], w[1]);
        }
        let g = b.build();
        let plain = layout(&g, &LayoutOptions::layered_default()).unwrap();
        let mut o = LayoutOptions::layered_default();
        o.wrap.viewport_aspect = 16.0 / 9.0;
        let wrapped = layout(&g, &o).unwrap();

        let levels = |l: &crate::Layout| {
            let mut ys: Vec<i64> = l.nodes.iter().map(|p| p.center.y.round() as i64).collect();
            ys.sort_unstable();
            ys.dedup();
            ys.len()
        };
        assert_eq!(levels(&plain), 1);
        assert!(levels(&wrapped) >= 2, "{:?}", wrapped.nodes);
        let fit = |l: &crate::Layout| (16.0f64 / l.bounds.width()).min(9.0 / l.bounds.height());
        assert!(fit(&wrapped) > 1.2 * fit(&plain));
        for e in &wrapped.edges {
            for s in e.points.windows(2) {
                assert!(
                    (s[0].x - s[1].x).abs() < 0.5 || (s[0].y - s[1].y).abs() < 0.5,
                    "{:?}",
                    e.points
                );
            }
        }
    }
}

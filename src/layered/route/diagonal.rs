//! Flow (diagonal) style routing: ports where the centre ray leaves the node, radial port fans
//! on ellipses, and long edges drawn as a diagonal into the dummy lane, a straight run down it
//! and a diagonal out, the way ELK draws long edges.

use super::spread::PortItem;
use super::{PortMap, RouteCtx};
use crate::geometry::fp;
use crate::geometry::hit::seg_hits_box_pad;
use crate::geometry::polyline::simplify_collinear;
use crate::geometry::{Boxes, Side};
use crate::layered::ports::Port;
use crate::options::RoutingTuning;

/// Bucket key of a radial fan, after the four side keys of [`Side::index`]. A small circle has no
/// meaningful side, so flow edges on an ellipse share one bucket sorted by approach angle.
pub(super) const RADIAL: u8 = 4;

/// The border the centre -> `(ax, ay)` ray exits through. Top or bottom also needs the cross
/// offset to beat the layer offset, which keeps forward ports on wide, short nodes.
pub(super) fn ray_side(ctx: &RouteCtx, node: usize, ax: f64, ay: f64) -> Side {
    let (cx, cy, lw, ow) = (&ctx.b.cx, &ctx.b.cy, &ctx.b.layer_w, &ctx.b.order_w);
    let (dx, dy) = (ax - cx[node], ay - cy[node]);
    if dy.abs() * lw[node] <= dx.abs() * ow[node] || dy.abs() <= dx.abs() {
        if dx > 0.0 {
            Side::Right
        } else {
            Side::Left
        }
    } else if dy > 0.0 {
        Side::Bottom
    } else {
        Side::Top
    }
}

/// Preferred position on `side` of a port aiming at `(aim_cx, aim_cy)`. Flow edges use where the
/// centre -> aim line crosses the border. Orthogonal routing keeps `raw`.
pub(super) fn flow_pos(
    ctx: &RouteCtx,
    node: usize,
    side: Side,
    aim_cx: f64,
    aim_cy: f64,
    raw: f64,
) -> f64 {
    if !ctx.flow_edges {
        return raw;
    }
    let (cx, cy, lw, ow) = (&ctx.b.cx, &ctx.b.cy, &ctx.b.layer_w, &ctx.b.order_w);
    let (dx, dy) = (aim_cx - cx[node], aim_cy - cy[node]);
    if side.horizontal() {
        let hw = lw[node] / 2.0;
        if dx.abs() < 1e-6 {
            raw
        } else {
            cy[node] + dy * (hw / dx.abs())
        }
    } else {
        let hh = ow[node] / 2.0;
        if dy.abs() < 1e-6 {
            raw
        } else {
            cx[node] + dx * (hh / dy.abs())
        }
    }
}

/// Spread `list` (sorted by approach angle) around an ellipse. Cutting at the largest angular gap
/// keeps a cluster that straddles +/-pi contiguous.
pub(super) fn spread_radial(ctx: &RouteCtx, node: usize, list: &[PortItem], port: &mut PortMap) {
    use std::f64::consts::TAU;
    let (cx, cy, lw, ow) = (&ctx.b.cx, &ctx.b.cy, &ctx.b.layer_w, &ctx.b.order_w);
    let (min_port_gap, port_stroke_clear, stub_len) =
        (ctx.min_port_gap, ctx.port_stroke_clear, ctx.stub_len);
    let n = list.len();
    let (cxn, cyn) = (cx[node], cy[node]);
    let (hw, hh) = (lw[node] / 2.0, ow[node] / 2.0);
    let r_eff = (hw + hh) / 2.0;
    let mut cut = 0usize;
    let mut best_gap = -1.0;
    for i in 0..n {
        let a = list[i].sort;
        let b_ = list[(i + 1) % n].sort + if i + 1 == n { TAU } else { 0.0 };
        if b_ - a > best_gap {
            best_gap = b_ - a;
            cut = (i + 1) % n;
        }
    }
    let base = list[cut].sort;
    let unwrap_th = |th: f64| {
        let mut t = th - base;
        if t < -1e-9 {
            t += TAU;
        }
        t
    };
    let gap_lin = (0..n.saturating_sub(1)).fold(min_port_gap, |g, i| {
        g.max((list[i].thickness + list[i + 1].thickness) / 2.0 + port_stroke_clear)
    });
    let gap_ang = gap_lin / r_eff;
    let desired: Vec<f64> = (0..n)
        .map(|k| unwrap_th(list[(cut + k) % n].sort))
        .collect();
    // Cap the span just past the cluster and shrink the gap to fit, or ports wrap round the back.
    let d_span = desired[n - 1] - desired[0];
    let max_span = (d_span + 2.0 * gap_ang).min(TAU - gap_ang);
    let gap_eff = if n > 1 {
        gap_ang.min(max_span / (n as f64 - 1.0))
    } else {
        gap_ang
    };
    let lo_b = desired[0] - (max_span - d_span) / 2.0;
    let hi_b = lo_b + max_span;
    let mut pos = desired.clone();
    pos[0] = pos[0].max(lo_b);
    for i in 1..n {
        if pos[i] < pos[i - 1] + gap_eff {
            pos[i] = pos[i - 1] + gap_eff;
        }
    }
    if n > 1 && pos[n - 1] > hi_b {
        pos[n - 1] = hi_b;
        for i in (0..n - 1).rev() {
            pos[i] = pos[i].min(pos[i + 1] - gap_eff);
        }
    }
    for (k, p) in pos.iter().enumerate() {
        let it = &list[(cut + k) % n];
        let th = base + p;
        let (dc, ds) = (fp::cos(th), fp::sin(th));
        let scale = 1.0 / (fp::sq(dc / hw) + fp::sq(ds / hh)).sqrt();
        let anchor = (cxn + scale * dc, cyn + scale * ds);
        let stub = (anchor.0 + stub_len * dc, anchor.1 + stub_len * ds);
        let side = if ds.abs() * lw[node] <= dc.abs() * ow[node] {
            if dc > 0.0 {
                Side::Right
            } else {
                Side::Left
            }
        } else if ds > 0.0 {
            Side::Bottom
        } else {
            Side::Top
        };
        port.insert((it.arc, it.is_source), Port { anchor, stub, side });
    }
}

/// Fixes one end diagonal of a flow route. Returns `Some(vec![])` if `p -> q` clears every box,
/// `Some(vec![bend])` for the cheapest one-corner detour, and `None` if no detour clears.
pub(super) fn flow_diag_fix(
    p: (f64, f64),
    q: (f64, f64),
    b: &Boxes,
    skip1: usize,
    skip2: usize,
    r: &RoutingTuning,
) -> Option<Vec<(f64, f64)>> {
    // A diagonal that grazes a box reads as badly as one that hits it, so demand a real gap.
    let (clear, margin) = (r.diagonal_clear, r.diagonal_detour_margin);
    if !seg_hits_box_pad(p, q, b, skip1, skip2, clear) {
        return Some(vec![]);
    }
    let dist = |a: (f64, f64), c: (f64, f64)| fp::hypot(a.0 - c.0, a.1 - c.1);
    let mut best: Option<((f64, f64), f64)> = None;
    for i in 0..b.n_real {
        if i == skip1 || i == skip2 || b.layer_w[i] <= 0.0 {
            continue;
        }
        let hx = b.layer_w[i] / 2.0 + margin;
        let hy = b.order_w[i] / 2.0 + margin;
        for (sx, sy) in [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
            let c = (b.cx[i] + sx * hx, b.cy[i] + sy * hy);
            if seg_hits_box_pad(p, c, b, skip1, skip2, clear)
                || seg_hits_box_pad(c, q, b, skip1, skip2, clear)
            {
                continue;
            }
            let extra = dist(p, c) + dist(c, q) - dist(p, q);
            if best.is_none_or(|(_, e)| extra < e) {
                best = Some((c, extra));
            }
        }
    }
    best.map(|(c, _)| vec![c])
}

/// Flow drawing of a long edge along spine `v`, or `None` if no variant clears every box.
pub(super) fn long_route(
    ctx: &RouteCtx,
    arc: usize,
    chain: &[usize],
    (ps, pt): (Port, Port),
    v: &[(f64, f64)],
    side_port_used: &mut [bool],
) -> Option<Vec<(f64, f64)>> {
    let (b, o) = (ctx.b, ctx.o);
    let m = chain.len();
    let (lo, hi) = (chain[0], chain[m - 1]);
    // With side ports, run the lane on the target port's own line so the route ends straight.
    let aligned = if ctx.side_port && pt.side.horizontal() {
        let lane_c = pt.anchor.1;
        let cand = simplify_collinear(vec![
            ps.anchor,
            (v[1].0, lane_c),
            (v[m - 2].0, lane_c),
            pt.anchor,
        ]);
        (!ctx.poly_hits(&cand, lo, hi)).then_some(cand)
    } else {
        None
    };
    if let Some(cand) = aligned {
        if let Some(f) = side_port_used.get_mut(arc) {
            *f = true;
        }
        return Some(cand);
    }
    let head = flow_diag_fix(ps.anchor, v[1], b, lo, hi, &o.tuning.routing);
    let tail = flow_diag_fix(v[m - 2], pt.anchor, b, lo, hi, &o.tuning.routing);
    let (Some(hd), Some(tl)) = (head, tail) else {
        return None;
    };
    let mut pts = vec![ps.anchor];
    pts.extend(hd);
    pts.push(v[1]);
    pts.push(v[m - 2]);
    pts.extend(tl);
    pts.push(pt.anchor);
    let cand = simplify_collinear(pts);
    (!ctx.poly_hits(&cand, lo, hi)).then_some(cand)
}

//! Port spreading picks where along its border each arc end attaches.
//!
//! Endpoints are bucketed per (node, border) and ordered by where each edge heads. A bundle enters
//! its bucket once per lane, with one sort key per arc end so its lanes stay in corridor order.

use crate::geometry::fp;
use std::collections::BTreeMap;

use super::diagonal::{self, RADIAL};
use super::lanes::lane_dir;
use super::{ArcEnds, PortMap, RouteCtx, PORT_SPAN};
use crate::geometry::Side;
use crate::layered::ports::project_port;
use crate::sort::SortDyn;

/// Fraction of an end's offset from the border's middle that survives on a shared border, so
/// divergent edges do not park at the corners.
const CENTRE_PULL: f64 = 0.7;

/// Share of the port gap and stroke clearance a neighbour gives up so an end that continues its
/// row keeps its line. A jog of a few pixels at a port reads worse than two arcs a little closer
/// together.
const TIGHT_PORT_SHARE: f64 = 0.5;

/// One endpoint queueing for a port on a (node, border) pair. A bundle end queues once per lane.
#[derive(Clone, Copy)]
pub(super) struct PortItem {
    pub(super) arc: usize,
    pub(super) is_source: bool,
    /// Lane index in corridor order, `0` for a plain arc.
    lane: u32,
    /// Lane order along the border, which is the lane index reversed where [`lane_dir`] is
    /// negative.
    lane_rank: i32,
    /// Ordering key among the other ends on this border.
    pub(super) sort: f64,
    pos: f64,
    pub(super) thickness: f64,
    /// Offset from the border's middle of an offset-hinted end.
    fixed: Option<f64>,
}

/// Per bundle end, each lane's signed distance right of travel from the port, in corridor order.
pub(super) type LaneOffsets = BTreeMap<(usize, bool), Vec<f64>>;

/// Ordering key on a perpendicular border that nests the L's so the fan is planar.
fn perp_key(side: Side, is_source: bool, aim_order: f64) -> f64 {
    if is_source == (side == Side::Bottom) {
        -aim_order
    } else {
        aim_order
    }
}

pub(super) fn spread_ports(ctx: &RouteCtx, arc_ends: &ArcEnds) -> (PortMap, LaneOffsets) {
    let (b, stub_len) = (ctx.b, ctx.stub_len);
    let (cx, cy) = (&b.cx, &b.cy);
    let mut buckets: BTreeMap<(usize, u8), Vec<PortItem>> = BTreeMap::new();
    // A hinted end takes the border its hint names, so it leaves an ellipse's radial fan.
    let radial = |arc: usize, is_source: bool, node: usize| {
        ctx.radial(node) && ctx.hints.get(arc, is_source).is_none()
    };
    for (&arc, (s, t)) in arc_ends.iter() {
        let (s_sort, s_pos) = if radial(arc, true, s.node) {
            let th = fp::atan2(s.aim_order - cy[s.node], s.aim_layer - cx[s.node]);
            (th, th)
        } else if s.side.horizontal() {
            let p = diagonal::flow_pos(ctx, s.node, s.side, s.aim_layer, s.aim_order, s.aim_order);
            (p, p)
        } else {
            let p = diagonal::flow_pos(ctx, s.node, s.side, s.aim_layer, s.aim_order, s.aim_layer);
            (perp_key(s.side, true, s.aim_order), p)
        };
        let (t_sort, t_pos) = if radial(arc, false, t.node) {
            let th = fp::atan2(t.aim_order - cy[t.node], t.aim_layer - cx[t.node]);
            (th, th)
        } else if t.side.horizontal() {
            let p = diagonal::flow_pos(ctx, t.node, t.side, t.aim_layer, t.aim_order, t.aim_order);
            (p, p)
        } else {
            let p = diagonal::flow_pos(ctx, t.node, t.side, t.aim_layer, t.aim_order, t.aim_layer);
            (perp_key(t.side, false, t.aim_order), p)
        };
        let s_key = if radial(arc, true, s.node) {
            RADIAL
        } else {
            s.side.index()
        };
        let t_key = if radial(arc, false, t.node) {
            RADIAL
        } else {
            t.side.index()
        };
        let strokes = ctx.lane_strokes(arc);
        for (is_source, node, key, side, sort, pos) in [
            (true, s.node, s_key, s.side, s_sort, s_pos),
            (false, t.node, t_key, t.side, t_sort, t_pos),
        ] {
            let bucket = buckets.entry((node, key)).or_default();
            if strokes.is_empty() {
                bucket.push(PortItem {
                    arc,
                    is_source,
                    lane: 0,
                    lane_rank: 0,
                    sort,
                    pos,
                    thickness: ctx.thickness(arc),
                    fixed: ctx.hints.get(arc, is_source).and_then(|h| h.offset),
                });
                continue;
            }
            let dir = lane_dir(side, is_source);
            for (lane, &stroke) in strokes.iter().enumerate() {
                bucket.push(PortItem {
                    arc,
                    is_source,
                    lane: lane as u32,
                    lane_rank: (lane as f64 * dir) as i32,
                    sort,
                    pos,
                    thickness: stroke,
                    fixed: None,
                });
            }
        }
    }
    let mut port: PortMap = BTreeMap::new();
    let mut lane_offsets: LaneOffsets = BTreeMap::new();
    let mut bucket_keys: Vec<(usize, u8)> = buckets.keys().copied().collect();
    bucket_keys.sort_unstable_by_dyn(Ord::cmp);
    for key in bucket_keys {
        let side = match key.1 {
            0 => Side::Right,
            1 => Side::Left,
            2 => Side::Top,
            _ => Side::Bottom,
        };
        let node = key.0;
        // `bucket_keys` is `buckets.keys()` and each key is removed exactly once.
        let Some(mut list) = buckets.remove(&key) else {
            continue;
        };
        // Orthogonal ports on a perpendicular border run along the layer axis after their bend,
        // so order them by aim layer position first. Lane rank last keeps a bundle together.
        let by_lane =
            |a: &PortItem, b: &PortItem| a.arc.cmp(&b.arc).then(a.lane_rank.cmp(&b.lane_rank));
        if !ctx.flow_edges && !side.horizontal() {
            list.sort_unstable_by_dyn(|a, b| {
                a.pos
                    .total_cmp(&b.pos)
                    .then(a.sort.total_cmp(&b.sort))
                    .then(by_lane(a, b))
            });
        } else {
            list.sort_unstable_by_dyn(|a, b| a.sort.total_cmp(&b.sort).then(by_lane(a, b)));
        }
        if key.1 == RADIAL {
            diagonal::spread_radial(ctx, node, &list, &mut port);
            continue;
        }
        // With a pinned port, order by intended position so free ends spread round it.
        if list.iter().any(|it| it.fixed.is_some()) {
            let centre = if side.horizontal() {
                cy[node]
            } else {
                cx[node]
            };
            let at = |it: &PortItem| it.fixed.unwrap_or(it.pos - centre);
            // Ties keep the order the keys above gave them.
            let mut order: Vec<usize> = (0..list.len()).collect();
            order.sort_unstable_by_dyn(|&a, &b| {
                at(&list[a]).total_cmp(&at(&list[b])).then(a.cmp(&b))
            });
            list = order.iter().map(|&i| list[i]).collect();
        }
        let offs = border_offsets(ctx, node, side, &list);
        // A bundle end's port is the middle of its lanes, since the corridor routes through it.
        // The lanes keep their offsets from it, right of travel.
        let mut grouped: BTreeMap<(usize, bool), Vec<(u32, f64)>> = BTreeMap::new();
        for (it, &off) in list.iter().zip(&offs) {
            grouped
                .entry((it.arc, it.is_source))
                .or_default()
                .push((it.lane, off));
        }
        for ((arc, is_source), mut group) in grouped {
            group.sort_unstable_by_key_dyn(|&(lane, _)| lane);
            let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
            for &(_, off) in &group {
                lo = lo.min(off);
                hi = hi.max(off);
            }
            let middle = (lo + hi) / 2.0;
            port.insert(
                (arc, is_source),
                project_port(node, side, middle, b, stub_len),
            );
            if group.len() > 1 {
                let dir = lane_dir(side, is_source);
                lane_offsets.insert(
                    (arc, is_source),
                    group.iter().map(|&(_, off)| dir * (off - middle)).collect(),
                );
            }
        }
    }
    (port, lane_offsets)
}

/// Offset from the border's middle of every entry of one sorted bucket.
fn border_offsets(ctx: &RouteCtx, node: usize, side: Side, list: &[PortItem]) -> Vec<f64> {
    let b = ctx.b;
    let (cx, cy, lw, ow) = (&b.cx, &b.cy, &b.layer_w, &b.order_w);
    let min_port_gap = ctx.min_port_gap;
    let n = list.len();
    let (cxn, cyn) = (cx[node], cy[node]);
    let extent = if side.horizontal() {
        ow[node]
    } else {
        lw[node]
    };
    let centre = if side.horizontal() { cyn } else { cxn };
    // Two thick strokes at the plain port gap leave almost no space between them.
    let pair_gap = |a: usize, b: usize| {
        min_port_gap.max((list[a].thickness + list[b].thickness) / 2.0 + ctx.port_stroke_clear)
    };
    let fixed: Vec<Option<f64>> = list.iter().map(|it| it.fixed).collect();
    if n == 1 {
        if let Some(f) = fixed[0] {
            return vec![f];
        }
        return if ctx.flow_edges {
            let lim = extent * PORT_SPAN;
            vec![(list[0].pos - centre).clamp(-lim, lim)]
        } else {
            vec![0.0]
        };
    }
    let lim = extent * PORT_SPAN;
    let desired: Vec<f64> = list
        .iter()
        .map(|it| (it.pos - centre).clamp(-lim, lim))
        .collect();
    // An oversubscribed ellipse spaces by distance along the curve, so outer ports pack tighter.
    let oversubscribed = (2.0 * lim / (n - 1) as f64) < min_port_gap;
    if b.ellipse[node] && oversubscribed {
        let perp_half = (if side.horizontal() {
            lw[node]
        } else {
            ow[node]
        }) / 2.0;
        let gap_req = (1..n).fold(min_port_gap, |g, i| g.max(pair_gap(i - 1, i)));
        let mut offs = ellipse_offsets(&desired, lim, extent / 2.0, perp_half, gap_req);
        for (off, f) in offs.iter_mut().zip(&fixed) {
            if let Some(f) = *f {
                *off = f;
            }
        }
        offs
    } else {
        let gaps: Vec<f64> = (1..n).map(|i| pair_gap(i - 1, i)).collect();
        let floors: Vec<f64> = (1..n)
            .map(|i| {
                let strokes = (list[i - 1].thickness + list[i].thickness) / 2.0;
                (min_port_gap * TIGHT_PORT_SHARE)
                    .max(strokes + ctx.port_stroke_clear * TIGHT_PORT_SHARE)
            })
            .collect();
        let raw: Vec<f64> = list.iter().map(|it| it.pos - centre).collect();
        box_offsets(&desired, &raw, lim, gaps, &floors, &fixed)
    }
}

/// Offsets for an oversubscribed ellipse border. Pushes `desired` apart to `gap_req` Euclidean
/// distance on the curve, or to the largest gap that fits.
fn ellipse_offsets(
    desired: &[f64],
    lim: f64,
    half_along: f64,
    perp_half: f64,
    gap_req: f64,
) -> Vec<f64> {
    let n = desired.len();
    let span = |o: f64| ((1.0 - fp::sq(o / half_along)).max(0.0)).sqrt();
    let euclid = |a: f64, b: f64| fp::hypot(perp_half * (span(a) - span(b)), a - b);
    // Smallest `o >= prev` with `euclid(prev, o) >= gap`. The binary search runs over the
    // physical half-extent so an overflow past `lim` shows up and can be pushed back.
    let next_off = |prev: f64, gap: f64| -> f64 {
        if euclid(prev, half_along) < gap {
            return half_along;
        }
        let (mut lo, mut hi) = (prev, half_along);
        for _ in 0..40 {
            let mid = (lo + hi) / 2.0;
            if euclid(prev, mid) < gap {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        hi
    };
    let prev_off = |next: f64, gap: f64| -> f64 {
        if euclid(next, -half_along) < gap {
            return -half_along;
        }
        let (mut lo, mut hi) = (-half_along, next);
        for _ in 0..40 {
            let mid = (lo + hi) / 2.0;
            if euclid(next, mid) < gap {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        lo
    };
    let place = |gap: f64| -> Option<Vec<f64>> {
        let mut pos = desired.to_vec();
        pos[0] = pos[0].max(-lim);
        for i in 1..n {
            if pos[i] <= pos[i - 1] || euclid(pos[i - 1], pos[i]) < gap {
                pos[i] = next_off(pos[i - 1], gap);
            }
        }
        if pos[n - 1] > lim + 1e-6 {
            pos[n - 1] = lim;
            for i in (0..n - 1).rev() {
                pos[i] = pos[i].min(prev_off(pos[i + 1], gap));
            }
            if pos[0] < -lim - 1e-6 {
                return None;
            }
        }
        Some(pos)
    };
    place(gap_req).unwrap_or_else(|| {
        let (mut lo, mut hi) = (0.0, gap_req);
        // A zero gap never overflows, so the fallback is unreachable.
        let mut best = place(0.0).unwrap_or_else(|| desired.to_vec());
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            match place(mid) {
                Some(p) => {
                    best = p;
                    lo = mid;
                }
                None => {
                    hi = mid;
                }
            }
        }
        best
    })
}

/// Offsets for a box border, or an ellipse border with room. Aims are pulled by [`CENTRE_PULL`]
/// and kept `gaps` apart, then ends that could arrive straight return to their own line where
/// neighbours can close up to `floors`. Entries must be sorted by where they will sit.
fn box_offsets(
    desired: &[f64],
    raw: &[f64],
    lim: f64,
    mut gaps: Vec<f64>,
    floors: &[f64],
    fixed: &[Option<f64>],
) -> Vec<f64> {
    let n = desired.len();
    let total: f64 = gaps.iter().sum();
    if total > 2.0 * lim && total > 0.0 {
        let scale = 2.0 * lim / total;
        for gv in &mut gaps {
            *gv *= scale;
        }
    }
    let floors: Vec<f64> = floors.iter().zip(&gaps).map(|(f, g)| f.min(*g)).collect();
    let mut pos: Vec<f64> = desired
        .iter()
        .map(|d| (d * CENTRE_PULL).clamp(-lim, lim))
        .collect();
    pos[0] = pos[0].max(-lim);
    for i in 1..n {
        pos[i] = pos[i].max(pos[i - 1] + gaps[i - 1]);
    }
    if pos[n - 1] > lim {
        pos[n - 1] = lim;
        for i in (0..n - 1).rev() {
            pos[i] = pos[i].min(pos[i + 1] - gaps[i]);
        }
    }
    // Ports nearest the centre go first, so the arc that continues a row wins the space.
    let mut held = vec![false; n];
    if fixed.iter().any(Option::is_some) {
        pin_fixed(&mut pos, &gaps, lim, fixed);
        for (h, f) in held.iter_mut().zip(fixed) {
            *h = f.is_some();
        }
    }
    let mut order: Vec<usize> = (0..n)
        .filter(|&i| raw[i].abs() <= lim && !held[i])
        .collect();
    order.sort_unstable_by_dyn(|&a, &b| raw[a].abs().total_cmp(&raw[b].abs()).then(a.cmp(&b)));
    for i in order {
        let mut trial = pos.clone();
        trial[i] = raw[i];
        // A pinned port may sit beyond the span, so only free ends are held to it.
        let ok = settle_side(
            &mut trial,
            i,
            true,
            &gaps,
            &floors,
            &held,
            fixed[n - 1].is_some(),
            lim,
        ) && settle_side(
            &mut trial,
            i,
            false,
            &gaps,
            &floors,
            &held,
            fixed[0].is_some(),
            lim,
        );
        if ok {
            pos = trial;
            held[i] = true;
        }
    }
    pos
}

/// Makes room for `trial[i]` on one side of it. Neighbours move out to `gaps`, and where that runs
/// past the span they come back in from its end, closing up to `floors`. Returns `false` if a held
/// entry would have to move or the floors do not fit either.
#[allow(clippy::too_many_arguments)]
fn settle_side(
    trial: &mut [f64],
    i: usize,
    up: bool,
    gaps: &[f64],
    floors: &[f64],
    held: &[bool],
    end_pinned: bool,
    lim: f64,
) -> bool {
    let n = trial.len();
    let (side, end): (Vec<usize>, usize) = if up {
        ((i + 1..n).collect(), n - 1)
    } else {
        ((0..i).rev().collect(), 0)
    };
    if side.is_empty() {
        return true;
    }
    // Signed so one body serves both directions. `s * a > s * b` means a is further out than b.
    let s = if up { 1.0 } else { -1.0 };
    let gap_at = |j: usize| if up { gaps[j - 1] } else { gaps[j] };
    let floor_at = |j: usize| if up { floors[j - 1] } else { floors[j] };
    let prev = |j: usize| if up { j - 1 } else { j + 1 };
    for &j in &side {
        let need = trial[prev(j)] + s * gap_at(j);
        if s * trial[j] < s * need {
            if held[j] {
                return false;
            }
            trial[j] = need;
        }
    }
    if end_pinned || s * trial[end] <= lim + 1e-6 {
        return true;
    }
    if held[end] {
        return false;
    }
    trial[end] = s * lim;
    // Pull back from the end so each entry sits at most a floor inside its outer neighbour.
    let mut outer = end;
    for &j in side.iter().rev().skip(1) {
        let cap = trial[outer] - s * floor_at(outer);
        if s * trial[j] > s * cap {
            if held[j] {
                return false;
            }
            trial[j] = cap;
        }
        outer = j;
    }
    let first = side[0];
    s * (trial[first] - trial[i]) >= floor_at(first) - 1e-6
}

/// Put pinned entries at their `fixed` offset and fit the free runs into the room between them
/// and `-lim..lim`, scaling `gaps` down where they do not fit.
fn pin_fixed(pos: &mut [f64], gaps: &[f64], lim: f64, fixed: &[Option<f64>]) {
    let n = pos.len();
    for (p, f) in pos.iter_mut().zip(fixed) {
        if let Some(f) = *f {
            *p = f;
        }
    }
    let mut start = 0;
    while start < n {
        if fixed[start].is_some() {
            start += 1;
            continue;
        }
        let mut end = start;
        while end < n && fixed[end].is_none() {
            end += 1;
        }
        let lo = if start > 0 {
            pos[start - 1] + gaps[start - 1]
        } else {
            -lim
        };
        let hi = if end < n {
            pos[end] - gaps[end - 1]
        } else {
            lim
        };
        let inner = &gaps[start..end - 1];
        let run = &mut pos[start..end];
        if hi < lo {
            let mid = (lo + hi) / 2.0;
            run.iter_mut().for_each(|p| *p = mid);
        } else {
            let total: f64 = inner.iter().sum();
            let scale = if total > hi - lo && total > 0.0 {
                (hi - lo) / total
            } else {
                1.0
            };
            let k = run.len();
            run[0] = run[0].clamp(lo, hi);
            for i in 1..k {
                run[i] = run[i].max(run[i - 1] + inner[i - 1] * scale);
            }
            if run[k - 1] > hi {
                run[k - 1] = hi;
                for i in (0..k - 1).rev() {
                    run[i] = run[i].min(run[i + 1] - inner[i] * scale);
                }
            }
        }
        start = end;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Pinned entries keep their offset, even beyond the span.
    #[test]
    fn a_pinned_port_keeps_its_offset_and_the_rest_make_room() {
        let fixed = [None, Some(5.0), None, Some(44.0)];
        let desired = [-2.0, 0.0, 8.0, 30.0];
        let pos = box_offsets(&desired, &desired, 40.0, vec![18.0; 3], &[9.0; 3], &fixed);
        assert_eq!(pos[1], 5.0);
        assert_eq!(pos[3], 44.0);
        for w in pos.windows(2) {
            assert!(w[1] - w[0] >= 18.0 - 1e-9, "{pos:?}");
        }
        assert!(pos[0] >= -40.0, "{pos:?}");
    }

    /// Two ends on a 36px border, one continuing its row and one heading far down. The straight
    /// one keeps its line and the other closes up to the floor.
    #[test]
    fn a_straight_end_keeps_its_line_and_the_neighbour_closes_up() {
        let desired = [0.0, 16.2];
        let pos = box_offsets(&desired, &[0.0, 48.0], 16.2, vec![18.0], &[9.0], &[None; 2]);
        assert_eq!(pos[0], 0.0, "{pos:?}");
        assert!(pos[1] >= 9.0 && pos[1] <= 16.2 + 1e-9, "{pos:?}");
        // With room for the full gap nothing closes up.
        let roomy = box_offsets(&desired, &[0.0, 48.0], 40.0, vec![18.0], &[9.0], &[None; 2]);
        assert_eq!(roomy[0], 0.0);
        assert!(roomy[1] >= 18.0 - 1e-9, "{roomy:?}");
    }

    #[test]
    fn without_a_pin_the_spreading_is_unchanged() {
        let desired = [-2.0, 0.0, 8.0, 30.0];
        let pinned = box_offsets(
            &desired,
            &desired,
            40.0,
            vec![18.0; 3],
            &[9.0; 3],
            &[None; 4],
        );
        for w in pinned.windows(2) {
            assert!(w[1] - w[0] >= 18.0 - 1e-9, "{pinned:?}");
        }
        assert!(pinned.iter().all(|p| p.abs() <= 40.0 + 1e-6), "{pinned:?}");
    }

    #[test]
    fn a_squeezed_free_run_stays_between_its_pins() {
        let mut pos = vec![-30.0, 0.0, 1.0, 10.0];
        pin_fixed(
            &mut pos,
            &[18.0; 3],
            40.0,
            &[Some(-30.0), None, None, Some(10.0)],
        );
        assert_eq!((pos[0], pos[3]), (-30.0, 10.0));
        assert!(
            pos[1] > -30.0 && pos[2] < 10.0 && pos[1] <= pos[2],
            "{pos:?}"
        );
    }
}

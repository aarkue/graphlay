//! Repairs on the first drawing. Port jogs are snapped, short steps flattened, parallel bundle
//! channels nested and endpoints settled back on their outlines. In diagonal style, side-port
//! edges that came out crossing can take the plain-port route instead.

use crate::geometry::fp;
use std::collections::BTreeMap;

use super::geom::{border_reach, interference};
use super::lanes::tight_lane_pattern;
use super::{route_edges, PortMap, RouteCtx};
use crate::geometry::hit::{arc_crossings, seg_hits_box, seg_hits_box_pad, segments_cross};
use crate::geometry::polyline::simplify_collinear;
use crate::geometry::shape::{
    border_toward, leaves_inward, outline_distance, project_onto_outline,
};
use crate::geometry::Boxes;
use crate::layered::state::LayeredState;
use crate::layered::steps::flatten_short_steps;
use crate::options::{LayoutOptions, RoutingTuning};
use crate::sort::SortDyn;

/// Matches the `endpoint_off_outline` metric's tolerance.
const ENDPOINT_OUTLINE_TOL: f64 = 0.5;

/// Per arc, snap tiny endpoint jogs, then flatten short steps where every port stays on its
/// border, clear of its neighbours, and interference does not grow. Hinted ends stay within
/// their hints.
pub(super) fn flatten_steps(ctx: &RouteCtx, port: &PortMap, routes: &mut [Vec<(f64, f64)>]) {
    let (b, o) = (ctx.b, ctx.o);
    let mut anchors_of: BTreeMap<usize, Vec<(f64, f64)>> = BTreeMap::new();
    for (arc, chain) in ctx.ordered() {
        if chain.len() < 2 {
            continue;
        }
        anchors_of
            .entry(chain[0])
            .or_default()
            .push(port[&(arc, true)].anchor);
        anchors_of
            .entry(chain[chain.len() - 1])
            .or_default()
            .push(port[&(arc, false)].anchor);
    }
    let dist = |a: (f64, f64), b: (f64, f64)| fp::hypot(a.0 - b.0, a.1 - b.1);
    for (arc, chain) in ctx.ordered() {
        if chain.len() < 2 {
            continue;
        }
        let (hy, ty) = (
            port[&(arc, true)].side.horizontal(),
            port[&(arc, false)].side.horizontal(),
        );
        let hints = &ctx.hints;
        let before = hints.hinted(arc).then(|| routes[arc].clone());
        snap_endpoint_jogs(
            &mut routes[arc],
            (!hints.pinned(arc, true)).then_some(hy),
            (!hints.pinned(arc, false)).then_some(ty),
            &o.tuning.routing,
        );
        // A snap can slide a hinted end past a corner, and settling an end that left its outline
        // would tilt the hinted one, so undo it in either case.
        if let Some(before) = before {
            let r = &routes[arc];
            let on_outline = |p: (f64, f64), node: usize| {
                b.outline(node)
                    .is_none_or(|ol| outline_distance(p, ol) <= ENDPOINT_OUTLINE_TOL)
            };
            let kept = r.len() >= 2
                && hints.on_border(arc, true, r[0], b)
                && hints.on_border(arc, false, r[r.len() - 1], b)
                && on_outline(r[0], chain[0])
                && on_outline(r[r.len() - 1], chain[chain.len() - 1])
                && hints.keeps_ends_straight(arc, &before, r, b);
            if !kept {
                routes[arc] = before;
            }
        }
        let (lo, hi) = (chain[0], chain[chain.len() - 1]);
        let slide_ok = |head: bool, new: (f64, f64)| {
            if hints.pinned(arc, head) || !hints.on_border(arc, head, new, b) {
                return false;
            }
            let (node, old, along_y) = if head {
                (lo, port[&(arc, true)].anchor, hy)
            } else {
                (hi, port[&(arc, false)].anchor, ty)
            };
            // A merged step can ask for a slide taller than the node, past the corner.
            let on_border = if along_y {
                (new.1 - b.cy[node]).abs() <= border_reach(b.order_w[node], b.ellipse[node])
            } else {
                (new.0 - b.cx[node]).abs() <= border_reach(b.layer_w[node], b.ellipse[node])
            };
            on_border
                && anchors_of[&node]
                    .iter()
                    .filter(|&&a| a != old)
                    .all(|&a| dist(a, new) >= o.spacing.port_gap.min(dist(a, old)))
        };
        // Taken out so `routes` can be borrowed as the others while this one is mutated.
        let mut mine = std::mem::take(&mut routes[arc]);
        {
            let others = &*routes;
            let accept = |old: &[(f64, f64)], new: &[(f64, f64)]| {
                interference(new, others, o.spacing.port_gap)
                    <= interference(old, others, o.spacing.port_gap)
                    && (!hints.hinted(arc) || hints.keeps_ends_straight(arc, old, new, b))
            };
            // Lanes are drawn off this corridor later, so it must clear nodes by the fan's half
            // width.
            let fan = tight_lane_pattern(ctx.lane_strokes(arc), o.spacing.bundle_lane_gap)
                .iter()
                .zip(ctx.lane_strokes(arc))
                .map(|(p, s)| p.abs() + s / 2.0)
                .fold(-1.0, f64::max);
            flatten_short_steps(
                &mut mine,
                hy,
                ty,
                o.tuning.routing.step_flatten,
                slide_ok,
                |p, q| seg_hits_box_pad(p, q, b, lo, hi, fan) || hints.seg_cuts_own(arc, p, q, b),
                accept,
            );
        }
        routes[arc] = mine;
    }
}

/// Slide an endpoint's anchor and stub along its border onto the edge's main track.
/// `head_along_y` and `tail_along_y` give the border's axis, and `None` leaves that end alone.
pub(super) fn snap_endpoint_jogs(
    route: &mut Vec<(f64, f64)>,
    head_along_y: Option<bool>,
    tail_along_y: Option<bool>,
    r: &RoutingTuning,
) {
    let (snap, min_track) = (r.endpoint_snap, r.endpoint_snap_min_track);
    if route.len() < 3 {
        return;
    }
    // The track is the first segment from that end at least `min_track` long.
    let snap_one = |route: &mut [(f64, f64)], head: bool, along_y: bool| {
        let n = route.len();
        let anchor = if head { route[0] } else { route[n - 1] };
        let ks: Vec<usize> = if head {
            (0..n - 1).collect()
        } else {
            (1..n).rev().collect()
        };
        for k in ks {
            let (p, q) = if head {
                (route[k], route[k + 1])
            } else {
                (route[k - 1], route[k])
            };
            if fp::hypot(p.0 - q.0, p.1 - q.1) < min_track {
                continue;
            }
            let range = if head { 0..=k } else { k..=n - 1 };
            // Only slide along the endpoint's border.
            if along_y
                && (p.1 - q.1).abs() < 0.5
                && (anchor.1 - p.1).abs() > 0.5
                && (anchor.1 - p.1).abs() < snap
            {
                for pt in &mut route[range] {
                    pt.1 = p.1;
                }
            } else if !along_y
                && (p.0 - q.0).abs() < 0.5
                && (anchor.0 - p.0).abs() > 0.5
                && (anchor.0 - p.0).abs() < snap
            {
                for pt in &mut route[range] {
                    pt.0 = p.0;
                }
            }
            break;
        }
    };
    if let Some(along_y) = head_along_y {
        snap_one(route, true, along_y);
    }
    if let Some(along_y) = tail_along_y {
        snap_one(route, false, along_y);
    }
    *route = simplify_collinear(std::mem::take(route));
}

/// Uncross parallel long edges at their shared target by making the outer channel turn after
/// the inner one. Arcs where `keep(arc)` holds take no part, since the slide can move a port.
pub(super) fn nest_parallel_bundle_channels(
    routes: &mut [Vec<(f64, f64)>],
    chains: &BTreeMap<usize, Vec<usize>>,
    cy: &[f64],
    nest_gap: f64,
    keep: impl Fn(usize) -> bool,
) {
    // The channel is the longest run along the layer axis. Returns its downstream corner index,
    // the channel's order coord and that corner's layer coord.
    let channel = |r: &[(f64, f64)]| -> Option<(usize, f64, f64)> {
        let mut best: Option<(usize, f64)> = None; // (corner index, run length)
        for i in 0..r.len().saturating_sub(1) {
            let (dx, dy) = ((r[i + 1].0 - r[i].0).abs(), (r[i + 1].1 - r[i].1).abs());
            if dy < 1.0 && dx > best.map_or(1.0, |(_, l)| l) {
                let corner = if r[i + 1].0 >= r[i].0 { i + 1 } else { i };
                best = Some((corner, dx));
            }
        }
        best.map(|(c, _)| (c, r[c].1, r[c].0))
    };
    let crosses = |a: &[(f64, f64)], b: &[(f64, f64)]| -> bool {
        for wa in a.windows(2) {
            for wb in b.windows(2) {
                let (p1, p2, p3, p4) = (wa[0], wa[1], wb[0], wb[1]);
                if p1 == p3 || p1 == p4 || p2 == p3 || p2 == p4 {
                    continue;
                }
                if segments_cross(p1, p2, p3, p4) {
                    return true;
                }
            }
        }
        false
    };
    let mut bundles: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for (&arc, c) in chains {
        if c.len() > 2 && !keep(arc) {
            let (lo, hi) = (c[0], c[c.len() - 1]);
            bundles
                .entry((lo.min(hi), lo.max(hi)))
                .or_default()
                .push(arc);
        }
    }
    for (&(_, hi), arcs) in bundles.iter().filter(|(_, v)| v.len() >= 2) {
        let target_order = cy[hi];
        let mut sorted = arcs.clone();
        sorted.sort_unstable_by_dyn(Ord::cmp);
        for i in 0..sorted.len() {
            for j in i + 1..sorted.len() {
                let (a, b) = (sorted[i], sorted[j]);
                if !crosses(&routes[a], &routes[b]) {
                    continue;
                }
                let (Some((ca, ord_a, endx_a)), Some((cb, ord_b, endx_b))) =
                    (channel(&routes[a]), channel(&routes[b]))
                else {
                    continue;
                };
                // The outer channel is the one farther from the target, and must turn last.
                let (outer, oc, oend, iend) =
                    if (ord_a - target_order).abs() >= (ord_b - target_order).abs() {
                        (a, ca, endx_a, endx_b)
                    } else {
                        (b, cb, endx_b, endx_a)
                    };
                if oend >= iend {
                    continue; // already nested
                }
                let target_x = routes[outer].last().map_or(oend, |p| p.0);
                let new_x = (iend + nest_gap).min(target_x - 1.0);
                if new_x <= oend {
                    continue;
                }
                let old = routes[outer][oc].0;
                for p in routes[outer].iter_mut().skip(oc) {
                    if (p.0 - old).abs() < 0.5 {
                        p.0 = new_x;
                    } else {
                        break;
                    }
                }
            }
        }
    }
}

/// Put endpoints that earlier repairs slid off their node's outline back on it, projecting along
/// the route's last segment, else the centre ray.
pub(super) fn settle_endpoints_on_outline(
    routes: &mut [Vec<(f64, f64)>],
    chains: &BTreeMap<usize, Vec<usize>>,
    b: &Boxes,
) {
    for (&arc, chain) in chains {
        if chain.len() < 2 {
            continue;
        }
        let Some(route) = routes.get_mut(arc) else {
            continue;
        };
        if route.len() < 2 {
            continue;
        }
        let last = route.len() - 1;
        for (idx, inner, node) in [
            (0usize, 1usize, chain[0]),
            (last, last - 1, chain[chain.len() - 1]),
        ] {
            let Some(outline) = b.outline(node) else {
                continue;
            };
            if outline_distance(route[idx], outline) <= ENDPOINT_OUTLINE_TOL {
                continue;
            }
            if let Some(p) = project_onto_outline(route[idx], route[inner], outline)
                .or_else(|| border_toward(outline, route[idx]))
            {
                route[idx] = p;
            }
        }
    }
}

/// Give the plain-port route to side-port edges where it crosses less or clears a node box. The
/// second pass re-routes the whole graph, since shared port and lane setup dominates the cost.
pub(super) fn repair_side_ports(
    st: &LayeredState,
    boxes: &Boxes,
    o: &LayoutOptions,
    used: &[bool],
    routes: &mut [Vec<(f64, f64)>],
) {
    let candidates = || (0..routes.len()).filter(|&a| used.get(a).copied().unwrap_or(false));
    let hits = |arc: usize, route: &[(f64, f64)]| -> usize {
        let Some(chain) = st.chains.get(&arc).filter(|c| c.len() >= 2) else {
            return 0;
        };
        let (lo, hi) = (chain[0], chain[chain.len() - 1]);
        route
            .windows(2)
            .filter(|w| seg_hits_box(w[0], w[1], boxes, lo, hi))
            .count()
    };
    // When the lane towards the target's column would hit a node, the route falls back to the
    // lane below the source, and the side port sends the edge straight back across its face.
    let inward = |arc: usize, route: &[(f64, f64)]| -> bool {
        st.chains
            .get(&arc)
            .filter(|c| c.len() >= 2)
            .is_some_and(|c| ends_inward(route, boxes, c[0], c[c.len() - 1]))
    };
    if !candidates()
        .any(|a| arc_crossings(routes, a) > 0 || hits(a, &routes[a]) > 0 || inward(a, &routes[a]))
    {
        return;
    }
    let plain = route_edges(st, boxes, o, true, false, &mut []).routes;
    for arc in candidates() {
        if routes[arc] == plain[arc] {
            continue;
        }
        // A route through a node, or back across its own border, loses to any plain route that
        // has neither fault.
        if hits(arc, &routes[arc]) > 0 || inward(arc, &routes[arc]) {
            if hits(arc, &plain[arc]) == 0 && !inward(arc, &plain[arc]) {
                routes[arc] = plain[arc].clone();
            }
            continue;
        }
        let side_c = arc_crossings(routes, arc);
        if side_c == 0 {
            continue;
        }
        let prev = std::mem::replace(&mut routes[arc], plain[arc].clone());
        if arc_crossings(routes, arc) >= side_c {
            routes[arc] = prev;
        }
    }
}

/// Whether `route` leaves real node `lo` or enters real node `hi` heading back across the border
/// it attaches to ([`leaves_inward`]). Sub-pixel stubs at an end are skipped.
fn ends_inward(route: &[(f64, f64)], boxes: &Boxes, lo: usize, hi: usize) -> bool {
    let (Some(&first), Some(&last)) = (route.first(), route.last()) else {
        return false;
    };
    let clear_of =
        |end: (f64, f64)| move |q: &(f64, f64)| fp::hypot(q.0 - end.0, q.1 - end.1) > 0.5;
    let next = route.iter().skip(1).copied().find(clear_of(first));
    let prev = route.iter().rev().skip(1).copied().find(clear_of(last));
    let source = boxes.outline(lo).zip(next);
    let target = boxes.outline(hi).zip(prev);
    source.is_some_and(|(o, q)| leaves_inward(o, first, q))
        || target.is_some_and(|(o, q)| leaves_inward(o, last, q))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::geometry::shape::{NodeShape, Outline};

    /// A box and a circle 100 apart on the layer axis.
    fn boxes() -> Boxes {
        Boxes {
            cx: vec![0.0, 100.0],
            cy: vec![0.0, 0.0],
            layer_w: vec![40.0, 20.0],
            order_w: vec![20.0, 20.0],
            ellipse: vec![false, true],
            n_real: 2,
        }
    }

    #[test]
    fn settle_pulls_a_detached_endpoint_back_onto_the_outline() {
        let b = boxes();
        let chains: BTreeMap<usize, Vec<usize>> =
            [(0usize, vec![0usize, 1usize])].into_iter().collect();
        // Source 8px past the top of the box's right border, target 7px along the circle's chord.
        let mut routes = vec![vec![(20.0, 18.0), (60.0, 0.0), (90.0, 7.0)]];
        settle_endpoints_on_outline(&mut routes, &chains, &b);
        for (idx, node) in [(0usize, 0usize), (2usize, 1usize)] {
            let outline = b.outline(node).unwrap();
            let d = outline_distance(routes[0][idx], outline);
            assert!(
                d <= ENDPOINT_OUTLINE_TOL,
                "node {node}: {d}px off the outline"
            );
        }
    }

    #[test]
    fn settle_leaves_a_correct_endpoint_untouched() {
        let b = boxes();
        let chains: BTreeMap<usize, Vec<usize>> =
            [(0usize, vec![0usize, 1usize])].into_iter().collect();
        let before = vec![vec![(20.0, 0.0), (90.0, 0.0)]];
        let mut routes = before.clone();
        settle_endpoints_on_outline(&mut routes, &chains, &b);
        assert_eq!(routes, before);
    }

    #[test]
    fn settle_ignores_dummies_and_unrouted_chains() {
        let b = boxes();
        let chains: BTreeMap<usize, Vec<usize>> = [
            (0usize, vec![0usize, 5usize]),
            (1, vec![0, 1]),
            (2, vec![0]),
        ]
        .into_iter()
        .collect();
        let mut routes = vec![vec![(20.0, 9.0), (300.0, 9.0)], vec![], vec![]];
        settle_endpoints_on_outline(&mut routes, &chains, &b);
        assert_eq!(routes[0][1], (300.0, 9.0));
        assert_eq!(
            outline_distance(
                routes[0][0],
                Outline {
                    center: (0.0, 0.0),
                    size: (40.0, 20.0),
                    shape: NodeShape::Box
                }
            ),
            0.0
        );
    }
}

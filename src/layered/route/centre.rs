//! Port centring. Moves the ends on each node border towards the border's middle (see
//! [`crate::PortCentring`]) and straightens kinks between two level nodes. A move is undone when
//! [`move_score`] says it makes the drawing worse.

use crate::geometry::fp;
use std::collections::{BTreeMap, BTreeSet};

use super::geom::{border_reach, near_collinear_len};
use crate::geometry::hit::{arc_interior_crossings, point_seg_dist_residual, seg_hits_box};
use crate::geometry::polyline::simplify_collinear;
use crate::geometry::shape::project_onto_outline;
use crate::geometry::Boxes;
use crate::options::{LayoutOptions, PortCentring};
use crate::sort::SortDyn;

/// Pack step between an outgoing and an incoming end, in port gaps (two of a kind use half a gap),
/// so an arrowhead is not mistaken for part of the stroke beside it.
const OPPOSED_PACK_GAP: f64 = 0.65;
/// Widest pack worth making, as a fraction of the half-border.
const PACK_REACH: f64 = 0.8;
/// Shortest leg a moved end may leave between its border and its first corner, capped by the stub.
const MIN_LEG: f64 = 8.0;
/// Tallest jog [`straighten_kinks`] treats as a kink instead of a level change.
const KINK_MAX: f64 = 24.0;

/// One end of a routed arc on a node's border, as [`centre_ports`] sees it.
struct BorderEnd {
    arc: usize,
    /// Whether this is the route's first point.
    head: bool,
    /// Whether the drawn edge leaves the node here. Differs from `head` for a reversed arc.
    leaves: bool,
    node: usize,
    /// The coordinate along the border, `1` (y) on a left/right border and `0` (x) on top/bottom.
    along: usize,
    /// Direction the end segment heads away from the node, `+1.0` or `-1.0`.
    out: f64,
    /// Offset of the end from the border's middle.
    off: f64,
    /// Whether the route runs straight to its other end, so moving this end adds a jog.
    straight: bool,
}

/// Moves the ends on each node border towards its middle (see [`crate::PortCentring`]) without
/// putting two on one line. On an ellipse, a side end that turns at its first corner may take a
/// free top or bottom. A move is undone if [`move_score`] gets worse.
pub(super) fn centre_ports(
    routes: &mut [Vec<(f64, f64)>],
    chains: &BTreeMap<usize, Vec<usize>>,
    b: &Boxes,
    o: &LayoutOptions,
    movable: impl Fn(usize) -> bool,
    reversed: impl Fn(usize) -> bool,
    width: impl Fn(usize) -> f64,
) {
    let (mode, gap, stub) = (o.routing.centre_ports, o.spacing.port_gap, o.spacing.stub);
    if mode == PortCentring::Off {
        return;
    }
    let (ends, fixed, mut occupied) =
        collect_ends(routes, chains, b, mode, &movable, &reversed, &width);
    let mut borders: BTreeMap<BorderKey, Vec<usize>> = BTreeMap::new();
    for (i, e) in ends.iter().enumerate() {
        borders
            .entry((e.node, e.along, e.out as i8))
            .or_default()
            .push(i);
    }
    let score = |routes: &[Vec<(f64, f64)>], arcs: &[usize]| move_score(routes, arcs, chains, b);
    for ((node, along, out), mut members) in borders {
        let fixed_here = fixed
            .get(&(node, along, out))
            .map_or(&[][..], Vec::as_slice);
        let half = if along == 1 {
            b.order_w[node]
        } else {
            b.layer_w[node]
        } / 2.0;
        // A crowded side of a round node hands an end to a free top or bottom.
        if b.ellipse[node] && along == 1 {
            for dir in [-1.0f64, 1.0] {
                if members.len() < 2 || occupied.contains(&(node, 0, dir as i8)) {
                    continue;
                }
                let best = members
                    .iter()
                    .enumerate()
                    .filter(|&(_, &i)| !ends[i].straight)
                    .filter_map(|(k, &i)| {
                        corner_end(&routes[ends[i].arc], &ends[i], dir, stub, b).map(|r| (k, r))
                    })
                    .max_by(|x, y| x.1 .1.total_cmp(&y.1 .1));
                let Some((k, (route, _))) = best else {
                    continue;
                };
                let arc = ends[members[k]].arc;
                let was = score(routes, &[arc]);
                let old = std::mem::replace(&mut routes[arc], route);
                if score(routes, &[arc]).iter().zip(&was).any(|(n, w)| n > w) {
                    routes[arc] = old;
                } else {
                    occupied.insert((node, 0, dir as i8));
                    members.remove(k);
                }
            }
        }
        members.sort_by_dyn(|&x, &y| ends[x].off.total_cmp(&ends[y].off));
        let targets: Vec<f64> = if members.len() == 1 {
            let e = &ends[members[0]];
            if e.off.abs() <= half / 4.0 {
                continue;
            }
            vec![0.0]
        } else {
            let mut pos = vec![0.0];
            for w in members.windows(2) {
                let step = if ends[w[0]].leaves == ends[w[1]].leaves {
                    gap / 2.0
                } else {
                    gap * OPPOSED_PACK_GAP
                };
                pos.push(pos[pos.len() - 1] + step);
            }
            // A straight end near the middle keeps its line, otherwise the pack is centred.
            let shift = members
                .iter()
                .enumerate()
                .filter(|&(_, &i)| ends[i].straight && ends[i].off.abs() <= half / 3.0)
                .min_by(|x, y| ends[*x.1].off.abs().total_cmp(&ends[*y.1].off.abs()))
                .map(|(k, &i)| ends[i].off - pos[k])
                .unwrap_or(-pos[pos.len() - 1] / 2.0);
            let targets: Vec<f64> = pos.iter().map(|p| p + shift).collect();
            if targets.iter().any(|t| t.abs() > half * PACK_REACH) {
                continue;
            }
            targets
        };
        // An end may not land on or beside one that stays put.
        let crowds = members.iter().zip(&targets).any(|(&i, &at)| {
            let mine = width(ends[i].arc) / 2.0;
            fixed_here
                .iter()
                .any(|&(off, w)| (off - at).abs() < gap / 2.0 + mine + w - 1e-6)
        });
        if crowds {
            continue;
        }
        let arcs: Vec<usize> = members.iter().map(|&i| ends[i].arc).collect();
        let before: Vec<Vec<(f64, f64)>> = arcs.iter().map(|&a| routes[a].clone()).collect();
        let was = score(routes, &arcs);
        let mut ok = true;
        for (&i, &at) in members.iter().zip(&targets) {
            let e = &ends[i];
            if (e.off - at).abs() < 0.5 {
                continue;
            }
            let mid = if e.along == 1 {
                b.cy[e.node]
            } else {
                b.cx[e.node]
            };
            let chain = &chains[&e.arc];
            let far = if e.head {
                chain[chain.len() - 1]
            } else {
                chain[0]
            };
            let slid = e
                .straight
                .then(|| slide_straight(&routes[e.arc], e, mid + at, far, b))
                .flatten()
                .filter(|r| {
                    // The far end must stay `port_gap` clear of other ends on its border.
                    let p = if e.head { r[r.len() - 1] } else { r[0] };
                    chains.iter().filter(|&(&a, _)| a != e.arc).all(|(&a, c)| {
                        let Some(ra) = routes.get(a).filter(|ra| ra.len() >= 2) else {
                            return true;
                        };
                        [(c[0], ra[0]), (c[c.len() - 1], ra[ra.len() - 1])]
                            .iter()
                            .all(|&(n, q)| n != far || fp::hypot(q.0 - p.0, q.1 - p.1) >= gap)
                    })
                });
            match slid.or_else(|| move_end(&routes[e.arc], e, mid + at, stub, b)) {
                Some(r) => routes[e.arc] = r,
                None => ok = false,
            }
        }
        let now = score(routes, &arcs);
        if !ok || now.iter().zip(&was).any(|(n, w)| n > w) {
            for (&a, r) in arcs.iter().zip(before) {
                routes[a] = r;
            }
        }
    }
    straighten_kinks(routes, chains, b, o, &movable);
}

/// A node border, as `(node, along, out)` (see [`BorderEnd`]).
type BorderKey = (usize, usize, i8);

/// The ends that stay where they are, per border, as `(offset, half width)`.
type FixedEnds = BTreeMap<BorderKey, Vec<(f64, f64)>>;

/// Every routed end on a real node's border, split into the ones [`centre_ports`] may move and the
/// fixed ones, plus the set of borders any end uses.
fn collect_ends(
    routes: &[Vec<(f64, f64)>],
    chains: &BTreeMap<usize, Vec<usize>>,
    b: &Boxes,
    mode: PortCentring,
    movable: &impl Fn(usize) -> bool,
    reversed: &impl Fn(usize) -> bool,
    width: &impl Fn(usize) -> f64,
) -> (Vec<BorderEnd>, FixedEnds, BTreeSet<BorderKey>) {
    let mut ends: Vec<BorderEnd> = Vec::new();
    // A bundle's end counts as fixed, as wide as its corridor, because lanes are drawn around it.
    let mut fixed: FixedEnds = BTreeMap::new();
    // A corner end may only take a border no end uses.
    let mut occupied: BTreeSet<BorderKey> = BTreeSet::new();
    for (&arc, chain) in chains {
        let Some(route) = routes.get(arc) else {
            continue;
        };
        if chain.len() < 2 || route.len() < 2 || chain[0] == chain[chain.len() - 1] {
            continue;
        }
        for (head, node) in [(true, chain[0]), (false, chain[chain.len() - 1])] {
            if node >= b.n_real {
                continue;
            }
            let walk: Vec<(f64, f64)> = if head {
                route.clone()
            } else {
                route.iter().rev().copied().collect()
            };
            let p = walk[0];
            let Some(&q) = walk
                .iter()
                .skip(1)
                .find(|q| (q.0 - p.0).abs() + (q.1 - p.1).abs() > 0.5)
            else {
                continue;
            };
            let (along, out) = if (q.1 - p.1).abs() < 0.5 {
                (1, (q.0 - p.0).signum())
            } else if (q.0 - p.0).abs() < 0.5 {
                (0, (q.1 - p.1).signum())
            } else {
                continue;
            };
            occupied.insert((node, along, out as i8));
            let mid = if along == 1 { b.cy[node] } else { b.cx[node] };
            let off = if along == 1 { p.1 } else { p.0 } - mid;
            if !movable(arc) || (mode == PortCentring::Round && !b.ellipse[node]) {
                fixed
                    .entry((node, along, out as i8))
                    .or_default()
                    .push((off, width(arc) / 2.0));
                continue;
            }
            let level = if along == 1 { p.1 } else { p.0 };
            let straight = walk
                .iter()
                .all(|w| ((if along == 1 { w.1 } else { w.0 }) - level).abs() < 0.5);
            ends.push(BorderEnd {
                arc,
                head,
                leaves: head != reversed(arc),
                straight,
                node,
                along,
                out,
                off,
            });
        }
    }
    (ends, fixed, occupied)
}

/// Straightens a run-jog-run route between two level nodes by sliding both ends to one level,
/// within [`border_reach`] (half the half-border on an ellipse). Kept only if [`move_score`] does
/// not get worse.
fn straighten_kinks(
    routes: &mut [Vec<(f64, f64)>],
    chains: &BTreeMap<usize, Vec<usize>>,
    b: &Boxes,
    o: &LayoutOptions,
    movable: &impl Fn(usize) -> bool,
) {
    let clear = o.spacing.port_gap / 2.0;
    for (&arc, chain) in chains {
        let (lo, hi) = (chain[0], chain[chain.len() - 1]);
        if lo == hi || lo >= b.n_real || hi >= b.n_real || !movable(arc) {
            continue;
        }
        let r = routes[arc].clone();
        if r.len() != 4 {
            continue;
        }
        // `al` is the coordinate the two runs hold, `ac` the one they run along.
        let (al, ac) = if (r[0].1 - r[1].1).abs() < 0.5 && (r[2].1 - r[3].1).abs() < 0.5 {
            (1, 0)
        } else if (r[0].0 - r[1].0).abs() < 0.5 && (r[2].0 - r[3].0).abs() < 0.5 {
            (0, 1)
        } else {
            continue;
        };
        let get = |p: (f64, f64), k: usize| if k == 0 { p.0 } else { p.1 };
        let (y0, y1) = (get(r[0], al), get(r[3], al));
        if (y0 - y1).abs() > KINK_MAX || (y0 - y1).abs() < 0.5 {
            continue;
        }
        let range = |node: usize| {
            let (mid, ext) = if al == 1 {
                (b.cy[node], b.order_w[node])
            } else {
                (b.cx[node], b.layer_w[node])
            };
            let reach = if b.ellipse[node] {
                ext / 4.0
            } else {
                border_reach(ext, false)
            };
            (mid - reach, mid + reach, mid)
        };
        let (a_lo, a_hi, a_mid) = range(lo);
        let (b_lo, b_hi, b_mid) = range(hi);
        let (lo_lim, hi_lim) = (a_lo.max(b_lo), a_hi.min(b_hi));
        if lo_lim > hi_lim {
            continue;
        }
        let others: Vec<(usize, (f64, f64))> = chains
            .iter()
            .filter(|&(&a, _)| a != arc)
            .filter_map(|(&a, c)| routes.get(a).filter(|ra| ra.len() >= 2).map(|ra| (c, ra)))
            .flat_map(|(c, ra)| [(c[0], ra[0]), (c[c.len() - 1], ra[ra.len() - 1])])
            .filter(|&(n, _)| n == lo || n == hi)
            .collect();
        let mut cands = vec![y0, y1, (a_mid + b_mid) / 2.0, a_mid, b_mid];
        cands.retain(|&y| y >= lo_lim - 1e-9 && y <= hi_lim + 1e-9);
        cands.sort_by_dyn(|p, q| {
            let d = |y: f64| (y - a_mid).abs() + (y - b_mid).abs();
            d(*p).total_cmp(&d(*q))
        });
        for y in cands {
            let mut p = r[0];
            let mut q = r[3];
            if al == 1 {
                p.1 = y;
                q.1 = y;
            } else {
                p.0 = y;
                q.0 = y;
            }
            let (Some(ol), Some(oh)) = (b.outline(lo), b.outline(hi)) else {
                break;
            };
            let (Some(p), Some(q)) = (
                project_onto_outline(p, q, ol),
                project_onto_outline(q, p, oh),
            ) else {
                continue;
            };
            if get(q, ac) - get(p, ac) == 0.0
                || (get(q, ac) - get(p, ac)).signum() != (get(r[3], ac) - get(r[0], ac)).signum()
            {
                continue;
            }
            let crowded = others.iter().any(|&(n, e)| {
                let at = if n == lo { p } else { q };
                fp::hypot(e.0 - at.0, e.1 - at.1) < clear
            });
            if crowded {
                continue;
            }
            let was = move_score(routes, &[arc], chains, b);
            let old = std::mem::replace(&mut routes[arc], vec![p, q]);
            if move_score(routes, &[arc], chains, b)
                .iter()
                .zip(&was)
                .any(|(n, w)| n > w)
            {
                routes[arc] = old;
                continue;
            }
            break;
        }
    }
}

/// `[crossings, box hits, shared runs, touches]` of the routes of `arcs` against the drawing. A
/// touch is a corner of one route lying on another, which reads as a junction.
fn move_score(
    routes: &[Vec<(f64, f64)>],
    arcs: &[usize],
    chains: &BTreeMap<usize, Vec<usize>>,
    b: &Boxes,
) -> [usize; 4] {
    let (mut crossings, mut hits, mut overlaps, mut touches) = (0, 0, 0, 0);
    // Only interior corners count, because routes may meet at node ends.
    let corners_on = |r: &[(f64, f64)], other: &[(f64, f64)]| {
        let n = r.len();
        r.iter()
            .enumerate()
            .filter(|&(i, _)| i > 0 && i + 1 < n)
            .filter(|&(_, &p)| {
                other
                    .windows(2)
                    .any(|t| point_seg_dist_residual(p, t[0], t[1]) < 0.75)
            })
            .count()
    };
    for &a in arcs {
        crossings += arc_interior_crossings(routes, a);
        let (lo, hi) = (chains[&a][0], chains[&a][chains[&a].len() - 1]);
        hits += routes[a]
            .windows(2)
            .filter(|w| seg_hits_box(w[0], w[1], b, lo, hi))
            .count();
        for (o, other) in routes.iter().enumerate() {
            if o == a {
                continue;
            }
            for s in routes[a].windows(2) {
                overlaps += other
                    .windows(2)
                    .filter(|t| near_collinear_len(s[0], s[1], t[0], t[1]) > 1.0)
                    .count();
            }
            touches += corners_on(&routes[a], other) + corners_on(other, &routes[a]);
        }
    }
    [crossings, hits, overlaps, touches]
}

/// `route` with side end `e` moved onto the middle of the top (`dir < 0`) or bottom border of its
/// round node, rising straight to the run it turns into. `None` unless the route turns `dir`-ward
/// at its first corner with room for the new segment. Also returns how far out the run lies.
fn corner_end(
    route: &[(f64, f64)],
    e: &BorderEnd,
    dir: f64,
    stub: f64,
    b: &Boxes,
) -> Option<(Vec<(f64, f64)>, f64)> {
    let mut v: Vec<(f64, f64)> = if e.head {
        route.to_vec()
    } else {
        route.iter().rev().copied().collect()
    };
    let level = v[0].1;
    let mut j = 1;
    while j + 1 < v.len() && (v[j + 1].1 - level).abs() < 0.5 {
        j += 1;
    }
    if j + 2 >= v.len() {
        return None;
    }
    let (corner, turn, next) = (v[j], v[j + 1], v[j + 2]);
    let (cx, cy) = (b.cx[e.node], b.cy[e.node]);
    let rim = (cx, cy + dir * b.order_w[e.node] / 2.0);
    let riser_ok = (turn.0 - corner.0).abs() < 0.5 && (turn.1 - corner.1) * dir > 0.5;
    let run_ok = (next.1 - turn.1).abs() < 0.5 && (next.0 - cx).abs() > 1.0;
    if !riser_ok || !run_ok || (turn.1 - rim.1) * dir < stub.min(MIN_LEG) {
        return None;
    }
    let mut out = vec![rim, (cx, turn.1)];
    out.extend_from_slice(&v[j + 2..]);
    v = simplify_collinear(out);
    if !e.head {
        v.reverse();
    }
    Some((v, (turn.1 - cy).abs()))
}

/// A straight `route` moved whole onto the line `to`, so it stays straight where [`move_end`]
/// would jog it. `None` when the far border on `far` does not reach `to`, or would put the end on
/// a round node's shoulder.
fn slide_straight(
    route: &[(f64, f64)],
    e: &BorderEnd,
    to: f64,
    far: usize,
    b: &Boxes,
) -> Option<Vec<(f64, f64)>> {
    let al = e.along;
    let (mid, extent) = if al == 1 {
        (b.cy[far], b.order_w[far])
    } else {
        (b.cx[far], b.layer_w[far])
    };
    let reach = if b.ellipse[far] {
        extent / 6.0
    } else {
        border_reach(extent, false)
    };
    if far >= b.n_real || (to - mid).abs() > reach {
        return None;
    }
    let mut v: Vec<(f64, f64)> = route.to_vec();
    for p in v.iter_mut() {
        if al == 0 {
            p.0 = to
        } else {
            p.1 = to
        }
    }
    let last = v.len() - 1;
    let (near_i, far_i) = if e.head { (0, last) } else { (last, 0) };
    let (near_in, far_in) = (v[far_i], v[near_i]);
    v[near_i] = project_onto_outline(v[near_i], near_in, b.outline(e.node)?)?;
    v[far_i] = project_onto_outline(v[far_i], far_in, b.outline(far)?)?;
    Some(v)
}

/// `route` with end `e` moved to `to` along its border, or `None` when that cannot be done cleanly.
fn move_end(
    route: &[(f64, f64)],
    e: &BorderEnd,
    to: f64,
    stub: f64,
    b: &Boxes,
) -> Option<Vec<(f64, f64)>> {
    let mut v: Vec<(f64, f64)> = if e.head {
        route.to_vec()
    } else {
        route.iter().rev().copied().collect()
    };
    let (al, ac) = (e.along, 1 - e.along);
    let get = |p: (f64, f64), k: usize| if k == 0 { p.0 } else { p.1 };
    let set = |p: &mut (f64, f64), k: usize, val: f64| {
        if k == 0 {
            p.0 = val
        } else {
            p.1 = val
        }
    };
    // The end segment is v[0] .. v[j].
    let level = get(v[0], al);
    let mut j = 1;
    while j + 1 < v.len() && (get(v[j + 1], al) - level).abs() < 0.5 {
        j += 1;
    }
    if (get(v[j], al) - level).abs() >= 0.5 {
        return None;
    }
    let turns = j + 1 < v.len();
    if turns {
        // Slide the corner with the end segment so the route keeps its bends.
        for p in v.iter_mut().take(j + 1) {
            set(p, al, to);
        }
    } else {
        // A straight route jogs halfway, as far from either border as possible.
        let reach = (get(v[j], ac) - get(v[0], ac)).abs();
        let d = reach / 2.0;
        if d < stub.min(MIN_LEG) {
            return None;
        }
        let x = get(v[0], ac) + e.out * d;
        let mut c1 = v[0];
        set(&mut c1, ac, x);
        set(&mut c1, al, to);
        let mut c2 = v[0];
        set(&mut c2, ac, x);
        set(&mut c2, al, level);
        set(&mut v[0], al, to);
        v.insert(1, c2);
        v.insert(1, c1);
    }
    let outline = b.outline(e.node)?;
    let inner = v[1];
    let p = project_onto_outline(v[0], inner, outline)?;
    v[0] = p;
    if (get(v[0], ac) - get(inner, ac)) * e.out >= -0.5 {
        // The route would fold back.
        return None;
    }
    let mut v = simplify_collinear(v);
    if !e.head {
        v.reverse();
    }
    Some(v)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Default options with `mode` centring and an 18px port gap.
    fn centring(mode: PortCentring) -> LayoutOptions {
        let mut o = LayoutOptions::default();
        o.routing.centre_ports = mode;
        o.spacing.port_gap = 18.0;
        o
    }

    fn same_line(a: &[(f64, f64)], b: &[(f64, f64)]) -> bool {
        a.len() == b.len()
            && a.iter()
                .zip(b)
                .all(|(p, q)| (p.0 - q.0).abs() < 1e-6 && (p.1 - q.1).abs() < 1e-6)
    }

    /// A 52px circle (node 0) with boxes to its right. Node 1 is above, 2 level, 3 and 4 below.
    fn fan_boxes() -> Boxes {
        Boxes {
            cx: vec![0.0, 200.0, 200.0, 200.0, 200.0],
            cy: vec![0.0, -100.0, 0.0, 100.0, 150.0],
            layer_w: vec![52.0, 40.0, 40.0, 40.0, 40.0],
            order_w: vec![52.0, 20.0, 20.0, 20.0, 20.0],
            ellipse: vec![true, false, false, false, false],
            n_real: 5,
        }
    }

    fn rim(y: f64) -> (f64, f64) {
        ((26.0f64 * 26.0 - y * y).sqrt(), y)
    }

    /// Of three ends on a circle's right border, the climbing one moves to the top, the dropping
    /// one to the bottom, and the level one keeps its line.
    #[test]
    fn centring_rounds_turning_ends_onto_a_circle_s_free_top_and_bottom() {
        let b = fan_boxes();
        let chains: BTreeMap<usize, Vec<usize>> =
            [(0, vec![0, 1]), (1, vec![0, 2]), (2, vec![0, 3])]
                .into_iter()
                .collect();
        let before = vec![
            vec![rim(-16.0), (60.0, -16.0), (60.0, -100.0), (180.0, -100.0)],
            vec![(26.0, 0.0), (180.0, 0.0)],
            vec![rim(16.0), (80.0, 16.0), (80.0, 100.0), (180.0, 100.0)],
        ];
        let mut routes = before.clone();
        centre_ports(
            &mut routes,
            &chains,
            &b,
            &centring(PortCentring::Round),
            |_| true,
            |_| false,
            |_| 2.0,
        );
        assert_eq!(routes[1], before[1]);
        assert!(
            same_line(&routes[0], &[(0.0, -26.0), (0.0, -100.0), (180.0, -100.0)]),
            "{routes:?}"
        );
        assert!(
            same_line(&routes[2], &[(0.0, 26.0), (0.0, 100.0), (180.0, 100.0)]),
            "{routes:?}"
        );

        let mut routes = before.clone();
        centre_ports(
            &mut routes,
            &chains,
            &b,
            &centring(PortCentring::Off),
            |_| true,
            |_| false,
            |_| 2.0,
        );
        assert_eq!(routes, before);
    }

    /// With the top and bottom taken, the side's ends pack half a port gap apart around the
    /// straight end's line.
    #[test]
    fn centring_packs_a_side_around_its_straight_end() {
        let b = fan_boxes();
        let chains: BTreeMap<usize, Vec<usize>> = [
            (0, vec![0, 1]),
            (1, vec![0, 2]),
            (2, vec![0, 3]),
            (3, vec![1, 0]),
            (4, vec![4, 0]),
        ]
        .into_iter()
        .collect();
        let before = vec![
            vec![rim(-16.0), (60.0, -16.0), (60.0, -100.0), (180.0, -100.0)],
            vec![(26.0, 0.0), (180.0, 0.0)],
            vec![rim(16.0), (80.0, 16.0), (80.0, 100.0), (180.0, 100.0)],
            // These take the circle's top and bottom.
            vec![
                (200.0, -110.0),
                (200.0, -140.0),
                (0.0, -140.0),
                (0.0, -26.0),
            ],
            vec![(200.0, 160.0), (200.0, 190.0), (0.0, 190.0), (0.0, 26.0)],
        ];
        let mut routes = before.clone();
        centre_ports(
            &mut routes,
            &chains,
            &b,
            &centring(PortCentring::Round),
            |_| true,
            |_| false,
            |_| 2.0,
        );
        assert_eq!(routes[1], before[1]);
        assert!(
            same_line(
                &routes[0],
                &[rim(-9.0), (60.0, -9.0), (60.0, -100.0), (180.0, -100.0)]
            ),
            "{routes:?}"
        );
        assert!(
            same_line(
                &routes[2],
                &[rim(9.0), (80.0, 9.0), (80.0, 100.0), (180.0, 100.0)]
            ),
            "{routes:?}"
        );
    }

    /// Outgoing and incoming ends pack further apart than two of a kind, and a reversed arc's
    /// end counts by its drawn direction.
    #[test]
    fn centring_keeps_incoming_and_outgoing_ends_further_apart() {
        let mut b = fan_boxes();
        b.cy = vec![0.0, 0.0, 100.0, 150.0, 300.0];
        let chains: BTreeMap<usize, Vec<usize>> =
            [(0, vec![0, 2]), (1, vec![0, 1]), (2, vec![0, 3])]
                .into_iter()
                .collect();
        let before = vec![
            vec![rim(10.0), (70.0, 10.0), (70.0, 100.0), (180.0, 100.0)],
            vec![(26.0, 0.0), (180.0, 0.0)],
            vec![rim(20.0), (50.0, 20.0), (50.0, 150.0), (180.0, 150.0)],
        ];
        let mut routes = before.clone();
        // Arc 1 is reversed, so it is drawn into the circle. Arc 2 takes the free bottom.
        centre_ports(
            &mut routes,
            &chains,
            &b,
            &centring(PortCentring::Round),
            |_| true,
            |a| a == 1,
            |_| 2.0,
        );
        assert_eq!(routes[1], before[1]);
        assert!(
            same_line(&routes[2], &[(0.0, 26.0), (0.0, 150.0), (180.0, 150.0)]),
            "{routes:?}"
        );
        let y = routes[0][0].1;
        assert!((y - 18.0 * 0.65).abs() < 1e-6, "outgoing port at {y}");
        assert_eq!(routes[0].len(), 4);
    }

    /// A move that adds a crossing is undone.
    #[test]
    fn centring_is_undone_where_it_would_add_a_crossing() {
        let b = fan_boxes();
        let chains: BTreeMap<usize, Vec<usize>> =
            [(0, vec![0, 1]), (1, vec![2, 3])].into_iter().collect();
        // Arc 1 is clear of arc 0 before centring, and arc 0's riser would cut it after.
        let before = vec![
            vec![rim(-16.0), (60.0, -16.0), (60.0, -100.0), (180.0, -100.0)],
            vec![(50.0, -8.0), (70.0, -8.0)],
        ];
        let mut routes = before.clone();
        centre_ports(
            &mut routes,
            &chains,
            &b,
            &centring(PortCentring::Round),
            |_| true,
            |_| false,
            |_| 2.0,
        );
        assert_eq!(routes, before);
    }

    /// An 8px jog between a level box and circle is straightened. A 30px jog is a level change
    /// and stays.
    #[test]
    fn a_kink_between_two_level_nodes_is_drawn_straight() {
        let b = Boxes {
            cx: vec![0.0, 100.0],
            cy: vec![0.0, 0.0],
            layer_w: vec![20.0, 52.0],
            order_w: vec![52.0, 52.0],
            ellipse: vec![false, true],
            n_real: 2,
        };
        let chains: BTreeMap<usize, Vec<usize>> = [(0, vec![0, 1])].into_iter().collect();
        let mut routes = vec![vec![(10.0, 8.0), (40.0, 8.0), (40.0, 0.0), (74.0, 0.0)]];
        straighten_kinks(
            &mut routes,
            &chains,
            &b,
            &centring(PortCentring::Round),
            &|_| true,
        );
        assert!(
            same_line(&routes[0], &[(10.0, 0.0), (74.0, 0.0)]),
            "{routes:?}"
        );

        let tall = vec![vec![
            (10.0, 20.0),
            (40.0, 20.0),
            (40.0, -10.0),
            (74.0, -10.0),
        ]];
        let mut routes = tall.clone();
        straighten_kinks(
            &mut routes,
            &chains,
            &b,
            &centring(PortCentring::Round),
            &|_| true,
        );
        assert_eq!(routes, tall);
    }
}

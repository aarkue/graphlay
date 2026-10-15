//! Hit tests and clearance: segment against box, segment against segment, point to segment, and
//! the nearest coordinate outside a set of blocked intervals.
//!
//! The crossing and distance kernels come in several variants because each phase is tuned
//! against its own, and they disagree at edge cases (a T, a shared endpoint, a tiny segment).

use super::stats::pair_order;
use super::Boxes;
use super::{Pt, DEGENERATE, DEGENERATE_SQ, EPS, SLACK};
use crate::geometry::fp;
use crate::sort::SortDyn;

/// Whether segment `p->q` cuts through any real node box other than `skip1`/`skip2`.
pub fn seg_hits_box(p: (f64, f64), q: (f64, f64), b: &Boxes, skip1: usize, skip2: usize) -> bool {
    seg_hits_box_pad(p, q, b, skip1, skip2, -1.0)
}

/// [`seg_hits_box`] with each box inflated by `pad`. The plain test uses -1 so a route may touch
/// a border.
pub fn seg_hits_box_pad(
    p: (f64, f64),
    q: (f64, f64),
    b: &Boxes,
    skip1: usize,
    skip2: usize,
    pad: f64,
) -> bool {
    (0..b.n_real).any(|i| i != skip1 && i != skip2 && seg_hits_one_box(p, q, b, i, pad))
}

/// Whether segment `p->q` cuts through real node `i`'s box inflated by `pad`. Dummies are never
/// hit.
pub fn seg_hits_one_box(p: (f64, f64), q: (f64, f64), b: &Boxes, i: usize, pad: f64) -> bool {
    if i >= b.n_real || b.layer_w[i] <= 0.0 {
        return false;
    }
    let (xmin, xmax) = (
        b.cx[i] - b.layer_w[i] / 2.0 - pad,
        b.cx[i] + b.layer_w[i] / 2.0 + pad,
    );
    let (ymin, ymax) = (
        b.cy[i] - b.order_w[i] / 2.0 - pad,
        b.cy[i] + b.order_w[i] / 2.0 + pad,
    );
    if xmin >= xmax || ymin >= ymax {
        return false;
    }
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for &(pc, dc, lo, hi) in &[(p.0, q.0 - p.0, xmin, xmax), (p.1, q.1 - p.1, ymin, ymax)] {
        if dc.abs() < EPS {
            if pc < lo || pc > hi {
                return false;
            }
        } else {
            let (mut ta, mut tb) = ((lo - pc) / dc, (hi - pc) / dc);
            if ta > tb {
                std::mem::swap(&mut ta, &mut tb);
            }
            t0 = t0.max(ta);
            t1 = t1.min(tb);
        }
    }
    t0 < t1 - SLACK
}

/// Twice the signed area of `a, b, c`, positive when `c` is left of `a -> b`.
fn orient(a: Pt, b: Pt, c: Pt) -> f64 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

/// Sign-test crossing of `p1 -> p2` and `p3 -> p4`, with zero orientation on the non-positive side.
/// A T or collinear touch may count depending on direction, and shared endpoints are not skipped.
pub fn segments_cross(p1: Pt, p2: Pt, p3: Pt, p4: Pt) -> bool {
    (orient(p3, p4, p1) > 0.0) != (orient(p3, p4, p2) > 0.0)
        && (orient(p1, p2, p3) > 0.0) != (orient(p1, p2, p4) > 0.0)
}

/// Strict crossing of `p -> p2` and `q -> q2`. A T, a shared endpoint or a collinear touch never
/// counts.
pub fn segments_cross_interior(p: Pt, p2: Pt, q: Pt, q2: Pt) -> bool {
    orient(q, q2, p) * orient(q, q2, p2) < -SLACK && orient(p, p2, q) * orient(p, p2, q2) < -SLACK
}

/// Where `p -> p2` and `q -> q2` cross with both parameters in `(EPS, 1 - EPS)`, or `None` for
/// near-parallel segments, a T, or a touch at an end.
pub fn proper_intersection(p: Pt, p2: Pt, q: Pt, q2: Pt) -> Option<Pt> {
    let r = (p2.0 - p.0, p2.1 - p.1);
    let s = (q2.0 - q.0, q2.1 - q.1);
    let denom = r.0 * s.1 - r.1 * s.0;
    if denom.abs() < DEGENERATE {
        return None;
    }
    let qp = (q.0 - p.0, q.1 - p.1);
    let t = (qp.0 * s.1 - qp.1 * s.0) / denom;
    let u = (qp.0 * r.1 - qp.1 * r.0) / denom;
    if t > EPS && t < 1.0 - EPS && u > EPS && u < 1.0 - EPS {
        Some((p.0 + r.0 * t, p.1 + r.1 * t))
    } else {
        None
    }
}

/// Whether `p -> p2` and `q -> q2` have a `proper_intersection`.
pub fn segments_cross_proper(p: Pt, p2: Pt, q: Pt, q2: Pt) -> bool {
    proper_intersection(p, p2, q, q2).is_some()
}

/// Crossings of `arc`'s route with every other route by [`segments_cross`], skipping shared
/// points.
pub fn arc_crossings(routes: &[Vec<(f64, f64)>], arc: usize) -> usize {
    let mut n = 0;
    for j in 0..routes.len() {
        if j == arc {
            continue;
        }
        for wa in routes[arc].windows(2) {
            for wb in routes[j].windows(2) {
                let (p1, p2, p3, p4) = (wa[0], wa[1], wb[0], wb[1]);
                if p1 == p3 || p1 == p4 || p2 == p3 || p2 == p4 {
                    continue;
                }
                if segments_cross(p1, p2, p3, p4) {
                    n += 1;
                }
            }
        }
    }
    n
}

/// Crossings of `arc`'s route with every other route by [`segments_cross_interior`], so a T never
/// counts.
pub fn arc_interior_crossings(routes: &[Vec<Pt>], arc: usize) -> usize {
    let mut n = 0;
    for (j, other) in routes.iter().enumerate() {
        if j == arc {
            continue;
        }
        for s in routes[arc].windows(2) {
            for t in other.windows(2) {
                if segments_cross_interior(s[0], s[1], t[0], t[1]) {
                    n += 1;
                }
            }
        }
    }
    n
}

fn foot_param(p: Pt, a: Pt, d: Pt, len2: f64) -> f64 {
    (((p.0 - a.0) * d.0 + (p.1 - a.1) * d.1) / len2).clamp(0.0, 1.0)
}

fn foot_dist(p: Pt, a: Pt, b: Pt, is_point: impl Fn(f64) -> bool) -> f64 {
    let d = (b.0 - a.0, b.1 - a.1);
    let len2 = d.0 * d.0 + d.1 * d.1;
    if is_point(len2) {
        return fp::hypot(p.0 - a.0, p.1 - a.1);
    }
    let t = foot_param(p, a, d, len2);
    fp::hypot(p.0 - (a.0 + d.0 * t), p.1 - (a.1 + d.1 * t))
}

/// Distance from `p` to segment `a -> b`. Below `DEGENERATE_SQ` squared length it is the point `a`.
pub fn point_seg_dist(p: Pt, a: Pt, b: Pt) -> f64 {
    foot_dist(p, a, b, |len2| len2 < DEGENERATE_SQ)
}

/// [`point_seg_dist`] with the degenerate cut at `f64::EPSILON` squared length.
pub fn point_seg_dist_coarse(p: Pt, a: Pt, b: Pt) -> f64 {
    foot_dist(p, a, b, |len2| len2 <= f64::EPSILON)
}

/// [`point_seg_dist`] as the residual `(p - a) - t (b - a)`, which rounds differently, with the
/// degenerate cut at `DEGENERATE`.
pub fn point_seg_dist_residual(p: Pt, a: Pt, b: Pt) -> f64 {
    let d = (b.0 - a.0, b.1 - a.1);
    let len2 = d.0 * d.0 + d.1 * d.1;
    let t = if len2 < DEGENERATE {
        0.0
    } else {
        foot_param(p, a, d, len2)
    };
    fp::hypot(p.0 - a.0 - t * d.0, p.1 - a.1 - t * d.1)
}

/// `x` if it lies outside every interval in `ivals`, else the nearer end of the merged interval
/// containing it.
pub fn nearest_clear_lane(x: f64, ivals: &mut [(f64, f64)]) -> f64 {
    if ivals.is_empty() {
        return x;
    }
    ivals.sort_unstable_by_dyn(pair_order);
    let mut merged: Vec<(f64, f64)> = vec![];
    for &(a, b) in ivals.iter() {
        match merged.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => merged.push((a, b)),
        }
    }
    for &(a, b) in &merged {
        if x >= a && x <= b {
            return if x - a <= b - x { a } else { b };
        }
    }
    x
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// One 10x10 box centred at the origin.
    fn one_box() -> Boxes {
        Boxes {
            cx: vec![0.0],
            cy: vec![0.0],
            layer_w: vec![10.0],
            order_w: vec![10.0],
            ellipse: vec![false],
            n_real: 1,
        }
    }

    #[test]
    fn seg_hits_box_sees_a_segment_straight_through() {
        let b = one_box();
        assert!(seg_hits_box(
            (-20.0, 0.0),
            (20.0, 0.0),
            &b,
            usize::MAX,
            usize::MAX
        ));
        assert!(!seg_hits_box(
            (-20.0, 20.0),
            (20.0, 20.0),
            &b,
            usize::MAX,
            usize::MAX
        ));
        // The skipped box is the segment's own endpoint node.
        assert!(!seg_hits_box((-20.0, 0.0), (20.0, 0.0), &b, 0, usize::MAX));
    }

    #[test]
    fn seg_hits_box_pad_inflates_the_box() {
        let b = one_box();
        let (p, q) = ((-20.0, 8.0), (20.0, 8.0));
        assert!(!seg_hits_box(p, q, &b, usize::MAX, usize::MAX));
        assert!(seg_hits_box_pad(p, q, &b, usize::MAX, usize::MAX, 5.0));
    }

    #[test]
    fn segments_cross_is_a_proper_intersection() {
        assert!(segments_cross(
            (0.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
            (10.0, 0.0)
        ));
        assert!(!segments_cross(
            (0.0, 0.0),
            (10.0, 0.0),
            (0.0, 5.0),
            (10.0, 5.0)
        ));
        assert!(!segments_cross(
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 0.0),
            (20.0, 0.0)
        ));
    }

    #[test]
    fn nearest_clear_lane_leaves_a_clear_point_alone() {
        assert_eq!(nearest_clear_lane(20.0, &mut [(0.0, 10.0)]), 20.0);
        assert_eq!(nearest_clear_lane(3.0, &mut []), 3.0);
    }

    /// Intervals merge first, so a point is never pushed into another blocked span.
    #[test]
    fn nearest_clear_lane_escapes_the_nearest_side_of_the_merged_span() {
        assert_eq!(nearest_clear_lane(3.0, &mut [(0.0, 10.0)]), 0.0);
        assert_eq!(nearest_clear_lane(7.0, &mut [(0.0, 10.0)]), 10.0);
        assert_eq!(
            nearest_clear_lane(9.0, &mut [(0.0, 10.0), (8.0, 30.0)]),
            0.0
        );
    }
}

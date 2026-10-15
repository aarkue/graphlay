//! Geometry the repair passes judge their moves by: route interference, near-collinear overlaps,
//! and how far along a border a port may slide. Shared segment kernels live in
//! [`crate::geometry::hit`].

use crate::geometry::fp;
use crate::geometry::hit::segments_cross;

/// Two runs closer than this read as one line, even when the metric's 0.5px test misses them.
pub(super) const RUN_NEAR: f64 = 2.5;

/// Max distance of a port from its border's middle. An ellipse gets less because its outline
/// curves away from the tangent a port slides along.
pub(super) fn border_reach(extent: f64, ellipse: bool) -> f64 {
    extent / 2.0 * if ellipse { 0.5 } else { 0.7 }
}

/// `(crossings, overlaps)` of `route` against `others`, ordered so crossings compare first. An
/// overlap is two parallel axis-aligned segments within `gap` that share more than a pixel.
pub(super) fn interference(
    route: &[(f64, f64)],
    others: &[Vec<(f64, f64)>],
    gap: f64,
) -> (usize, usize) {
    let (mut cross, mut overlap) = (0usize, 0usize);
    if route.len() < 2 {
        return (0, 0);
    }
    let (mut x0, mut y0, mut x1, mut y1) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for p in route {
        x0 = x0.min(p.0);
        y0 = y0.min(p.1);
        x1 = x1.max(p.0);
        y1 = y1.max(p.1);
    }
    for other in others {
        if other.len() < 2 {
            continue;
        }
        // A bounding-box reject skips most pair tests.
        let clear = other.iter().all(|p| p.0 < x0 - gap)
            || other.iter().all(|p| p.0 > x1 + gap)
            || other.iter().all(|p| p.1 < y0 - gap)
            || other.iter().all(|p| p.1 > y1 + gap);
        if clear {
            continue;
        }
        for wa in route.windows(2) {
            for wb in other.windows(2) {
                let (p1, p2, p3, p4) = (wa[0], wa[1], wb[0], wb[1]);
                // Edges out of one port share a point without crossing, but can still overlap.
                let touching = p1 == p3 || p1 == p4 || p2 == p3 || p2 == p4;
                if !touching && segments_cross(p1, p2, p3, p4) {
                    cross += 1;
                    continue;
                }
                let a_h = (p1.1 - p2.1).abs() < 0.5;
                let a_v = (p1.0 - p2.0).abs() < 0.5;
                let b_h = (p3.1 - p4.1).abs() < 0.5;
                let b_v = (p3.0 - p4.0).abs() < 0.5;
                let shared = |a: f64, b: f64, c: f64, d: f64| {
                    a.min(b).max(c.min(d)) + 1.0 < a.max(b).min(c.max(d))
                };
                let along_x =
                    a_h && b_h && (p1.1 - p3.1).abs() < gap && shared(p1.0, p2.0, p3.0, p4.0);
                let along_y =
                    a_v && b_v && (p1.0 - p3.0).abs() < gap && shared(p1.1, p2.1, p3.1, p4.1);
                if along_x || along_y {
                    overlap += 1;
                }
            }
        }
    }
    (cross, overlap)
}

/// Length over which `q -> q2` lies within [`RUN_NEAR`] of `p -> p2`, else `0.0`.
pub(super) fn near_collinear_len(
    p: (f64, f64),
    p2: (f64, f64),
    q: (f64, f64),
    q2: (f64, f64),
) -> f64 {
    let r = (p2.0 - p.0, p2.1 - p.1);
    let len = fp::hypot(r.0, r.1);
    if len < 1e-9 {
        return 0.0;
    }
    let dir = (r.0 / len, r.1 / len);
    let perp = |x: (f64, f64)| (dir.0 * (x.1 - p.1) - dir.1 * (x.0 - p.0)).abs();
    if perp(q) > RUN_NEAR || perp(q2) > RUN_NEAR {
        return 0.0;
    }
    let along = |x: (f64, f64)| dir.0 * (x.0 - p.0) + dir.1 * (x.1 - p.1);
    let (t1, t2) = (along(q), along(q2));
    (t1.max(t2).min(len) - t1.min(t2).max(0.0)).max(0.0)
}

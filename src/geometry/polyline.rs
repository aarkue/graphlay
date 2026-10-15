//! Polyline operations: line intersection, lateral offsetting, collinear simplification,
//! Ramer-Douglas-Peucker thinning and point at a fraction of the length. SVG path output lives in
//! `svg::render`.
#![cfg_attr(not(feature = "svg"), allow(dead_code))]

use super::shape::{border_toward, outline_distance, project_onto_outline, Outline};
use super::{Pt, EPS, SLACK};
use crate::geometry::fp;

/// Intersection of two infinite lines given by two points each, or `None` when near-parallel.
pub fn intersect_lines(
    l1: ((f64, f64), (f64, f64)),
    l2: ((f64, f64), (f64, f64)),
) -> Option<(f64, f64)> {
    let (p, p2) = l1;
    let (q, q2) = l2;
    let r = (p2.0 - p.0, p2.1 - p.1);
    let s = (q2.0 - q.0, q2.1 - q.1);
    let denom = r.0 * s.1 - r.1 * s.0;
    if denom.abs() < EPS {
        return None;
    }
    let t = ((q.0 - p.0) * s.1 - (q.1 - p.1) * s.0) / denom;
    Some((p.0 + t * r.0, p.1 + t * r.1))
}

/// Shifts a polyline sideways by `d` (positive is right of travel), mitering the corners.
/// Endpoints move too. [`offset_route`] puts them back on the node borders.
pub fn offset_polyline(route: &[Pt], d: f64) -> Vec<Pt> {
    offset_polyline_segments(route, &[d])
}

/// [`offset_polyline`] with `d[i]` shifting segment `i`, the last entry repeating. A segment
/// parallel to the previous one keeps its distance, since there is no corner to join at.
pub fn offset_polyline_segments(route: &[Pt], d: &[f64]) -> Vec<Pt> {
    let mut pts: Vec<(f64, f64)> = Vec::with_capacity(route.len());
    for &p in route {
        if pts
            .last()
            .is_none_or(|&q: &(f64, f64)| fp::hypot(p.0 - q.0, p.1 - q.1) > SLACK)
        {
            pts.push(p);
        }
    }
    if pts.len() < 2 {
        return route.to_vec();
    }
    let n = pts.len();
    let mut carried = 0.0;
    let seg: Vec<((f64, f64), (f64, f64))> = (0..n - 1)
        .map(|i| {
            let (ax, ay) = pts[i];
            let (bx, by) = pts[i + 1];
            let len = fp::hypot(bx - ax, by - ay);
            let (nx, ny) = (-(by - ay) / len, (bx - ax) / len);
            let want = d.get(i).or_else(|| d.last()).copied().unwrap_or(0.0);
            let parallel = i > 0 && {
                let (px, py) = (pts[i].0 - pts[i - 1].0, pts[i].1 - pts[i - 1].1);
                ((bx - ax) * py - (by - ay) * px).abs() < EPS * len.max(1.0)
            };
            let dist = if parallel { carried } else { want };
            carried = dist;
            (
                (ax + nx * dist, ay + ny * dist),
                (bx + nx * dist, by + ny * dist),
            )
        })
        .collect();
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(n);
    out.push(seg[0].0);
    for i in 1..n - 1 {
        out.push(intersect_lines(seg[i - 1], seg[i]).unwrap_or(seg[i].0));
    }
    out.push(seg[n - 2].1);
    out
}

/// [`offset_polyline`], with both endpoints re-projected onto the node outlines they attach to.
pub fn offset_route(route: &[Pt], d: f64, from: Outline, to: Outline) -> Vec<Pt> {
    let mut out = offset_polyline(route, d);
    if out.len() < 2 {
        return out;
    }
    // A large offset can make the `inner`->`end` line miss the node, so fall back to the centre
    // ray, but only for an endpoint within reach of this outline.
    let snap = |end: Pt, inner: Pt, outline: Outline| -> Option<Pt> {
        if let Some(p) = project_onto_outline(end, inner, outline) {
            return Some(p);
        }
        let reach = d.abs() + outline.size.0.max(outline.size.1);
        (outline_distance(end, outline) <= reach)
            .then(|| border_toward(outline, end).or_else(|| border_toward(outline, inner)))
            .flatten()
    };
    let last = out.len() - 1;
    if let Some(p) = snap(out[0], out[1], from) {
        out[0] = p;
    }
    if let Some(p) = snap(out[last], out[last - 1], to) {
        out[last] = p;
    }
    out
}

/// Drops coincident points (within 0.5) and points on the line between their neighbours.
pub fn simplify_collinear(pts: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    let mut dd: Vec<(f64, f64)> = Vec::with_capacity(pts.len());
    for p in pts {
        if dd
            .last()
            .is_none_or(|q: &(f64, f64)| (p.0 - q.0).abs() > 0.5 || (p.1 - q.1).abs() > 0.5)
        {
            dd.push(p);
        }
    }
    if dd.len() <= 2 {
        return dd;
    }
    let mut out: Vec<(f64, f64)> = vec![dd[0]];
    for i in 1..dd.len() - 1 {
        let (a, b, c) = (out.last().copied().unwrap_or(dd[0]), dd[i], dd[i + 1]);
        let cross = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
        if cross.abs() > 1.0 {
            out.push(b);
        }
    }
    if let Some(&last) = dd.last() {
        out.push(last);
    }
    out
}

/// Shortens the last segment by `by` to leave room for an arrowhead. It never passes the last
/// corner, because the marker orients along the final segment.
pub fn shorten_end(pts: &[Pt], by: f64) -> Vec<Pt> {
    if pts.len() < 2 || by <= 0.0 {
        return pts.to_vec();
    }
    let mut out = pts.to_vec();
    let n = out.len();
    let (ax, ay) = out[n - 2];
    let (bx, by_) = out[n - 1];
    let len = f64::hypot(bx - ax, by_ - ay);
    let trim = by.min(len - 0.5).max(0.0);
    if trim > 0.0 {
        out[n - 1] = (bx - (bx - ax) / len * trim, by_ - (by_ - ay) / len * trim);
    }
    out
}

/// Interior points closer than this (px) to the previous kept point are dropped.
const MIN_POINT_GAP: f64 = 3.0;

/// RDP, then drops points closer than [`MIN_POINT_GAP`]. The first and last segments stay as they
/// are because they orient the end markers.
pub fn clean_path(pts: &[Pt], tol: f64) -> Vec<Pt> {
    if pts.len() < 4 {
        return pts.to_vec();
    }
    // RDP would turn a riser shorter than `tol` into a slant, so orthogonal routes only lose
    // collinear points.
    let orthogonal = pts
        .windows(2)
        .all(|w| (w[0].0 - w[1].0).abs() < 0.5 || (w[0].1 - w[1].1).abs() < 0.5);
    if orthogonal {
        return simplify_collinear(pts.to_vec());
    }
    let mut s = vec![pts[0]];
    s.extend(rdp(&pts[1..pts.len() - 1], tol));
    s.push(pts[pts.len() - 1]);
    let mut out: Vec<Pt> = Vec::with_capacity(s.len());
    for (i, &p) in s.iter().enumerate() {
        let keep_end = i <= 1 || i >= s.len() - 2;
        match out.last() {
            Some(&last) if !keep_end && fp::hypot(last.0 - p.0, last.1 - p.1) < MIN_POINT_GAP => {}
            _ => out.push(p),
        }
    }
    out
}

/// Ramer-Douglas-Peucker simplification, dropping points within `tol` of the chord.
/// Iterative because the recursive form can nest once per point and overflow wasm's 1 MB stack.
pub fn rdp(pts: &[Pt], tol: f64) -> Vec<Pt> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    let mut spans = vec![(0usize, pts.len() - 1)];
    while let Some((lo, hi)) = spans.pop() {
        if hi <= lo + 1 {
            continue;
        }
        let (a, b) = (pts[lo], pts[hi]);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = f64::hypot(dx, dy).max(EPS);
        let mut far = lo;
        let mut fmax = 0.0;
        for (i, &p) in pts.iter().enumerate().take(hi).skip(lo + 1) {
            let d = ((p.0 - a.0) * dy - (p.1 - a.1) * dx).abs() / len;
            if d > fmax {
                fmax = d;
                far = i;
            }
        }
        // `far == lo` means every point is on the chord, which with a negative `tol` would split
        // forever.
        if fmax <= tol || far == lo {
            continue;
        }
        keep[far] = true;
        spans.push((lo, far));
        spans.push((far, hi));
    }
    pts.iter()
        .zip(&keep)
        .filter_map(|(&p, &k)| k.then_some(p))
        .collect()
}

/// Point at `frac` (clamped to 0..1) of the polyline's total length.
pub fn polyline_point_at(pts: &[Pt], frac: f64) -> Pt {
    if pts.len() < 2 {
        return pts.first().copied().unwrap_or((0.0, 0.0));
    }
    let segs: Vec<f64> = (1..pts.len())
        .map(|i| f64::hypot(pts[i].0 - pts[i - 1].0, pts[i].1 - pts[i - 1].1))
        .collect();
    let target: f64 = segs.iter().sum::<f64>() * frac.clamp(0.0, 1.0);
    let mut acc = 0.0;
    for (i, &s) in segs.iter().enumerate() {
        if acc + s >= target {
            let t = if s == 0.0 { 0.0 } else { (target - acc) / s };
            return (
                pts[i].0 + (pts[i + 1].0 - pts[i].0) * t,
                pts[i].1 + (pts[i + 1].1 - pts[i].1) * t,
            );
        }
        acc += s;
    }
    pts.last().copied().unwrap_or((0.0, 0.0))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::geometry::shape::NodeShape;

    #[test]
    fn intersect_lines_meets_at_the_crossing() {
        assert_eq!(
            intersect_lines(((0.0, 0.0), (1.0, 0.0)), ((5.0, -1.0), (5.0, 1.0))),
            Some((5.0, 0.0))
        );
        assert_eq!(
            intersect_lines(((0.0, 0.0), (1.0, 0.0)), ((0.0, 5.0), (1.0, 5.0))),
            None
        );
    }

    #[test]
    fn clean_path_keeps_an_orthogonal_route_orthogonal() {
        let pts = vec![
            (0.0, 0.0),
            (100.0, 0.0),
            (100.0, 6.0),
            (200.0, 6.0),
            (200.0, 80.0),
        ];
        assert_eq!(clean_path(&pts, 8.0), pts);
    }

    #[test]
    fn simplify_collinear_drops_straight_through_points_but_keeps_corners() {
        assert_eq!(
            simplify_collinear(vec![(0.0, 0.0), (5.0, 0.0), (10.0, 0.0)]),
            vec![(0.0, 0.0), (10.0, 0.0)]
        );
        assert_eq!(
            simplify_collinear(vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]),
            vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]
        );
    }

    #[test]
    fn simplify_collinear_drops_coincident_points() {
        assert_eq!(
            simplify_collinear(vec![(0.0, 0.0), (0.2, 0.2), (10.0, 0.0)]),
            vec![(0.0, 0.0), (10.0, 0.0)]
        );
    }

    /// Endpoints far from any node stay where the offset put them.
    #[test]
    fn offset_route_shifts_by_exactly_d() {
        let far = Outline {
            center: (1000.0, 1000.0),
            size: (10.0, 10.0),
            shape: NodeShape::Box,
        };
        assert_eq!(
            offset_route(&[(0.0, 0.0), (10.0, 0.0)], 2.0, far, far),
            vec![(0.0, 2.0), (10.0, 2.0)]
        );
    }

    #[test]
    fn offset_polyline_segments_moves_each_segment_by_its_own_distance() {
        assert_eq!(
            offset_polyline_segments(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], &[2.0, 4.0]),
            vec![(0.0, 2.0), (6.0, 2.0), (6.0, 10.0)]
        );
    }

    #[test]
    fn offset_polyline_segments_carries_a_parallel_neighbour() {
        assert_eq!(
            offset_polyline_segments(&[(0.0, 0.0), (10.0, 0.0), (20.0, 0.0)], &[2.0, 8.0]),
            vec![(0.0, 2.0), (10.0, 2.0), (20.0, 2.0)]
        );
    }

    #[test]
    fn offset_route_reprojects_onto_the_borders() {
        let src = Outline {
            center: (0.0, 0.0),
            size: (10.0, 10.0),
            shape: NodeShape::Box,
        };
        let dst = Outline {
            center: (100.0, 0.0),
            size: (10.0, 10.0),
            shape: NodeShape::Box,
        };
        let out = offset_route(&[(5.0, 0.0), (95.0, 0.0)], 2.0, src, dst);
        assert_eq!(out.first().copied(), Some((5.0, 2.0)));
        assert_eq!(out.last().copied(), Some((95.0, 2.0)));
    }

    #[test]
    fn offset_route_falls_back_to_the_centre_ray() {
        let src = Outline {
            center: (0.0, 0.0),
            size: (10.0, 10.0),
            shape: NodeShape::Ellipse,
        };
        let dst = Outline {
            center: (100.0, 0.0),
            size: (10.0, 10.0),
            shape: NodeShape::Ellipse,
        };
        // The offset line runs clear of both circles, so only the centre ray can place the ends.
        let out = offset_route(&[(5.0, 0.0), (95.0, 0.0)], 20.0, src, dst);
        let on_border =
            |p: (f64, f64), c: (f64, f64)| (fp::hypot(p.0 - c.0, p.1 - c.1) - 5.0).abs() < 1e-9;
        let (first, last) = (out[0], out[out.len() - 1]);
        assert!(on_border(first, (0.0, 0.0)), "{first:?}");
        assert!(on_border(last, (100.0, 0.0)), "{last:?}");
    }

    #[test]
    fn rdp_drops_a_point_within_tolerance_and_keeps_a_corner() {
        assert_eq!(
            rdp(&[(0.0, 0.0), (5.0, 0.1), (10.0, 0.0)], 1.0),
            vec![(0.0, 0.0), (10.0, 0.0)]
        );
        assert_eq!(
            rdp(&[(0.0, 0.0), (5.0, 9.0), (10.0, 0.0)], 1.0),
            vec![(0.0, 0.0), (5.0, 9.0), (10.0, 0.0)]
        );
    }

    /// Runs maximally lopsided splits on a 512 KiB stack. 3 000 corners already exceed what
    /// recursion fits, and more would only make the quadratic rescan slow.
    #[test]
    fn rdp_survives_a_hundred_thousand_points_on_a_small_stack() {
        const N: usize = 100_000;
        const CORNERS: usize = 3_000;
        let out = std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(|| {
                // A shrinking zigzag then a straight run, so each split peels off one corner.
                let pts: Vec<Pt> = (0..N)
                    .map(|i| {
                        let s = if i % 2 == 0 { 1.0 } else { -1.0 };
                        (i as f64, s * (CORNERS.saturating_sub(i)) as f64)
                    })
                    .collect();
                rdp(&pts, 1.0)
            })
            .expect("spawn")
            .join()
            .expect("a 100 000-point polyline must not overflow a 512 KiB stack");
        assert_eq!(out.len(), CORNERS + 1);
        assert_eq!(out[0], (0.0, CORNERS as f64));
        assert_eq!(out[CORNERS], ((N - 1) as f64, 0.0));
    }

    #[test]
    fn rdp_matches_the_recursive_definition() {
        fn rdp_recursive(pts: &[Pt], tol: f64) -> Vec<Pt> {
            if pts.len() < 3 {
                return pts.to_vec();
            }
            let (a, b) = (pts[0], pts[pts.len() - 1]);
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len = f64::hypot(dx, dy).max(1e-9);
            let mut far = 0usize;
            let mut fmax = 0.0;
            for (i, &p) in pts.iter().enumerate().take(pts.len() - 1).skip(1) {
                let d = ((p.0 - a.0) * dy - (p.1 - a.1) * dx).abs() / len;
                if d > fmax {
                    fmax = d;
                    far = i;
                }
            }
            if fmax <= tol {
                return vec![a, b];
            }
            let mut left = rdp_recursive(&pts[..=far], tol);
            let right = rdp_recursive(&pts[far..], tol);
            left.pop();
            left.extend(right);
            left
        }

        let mut state: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((state >> 33) as u32 % 2_000) as f64 / 10.0 - 100.0
        };
        for case in 0..200 {
            let n = 2 + case % 60;
            let pts: Vec<Pt> = (0..n).map(|_| (next(), next())).collect();
            for &tol in &[0.0, 0.5, 2.0, 25.0] {
                assert_eq!(
                    rdp(&pts, tol),
                    rdp_recursive(&pts, tol),
                    "case {case}, tol {tol}, points {pts:?}"
                );
            }
        }
    }

    #[test]
    fn polyline_point_at_walks_the_length() {
        let pts = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)];
        assert_eq!(polyline_point_at(&pts, 0.0), (0.0, 0.0));
        assert_eq!(polyline_point_at(&pts, 0.5), (10.0, 0.0));
        assert_eq!(polyline_point_at(&pts, 1.0), (10.0, 10.0));
    }

    #[test]
    fn shorten_end_trims_the_last_segment() {
        assert_eq!(
            shorten_end(&[(0.0, 0.0), (10.0, 0.0)], 2.0),
            vec![(0.0, 0.0), (8.0, 0.0)]
        );
        // A trim longer than the segment stops half a pixel short of the corner.
        assert_eq!(
            shorten_end(&[(0.0, 0.0), (10.0, 0.0)], 100.0),
            vec![(0.0, 0.0), (0.5, 0.0)]
        );
    }
}

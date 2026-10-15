//! Metrics on which border each route end attaches to.
//!
//! An end on a border across the layout axis reads as an edge arriving from an odd direction. The
//! border is read off the geometry, so hand-written or loaded routes measure the same way as
//! engine output.

use super::{EdgeGeom, LayoutMetrics, NodeBox, Problem, SHARED_ENDPOINT_TOL};
use crate::geometry::fp;
use crate::geometry::shape::{leaves_inward, NodeShape, Outline};

/// How many times longer along one axis than the other a last segment must be to count as
/// axis-aligned.
const AXIS_ALIGNED_RATIO: f64 = 4.0;

/// Fraction of the half-border past which an end counts as off centre. On a circle, a third of
/// the radius is where the stroke meets the curve visibly askew (about 20 degrees).
const OFF_CENTRE: f64 = 1.0 / 3.0;

/// Fills [`LayoutMetrics::perpendicular_ends`], [`LayoutMetrics::off_centre_ends`],
/// [`LayoutMetrics::inward_ends`] and [`LayoutMetrics::converged_ends`]. Self-loops are skipped.
pub(super) fn fill_port_metrics(
    problem: &Problem,
    edges: &[EdgeGeom],
    boxes: &[NodeBox],
    m: &mut LayoutMetrics,
) {
    let mut count = 0;
    let mut off_centre = 0;
    let mut inward = 0;
    let mut ends: Vec<(usize, [f64; 2])> = Vec::new();
    for e in edges {
        if e.from == e.to {
            continue;
        }
        let n = e.pts.len();
        for (node, end, inner) in [(e.from, 0usize, 1usize), (e.to, n - 1, n - 2)] {
            let Some(nb) = boxes.get(node) else { continue };
            // Skip sub-pixel stubs that endpoint settling can leave behind.
            let toward = (0..n)
                .map(|k| if end == 0 { k } else { n - 1 - k })
                .skip(1)
                .find(|&k| {
                    let d = [e.pts[k][0] - e.pts[end][0], e.pts[k][1] - e.pts[end][1]];
                    fp::hypot(d[0], d[1]) > 0.5
                })
                .unwrap_or(inner);
            if end_is_cross_axis(e.pts[end], e.pts[toward], nb, problem.left_right) {
                count += 1;
            }
            if end_is_off_centre(e.pts[end], e.pts[toward], nb) {
                off_centre += 1;
            }
            let outline = Outline {
                center: (nb.cx, nb.cy),
                size: (2.0 * nb.hw, 2.0 * nb.hh),
                shape: if nb.ellipse {
                    NodeShape::Ellipse
                } else {
                    NodeShape::Box
                },
            };
            let at = |k: usize| (e.pts[k][0], e.pts[k][1]);
            if toward != end && leaves_inward(outline, at(end), at(toward)) {
                inward += 1;
            }
            ends.push((node, e.pts[end]));
        }
    }
    m.perpendicular_ends = count;
    m.off_centre_ends = off_centre;
    m.inward_ends = inward;
    m.converged_ends = ends
        .iter()
        .enumerate()
        .filter(|&(i, &(n, p))| {
            ends.iter().enumerate().any(|(j, &(n2, q))| {
                j != i && n2 == n && fp::hypot(p[0] - q[0], p[1] - q[1]) <= SHARED_ENDPOINT_TOL
            })
        })
        .count();
}

/// Whether the end at `p`, continuing toward `q`, sits more than [`OFF_CENTRE`] of the
/// half-border from its border's middle. A diagonal last segment names no border and is not judged.
fn end_is_off_centre(p: [f64; 2], q: [f64; 2], nb: &NodeBox) -> bool {
    let (dx, dy) = ((q[0] - p[0]).abs(), (q[1] - p[1]).abs());
    if dy > dx * AXIS_ALIGNED_RATIO {
        (p[0] - nb.cx).abs() > OFF_CENTRE * nb.hw
    } else if dx > dy * AXIS_ALIGNED_RATIO {
        (p[1] - nb.cy).abs() > OFF_CENTRE * nb.hh
    } else {
        false
    }
}

/// Whether the end at `p`, continuing toward `q`, attaches to a border of `nb` across the layout
/// axis (top/bottom in a left-to-right drawing, left/right in a top-to-bottom one).
///
/// An axis-aligned last segment names the border. For a diagonal one the point decides, with the
/// router's rule. On a box it is the nearest side, and on an ellipse the side its centre ray exits.
fn end_is_cross_axis(p: [f64; 2], q: [f64; 2], nb: &NodeBox, left_right: bool) -> bool {
    let (dx, dy) = ((q[0] - p[0]).abs(), (q[1] - p[1]).abs());
    let vertical_segment = if dy > dx * AXIS_ALIGNED_RATIO {
        Some(true)
    } else if dx > dy * AXIS_ALIGNED_RATIO {
        Some(false)
    } else {
        None
    };
    let on_top_or_bottom = match vertical_segment {
        Some(v) => v,
        None if nb.ellipse => {
            let (rx, ry) = (p[0] - nb.cx, p[1] - nb.cy);
            ry.abs() * nb.hw > rx.abs() * nb.hh
        }
        None => {
            let to_side = (p[0] - nb.left()).abs().min((p[0] - nb.right()).abs());
            let to_top_bottom = (p[1] - nb.top()).abs().min((p[1] - nb.bottom()).abs());
            to_top_bottom < to_side
        }
    };
    on_top_or_bottom == left_right
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::super::tests::compute;
    use super::super::tests::{boxes, layout, spec};

    /// Nodes are 100x50. In a left-to-right drawing an end arriving vertically is on a top or
    /// bottom border and counts. One arriving horizontally is on a facing border and does not.
    #[test]
    fn a_vertical_arrival_in_a_left_right_drawing_is_perpendicular() {
        let s = spec(boxes(3), vec![(0, 1), (0, 2)], "LR");
        let l = layout(
            vec![[0.0, 0.0], [400.0, 0.0], [400.0, 300.0]],
            vec![
                // Straight across between facing borders.
                vec![[50.0, 0.0], [350.0, 0.0]],
                // Out of the source's right border, then down into the target's top.
                vec![[50.0, 0.0], [400.0, 0.0], [400.0, 275.0]],
            ],
        );
        let m = compute(&s, &l);
        assert_eq!(m.perpendicular_ends, 1, "{m:?}");
    }

    /// The same geometry read as a top-to-bottom drawing swaps which borders are the odd ones.
    #[test]
    fn the_layout_direction_decides_which_borders_are_perpendicular() {
        let s = spec(boxes(2), vec![(0, 1)], "TB");
        // Leaves the source's bottom border, then enters the target from its left.
        let l = layout(
            vec![[0.0, 0.0], [300.0, 400.0]],
            vec![vec![[0.0, 25.0], [0.0, 400.0], [250.0, 400.0]]],
        );
        assert_eq!(compute(&s, &l).perpendicular_ends, 1);
    }

    /// A diagonal last segment names no border, so the point decides. On a box it is the side the
    /// point sits on, and on an ellipse the side its centre ray exits through.
    #[test]
    fn a_diagonal_end_is_classified_by_the_border_it_sits_on() {
        let s = spec(boxes(2), vec![(0, 1)], "LR");
        // On the right border of the source (x = 50) and on the bottom border of the target.
        let l = layout(
            vec![[0.0, 0.0], [400.0, 300.0]],
            vec![vec![[50.0, 10.0], [390.0, 275.0]]],
        );
        assert_eq!(compute(&s, &l).perpendicular_ends, 1);
    }

    /// Self-loops and unrouted edges are not judged at all.
    #[test]
    fn loops_and_unrouted_edges_are_skipped() {
        let s = spec(boxes(2), vec![(0, 0), (0, 1)], "LR");
        let l = layout(
            vec![[0.0, 0.0], [400.0, 0.0]],
            vec![
                vec![[0.0, -25.0], [0.0, -60.0], [30.0, -60.0], [30.0, -25.0]],
                vec![[50.0, 0.0]],
            ],
        );
        assert_eq!(compute(&s, &l).perpendicular_ends, 0);
    }
}

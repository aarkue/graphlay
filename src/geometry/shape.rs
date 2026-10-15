//! Node outlines: the border shape a node is drawn with, and where a ray meets that border.

use super::{DEGENERATE, EPS};
use crate::geometry::fp;

/// A node's drawn outline: centre, full size and border shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Outline {
    /// Centre of the node in layout space.
    pub center: (f64, f64),
    /// Full `(width, height)` of its bounding box.
    pub size: (f64, f64),
    /// Which border the outline follows.
    pub shape: NodeShape,
}

/// Fraction of its half-extent an ellipse spans at perpendicular offset `o` from the centre, where
/// `half` is the half-extent along the offset's axis. Returns `1.0` when `half <= 0`, and `0.0`
/// past the extent.
pub fn ellipse_span(o: f64, half: f64) -> f64 {
    if half <= 0.0 {
        1.0
    } else {
        (1.0 - fp::sq(o / half)).max(0.0).sqrt()
    }
}

/// Border shape of a node, public as [`crate::geometry::Shape`]. Edge ends are projected onto it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum NodeShape {
    /// The node's bounding rectangle is its border.
    #[default]
    Box,
    /// The ellipse inscribed in the node's bounding rectangle is its border.
    Ellipse,
}

/// Unsigned distance from `p` to `outline`'s border, `0` on it and positive inside and outside.
/// For an ellipse it is measured along the centre-to-`p` ray, exact for a circle and an upper bound
/// otherwise. Returns `0` for a degenerate outline.
pub fn outline_distance(p: (f64, f64), outline: Outline) -> f64 {
    let (hw, hh) = (outline.size.0 / 2.0, outline.size.1 / 2.0);
    let (dx, dy) = (p.0 - outline.center.0, p.1 - outline.center.1);
    if hw <= 0.0 || hh <= 0.0 {
        return 0.0;
    }
    match outline.shape {
        NodeShape::Ellipse => {
            let r = fp::hypot(dx / hw, dy / hh);
            if r < EPS {
                return hw.min(hh);
            }
            let k = 1.0 - 1.0 / r;
            fp::hypot(dx * k, dy * k)
        }
        NodeShape::Box => {
            let (ox, oy) = (dx.abs() - hw, dy.abs() - hh);
            if ox > 0.0 || oy > 0.0 {
                fp::hypot(ox.max(0.0), oy.max(0.0))
            } else {
                (-ox).min(-oy)
            }
        }
    }
}

/// Border point of `outline` on the ray from its centre toward `aim`. It hits where
/// [`project_onto_outline`] misses, and is `None` only for a degenerate outline or `aim` at centre.
pub fn border_toward(outline: Outline, aim: (f64, f64)) -> Option<(f64, f64)> {
    project_onto_outline(aim, outline.center, outline)
}

/// Cosine below which a segment leaving a border counts as heading back into the node's side.
/// About 12 degrees past the border's tangent, so a route sliding along its border does not count.
const INWARD_COS: f64 = -0.2;

/// Whether a route attached to `outline` at `end` heads back across that border toward `next`
/// (e.g., leaves the right face and turns left), which makes it read as a wrong-side edge.
pub(crate) fn leaves_inward(outline: Outline, end: (f64, f64), next: (f64, f64)) -> bool {
    let (hw, hh) = (outline.size.0 / 2.0, outline.size.1 / 2.0);
    if hw <= EPS || hh <= EPS {
        return false;
    }
    let (u, v) = (
        (end.0 - outline.center.0) / hw,
        (end.1 - outline.center.1) / hh,
    );
    let normal = match outline.shape {
        NodeShape::Ellipse => (u / hw, v / hh),
        NodeShape::Box if u.abs() > v.abs() + 0.02 => (u.signum(), 0.0),
        NodeShape::Box if v.abs() > u.abs() + 0.02 => (0.0, v.signum()),
        NodeShape::Box => (u.signum(), v.signum()),
    };
    let dir = (next.0 - end.0, next.1 - end.1);
    let (nl, dl) = (fp::hypot(normal.0, normal.1), fp::hypot(dir.0, dir.1));
    if nl <= DEGENERATE || dl <= DEGENERATE {
        return false;
    }
    (normal.0 * dir.0 + normal.1 * dir.1) / (nl * dl) < INWARD_COS
}

/// Moves `end` along the `inner`->`end` line onto `outline`'s border, picking the crossing nearest
/// `end`. `None` when the line misses the node or `inner == end`.
pub fn project_onto_outline(
    end: (f64, f64),
    inner: (f64, f64),
    outline: Outline,
) -> Option<(f64, f64)> {
    let c = outline.center;
    let (hw, hh) = (outline.size.0 / 2.0, outline.size.1 / 2.0);
    let d = (end.0 - inner.0, end.1 - inner.1);
    let len = fp::hypot(d.0, d.1);
    if len < EPS {
        return None;
    }
    let dir = (d.0 / len, d.1 / len);
    match outline.shape {
        NodeShape::Ellipse => {
            // Solve |(inner + t*dir - c) / (hw, hh)|^2 = 1 for t, measured from `inner`.
            let ox = (inner.0 - c.0) / hw;
            let oy = (inner.1 - c.1) / hh;
            let dx = dir.0 / hw;
            let dy = dir.1 / hh;
            let a = dx * dx + dy * dy;
            let b = 2.0 * (ox * dx + oy * dy);
            let cc = ox * ox + oy * oy - 1.0;
            let disc = b * b - 4.0 * a * cc;
            if disc < 0.0 || a.abs() < DEGENERATE {
                return None;
            }
            let sq = disc.sqrt();
            let (t1, t2) = ((-b - sq) / (2.0 * a), (-b + sq) / (2.0 * a));
            let t = if (t1 - len).abs() <= (t2 - len).abs() {
                t1
            } else {
                t2
            };
            Some((inner.0 + t * dir.0, inner.1 + t * dir.1))
        }
        NodeShape::Box => {
            let mut best: Option<((f64, f64), f64)> = None;
            for (coord, vertical) in [
                (c.0 - hw, true),
                (c.0 + hw, true),
                (c.1 - hh, false),
                (c.1 + hh, false),
            ] {
                let (o, dd, cross_o, cross_d, span_c, span_h) = if vertical {
                    (inner.0, dir.0, inner.1, dir.1, c.1, hh)
                } else {
                    (inner.1, dir.1, inner.0, dir.0, c.0, hw)
                };
                if dd.abs() < EPS {
                    continue;
                }
                let t = (coord - o) / dd;
                let along = cross_o + t * cross_d;
                if (along - span_c).abs() > span_h + 0.5 {
                    continue;
                }
                let p = (inner.0 + t * dir.0, inner.1 + t * dir.1);
                let dist = fp::hypot(p.0 - end.0, p.1 - end.1);
                if best.is_none_or(|(_, bd)| dist < bd) {
                    best = Some((p, dist));
                }
            }
            best.map(|(p, _)| p)
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// A 100x40 box at the origin.
    fn box_outline() -> Outline {
        Outline {
            center: (0.0, 0.0),
            size: (100.0, 40.0),
            shape: NodeShape::Box,
        }
    }

    #[test]
    fn an_end_that_turns_back_across_its_border_leaves_inward() {
        let o = box_outline();
        // Leaving the right face and heading left under the node is inward.
        assert!(leaves_inward(o, (50.0, 18.0), (-10.0, 80.0)));
        // Leaving the right face to the right, or sliding along it, is not.
        assert!(!leaves_inward(o, (50.0, 18.0), (90.0, 60.0)));
        assert!(!leaves_inward(o, (50.0, 0.0), (50.0, 60.0)));
        // Leaving the bottom face downwards and to the left is not inward.
        assert!(!leaves_inward(o, (40.0, 20.0), (-60.0, 80.0)));
        let circle = Outline {
            shape: NodeShape::Ellipse,
            size: (40.0, 40.0),
            ..o
        };
        assert!(leaves_inward(circle, (20.0, 0.0), (-20.0, 30.0)));
        assert!(!leaves_inward(circle, (0.0, 20.0), (-20.0, 60.0)));
    }

    #[test]
    fn ellipse_span_is_the_unit_circle() {
        assert_eq!(ellipse_span(0.0, 5.0), 1.0);
        assert_eq!(ellipse_span(3.0, 5.0), 0.8);
        assert_eq!(ellipse_span(5.0, 5.0), 0.0);
        assert_eq!(ellipse_span(9.0, 5.0), 0.0);
        // A zero-extent axis spans fully, so a dummy never collapses a projection.
        assert_eq!(ellipse_span(1.0, 0.0), 1.0);
    }

    fn outline(shape: NodeShape) -> Outline {
        Outline {
            center: (0.0, 0.0),
            size: (10.0, 10.0),
            shape,
        }
    }

    #[test]
    fn project_onto_outline_lands_on_the_border() {
        for shape in [NodeShape::Box, NodeShape::Ellipse] {
            let p = project_onto_outline((20.0, 0.0), (0.0, 0.0), outline(shape))
                .unwrap_or_else(|| panic!("{shape:?} must have a border point"));
            assert!((p.0 - 5.0).abs() < 1e-12 && p.1 == 0.0, "{shape:?}: {p:?}");
        }
    }

    /// At 45 degrees the circle's border point is (r/sqrt2, r/sqrt2), not the box corner.
    #[test]
    fn project_onto_outline_follows_the_ellipse() {
        let p =
            project_onto_outline((20.0, 20.0), (0.0, 0.0), outline(NodeShape::Ellipse)).unwrap();
        let r = 5.0 / 2.0f64.sqrt();
        assert!((p.0 - r).abs() < 1e-9 && (p.1 - r).abs() < 1e-9, "{p:?}");
    }

    #[test]
    fn border_toward_always_finds_a_point() {
        for shape in [NodeShape::Box, NodeShape::Ellipse] {
            let p = border_toward(outline(shape), (100.0, 0.0))
                .unwrap_or_else(|| panic!("{shape:?} must have a border point"));
            assert!(
                (p.0 - 5.0).abs() < 1e-9 && p.1.abs() < 1e-9,
                "{shape:?}: {p:?}"
            );
            assert!(border_toward(outline(shape), (0.0, -1000.0)).is_some());
        }
        assert_eq!(border_toward(outline(NodeShape::Box), (0.0, 0.0)), None);
    }

    #[test]
    fn outline_distance_measures_both_ways() {
        for shape in [NodeShape::Box, NodeShape::Ellipse] {
            let o = outline(shape);
            assert!(outline_distance((5.0, 0.0), o) < 1e-9, "{shape:?}");
            assert!(
                (outline_distance((8.0, 0.0), o) - 3.0).abs() < 1e-9,
                "{shape:?}"
            );
            assert!(
                (outline_distance((3.0, 0.0), o) - 2.0).abs() < 1e-9,
                "{shape:?}"
            );
        }
    }

    #[test]
    fn project_onto_outline_misses_return_none() {
        let far = Outline {
            center: (1000.0, 1000.0),
            size: (10.0, 10.0),
            shape: NodeShape::Box,
        };
        assert_eq!(project_onto_outline((20.0, 0.0), (0.0, 0.0), far), None);
        assert_eq!(
            project_onto_outline((1.0, 1.0), (1.0, 1.0), outline(NodeShape::Box)),
            None
        );
    }
}

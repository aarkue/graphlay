//! Ports, where an arc meets a node's border, and their projection onto that border.
//!
//! Side selection and port spreading live in `route::sides` and `route::spread`.

use crate::geometry::shape::ellipse_span;
use crate::geometry::{Boxes, Side};

/// A resolved port: where the arc meets the node border and a stub just outside it.
#[derive(Clone, Copy)]
pub struct Port {
    pub anchor: (f64, f64),
    pub stub: (f64, f64),
    pub side: Side,
}

/// Share of a border's half-extent a port offset may use.
pub(super) const PORT_SPAN: f64 = 0.95;

/// Largest offset from the centre of `side`'s border that [`project_port`] allows.
///
/// A port must stay on its border, and an ellipse's outline has no width at the end of its span.
pub fn port_offset_limit(node: usize, side: Side, b: &Boxes) -> f64 {
    let half = if side.horizontal() {
        b.order_w[node] / 2.0
    } else {
        b.layer_w[node] / 2.0
    };
    half * PORT_SPAN
}

/// Projects a port onto `node`'s border on `side` at signed offset `off`, clamped to
/// [`port_offset_limit`], with a straight stub of length `stub`. On an ellipse the anchor lands on
/// the outline and the stub starts just outside the bounding box.
pub fn project_port(node: usize, side: Side, off: f64, b: &Boxes, stub: f64) -> Port {
    let (cxn, cyn) = (b.cx[node], b.cy[node]);
    let (hw, hh) = (b.layer_w[node] / 2.0, b.order_w[node] / 2.0);
    let ell = b.ellipse[node];
    let lim = port_offset_limit(node, side, b);
    let ell_span = ellipse_span;
    let (anchor, stub) = match side {
        Side::Right => {
            let o = off.clamp(-lim, lim);
            let ax = if ell {
                cxn + hw * ell_span(o, hh)
            } else {
                cxn + hw
            };
            ((ax, cyn + o), (cxn + hw + stub, cyn + o))
        }
        Side::Left => {
            let o = off.clamp(-lim, lim);
            let ax = if ell {
                cxn - hw * ell_span(o, hh)
            } else {
                cxn - hw
            };
            ((ax, cyn + o), (cxn - hw - stub, cyn + o))
        }
        Side::Top => {
            let o = off.clamp(-lim, lim);
            let ay = if ell {
                cyn - hh * ell_span(o, hw)
            } else {
                cyn - hh
            };
            ((cxn + o, ay), (cxn + o, cyn - hh - stub))
        }
        Side::Bottom => {
            let o = off.clamp(-lim, lim);
            let ay = if ell {
                cyn + hh * ell_span(o, hw)
            } else {
                cyn + hh
            };
            ((cxn + o, ay), (cxn + o, cyn + hh + stub))
        }
    };
    Port { anchor, stub, side }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::geometry::Boxes;

    fn boxes() -> Boxes {
        Boxes {
            cx: vec![100.0],
            cy: vec![50.0],
            layer_w: vec![120.0],
            order_w: vec![40.0],
            ellipse: vec![false],
            n_real: 1,
        }
    }

    /// [`port_offset_limit`] is exactly the offset [`project_port`] clamps to.
    #[test]
    fn port_offset_limit_is_what_project_port_clamps_to() {
        let b = boxes();
        for (side, along_border) in [
            (Side::Right, false),
            (Side::Left, false),
            (Side::Top, true),
            (Side::Bottom, true),
        ] {
            let lim = port_offset_limit(0, side, &b);
            let centre = if along_border { b.cx[0] } else { b.cy[0] };
            for want in [1000.0, -1000.0] {
                let p = project_port(0, side, want, &b, 15.0);
                let got = if along_border { p.anchor.0 } else { p.anchor.1 };
                assert!(
                    (got - (centre + lim * want.signum())).abs() < 1e-9,
                    "{side:?}: limit {lim} but the port landed at {got}"
                );
            }
        }
        // Right borders span the order axis and bottom borders the layer axis.
        assert_eq!(port_offset_limit(0, Side::Right, &b), 19.0);
        assert_eq!(port_offset_limit(0, Side::Bottom, &b), 57.0);
    }
}

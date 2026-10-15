//! Port hints in the router. [`crate::Edge::source_port`] and [`crate::Edge::target_port`] are
//! resolved onto routed arc ends, and an end whose border faces away from its route gets a
//! go-around out of its own node.
//!
//! [`Hints::of`] translates output-space hints to flow-space arc ends, flipping the end if the arc
//! was turned by [`LayeredState::ranked_backwards`] or
//! [`crate::layered::state::LArc::reversed`] (two turns cancel). Later stages leave a
//! [`Hints::pinned`] end in place and keep a side-hinted one on its border.

use crate::geometry::fp;
use std::collections::BTreeMap;

use super::{PortMap, RouteCtx};
use crate::geometry::hit::{seg_hits_box, seg_hits_one_box};
use crate::geometry::polyline::simplify_collinear;
use crate::geometry::shape::outline_distance;
use crate::geometry::{Boxes, Side};
use crate::layered::ports::{port_offset_limit, Port};
use crate::layered::state::LayeredState;

/// Clearance of a hinted route from its own node's box, apart from the end segment. Above the
/// half pixel collinear simplification merges, so a run along the border is never folded in.
const OWN_CLEAR: f64 = 1.0;

/// One hinted arc end in flow space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::layered) struct EndHint {
    pub(in crate::layered) node: usize,
    pub(in crate::layered) side: Side,
    /// Signed distance from the border's middle, clamped to [`port_offset_limit`].
    pub(in crate::layered) offset: Option<f64>,
}

/// Hinted arc ends keyed by `(arc, is_source)`, where the source is the lower-layer end.
#[derive(Default)]
pub(in crate::layered) struct Hints {
    ends: BTreeMap<(usize, bool), EndHint>,
}

impl Hints {
    #[cfg(test)]
    pub(super) fn none() -> Self {
        Hints::default()
    }

    pub(in crate::layered) fn of(st: &LayeredState, b: &Boxes) -> Self {
        let mut ends = BTreeMap::new();
        if st.port_hints.is_empty() {
            return Hints { ends };
        }
        // All arcs of an edge turn together.
        let mut turned = vec![false; st.port_hints.len()];
        for a in st.arcs.iter().filter(|a| a.reversed) {
            if let Some(t) = turned.get_mut(a.edge) {
                *t = true;
            }
        }
        for (&edge, chain) in &st.chains {
            let (Some(hint), Some(&first), Some(&last)) =
                (st.port_hints.get(edge), chain.first(), chain.last())
            else {
                continue;
            };
            if chain.len() < 2 {
                continue;
            }
            // The route starts at the caller's source unless exactly one reversal turned it.
            let backwards = st.ranked_backwards.get(edge).copied().unwrap_or(false);
            let turned = turned.get(edge).copied().unwrap_or(false);
            let (low, high) = if turned == backwards {
                (hint[0], hint[1])
            } else {
                (hint[1], hint[0])
            };
            for (is_source, hint, node) in [(true, low, first), (false, high, last)] {
                let Some(hint) = hint else {
                    continue;
                };
                if node >= b.n_real {
                    continue;
                }
                let side = st.axis.to_flow_side(hint.side());
                let offset = hint.offset().map(|t| {
                    let half = if side.horizontal() {
                        b.order_w[node]
                    } else {
                        b.layer_w[node]
                    } / 2.0;
                    let lim = port_offset_limit(node, side, b);
                    (t * half).clamp(-lim, lim)
                });
                ends.insert((edge, is_source), EndHint { node, side, offset });
            }
        }
        Hints { ends }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    pub(super) fn get(&self, arc: usize, is_source: bool) -> Option<EndHint> {
        self.ends.get(&(arc, is_source)).copied()
    }

    /// Where along the order axis the hinted end of `edge` at `node` attaches, relative to the
    /// node's centre. Zero for an unhinted end or one on a perpendicular border.
    pub(in crate::layered) fn order_offset(&self, edge: usize, node: usize) -> f64 {
        [true, false]
            .into_iter()
            .filter_map(|is_source| self.get(edge, is_source))
            .find(|h| h.node == node && h.side.horizontal())
            .and_then(|h| h.offset)
            .unwrap_or(0.0)
    }

    /// An end with an offset hint is pinned, and no stage may move it.
    pub(super) fn pinned(&self, arc: usize, is_source: bool) -> bool {
        self.get(arc, is_source).is_some_and(|h| h.offset.is_some())
    }

    pub(super) fn hinted(&self, arc: usize) -> bool {
        self.get(arc, true).is_some() || self.get(arc, false).is_some()
    }

    /// Whether `p` lies on the hinted border's span of the outline. True for an unhinted end.
    pub(super) fn on_border(&self, arc: usize, is_source: bool, p: (f64, f64), b: &Boxes) -> bool {
        let Some(h) = self.get(arc, is_source) else {
            return true;
        };
        let Some(outline) = b.outline(h.node) else {
            return true;
        };
        let (dx, dy) = (p.0 - b.cx[h.node], p.1 - b.cy[h.node]);
        let (normal, along) = match h.side {
            Side::Right => (dx, dy),
            Side::Left => (-dx, dy),
            Side::Bottom => (dy, dx),
            Side::Top => (-dy, dx),
        };
        outline_distance(p, outline) <= 0.5
            && normal > 0.0
            && along.abs() <= port_offset_limit(h.node, h.side, b) + 0.5
    }

    pub(super) fn cuts_own_boxes(&self, arc: usize, route: &[(f64, f64)], b: &Boxes) -> bool {
        [true, false].into_iter().any(|is_source| {
            self.get(arc, is_source)
                .is_some_and(|h| cuts_own_box(route, is_source, h.node, h.side, b))
        })
    }

    /// Whether moving `old` to `new` keeps straight-out hinted ends straight and cuts no hinted
    /// node it did not already cut. Shifting a run that is not quite axis-aligned can tilt an end.
    pub(super) fn keeps_ends_straight(
        &self,
        arc: usize,
        old: &[(f64, f64)],
        new: &[(f64, f64)],
        b: &Boxes,
    ) -> bool {
        [true, false].into_iter().all(|is_source| {
            self.get(arc, is_source).is_none_or(|h| {
                !straight_out(old, is_source, h.side) || straight_out(new, is_source, h.side)
            })
        }) && (!self.hinted(arc)
            || !self.cuts_own_boxes(arc, new, b)
            || self.cuts_own_boxes(arc, old, b))
    }

    /// Whether `p -> q` comes within [`OWN_CLEAR`] of a hinted end's node, other than as the end
    /// segment leaving straight out. On an ellipse the end segment counts too.
    pub(super) fn seg_cuts_own(&self, arc: usize, p: (f64, f64), q: (f64, f64), b: &Boxes) -> bool {
        [true, false].into_iter().any(|is_source| {
            self.get(arc, is_source).is_some_and(|h| {
                seg_hits_one_box(p, q, b, h.node, OWN_CLEAR)
                    && !leaves_border(p, q, h, b)
                    && !leaves_border(q, p, h, b)
            })
        })
    }
}

/// Whether the head (else tail) end segment runs outward along `side`'s normal.
fn straight_out(route: &[(f64, f64)], head: bool, side: Side) -> bool {
    let n = route.len();
    if n < 2 {
        return false;
    }
    let (p, q) = if head {
        (route[0], route[1])
    } else {
        (route[n - 1], route[n - 2])
    };
    let (dx, dy) = (q.0 - p.0, q.1 - p.1);
    let (out, across) = match side {
        Side::Right => (dx, dy),
        Side::Left => (-dx, dy),
        Side::Bottom => (dy, dx),
        Side::Top => (-dy, dx),
    };
    out >= 0.5 && across.abs() < 0.5
}

fn leaves_border(p: (f64, f64), q: (f64, f64), h: EndHint, b: &Boxes) -> bool {
    let n = h.node;
    let (dx, dy) = (q.0 - p.0, q.1 - p.1);
    let (off, out, across) = match h.side {
        Side::Right => (p.0 - (b.cx[n] + b.layer_w[n] / 2.0), dx, dy),
        Side::Left => (b.cx[n] - b.layer_w[n] / 2.0 - p.0, -dx, dy),
        Side::Bottom => (p.1 - (b.cy[n] + b.order_w[n] / 2.0), dy, dx),
        Side::Top => (b.cy[n] - b.order_w[n] / 2.0 - p.1, -dy, dx),
    };
    off.abs() <= 0.5 && out >= 0.5 && across.abs() < 0.5
}

/// Outward unit normal of `side` in flow space (`Bottom` is `+cross`).
fn normal(side: Side) -> (f64, f64) {
    match side {
        Side::Right => (1.0, 0.0),
        Side::Left => (-1.0, 0.0),
        Side::Bottom => (0.0, 1.0),
        Side::Top => (0.0, -1.0),
    }
}

/// Whether `route` cuts its own end node, because the end segment does not head outward or a
/// later segment comes within [`OWN_CLEAR`]. The end segment is judged by direction because an
/// anchor on an ellipse sits inside the bounding box.
pub(super) fn cuts_own_box(
    route: &[(f64, f64)],
    head: bool,
    node: usize,
    side: Side,
    b: &Boxes,
) -> bool {
    let n = route.len();
    if n < 2 {
        return false;
    }
    let at = |k: usize| if head { route[k] } else { route[n - 1 - k] };
    let (p, q) = (at(0), at(1));
    let out = normal(side);
    // Less than half a pixel out is a run along the border.
    if (q.0 - p.0) * out.0 + (q.1 - p.1) * out.1 < 0.5 {
        return true;
    }
    (1..n - 1).any(|k| seg_hits_one_box(at(k), at(k + 1), b, node, OWN_CLEAR))
}

/// Route every hinted end that cuts its own node round it. Tries [`escape`] per end, then
/// [`rebuild`] for the whole route when that fails.
pub(super) fn leave_own_boxes(ctx: &RouteCtx, port: &PortMap, routes: &mut [Vec<(f64, f64)>]) {
    if ctx.hints.is_empty() {
        return;
    }
    let b = ctx.b;
    for (arc, chain) in ctx.ordered() {
        if chain.len() < 2 || routes[arc].len() < 2 {
            continue;
        }
        let (lo, hi) = (chain[0], chain[chain.len() - 1]);
        let broken = |r: &[(f64, f64)]| ctx.hints.cuts_own_boxes(arc, r, b);
        if !broken(&routes[arc]) {
            continue;
        }
        let mut r = routes[arc].clone();
        for (is_source, other) in [(true, hi), (false, lo)] {
            let Some(h) = ctx.hints.get(arc, is_source) else {
                continue;
            };
            if !cuts_own_box(&r, is_source, h.node, h.side, b) {
                continue;
            }
            if let Some(e) = escape(&r, is_source, h, other, ctx.stub_len, b) {
                r = e;
            }
        }
        if broken(&r) {
            if let (Some(&ps), Some(&pt)) = (port.get(&(arc, true)), port.get(&(arc, false))) {
                if let Some(whole) = rebuild((ps, lo), (pt, hi), ctx.stub_len, b) {
                    r = whole;
                }
            }
        }
        routes[arc] = r;
    }
}

/// The ring a stub's length out round one node's box.
struct Ring {
    anchor: (f64, f64),
    stub: (f64, f64),
    /// Corners clockwise in flow space (y down) from the top left.
    corners: [(f64, f64); 4],
    /// Edge `e` runs from corner `e` to corner `e + 1`.
    edge: usize,
}

impl Ring {
    fn new(node: usize, side: Side, anchor: (f64, f64), clear: f64, b: &Boxes) -> Self {
        let (cx, cy) = (b.cx[node], b.cy[node]);
        let (hw, hh) = (b.layer_w[node] / 2.0, b.order_w[node] / 2.0);
        let (x0, x1, y0, y1) = (
            cx - hw - clear,
            cx + hw + clear,
            cy - hh - clear,
            cy + hh + clear,
        );
        let (stub, edge) = match side {
            Side::Top => ((anchor.0, y0), 0),
            Side::Right => ((x1, anchor.1), 1),
            Side::Bottom => ((anchor.0, y1), 2),
            Side::Left => ((x0, anchor.1), 3),
        };
        Ring {
            anchor,
            stub,
            corners: [(x0, y0), (x1, y0), (x1, y1), (x0, y1)],
            edge,
        }
    }

    /// Paths from the anchor via the stub end round zero to three corners, either way round.
    fn ways(&self) -> Vec<Vec<(f64, f64)>> {
        let mut out = vec![vec![self.anchor, self.stub]];
        for clockwise in [true, false] {
            for k in 1..4usize {
                let mut path = vec![self.anchor, self.stub];
                for i in 0..k {
                    let ci = if clockwise {
                        (self.edge + 1 + i) % 4
                    } else {
                        (self.edge + 4 - i) % 4
                    };
                    path.push(self.corners[ci]);
                }
                out.push(path);
            }
        }
        out
    }
}

fn length(p: &[(f64, f64)]) -> f64 {
    p.windows(2)
        .map(|v| fp::hypot(v[1].0 - v[0].0, v[1].1 - v[0].1))
        .sum()
}

/// `(box hits, folds, length)`, lower is better.
type Score = (usize, usize, f64);

type Best = Option<(Score, Vec<(f64, f64)>)>;

fn better(a: Score, b: Score) -> bool {
    a.0.cmp(&b.0)
        .then(a.1.cmp(&b.1))
        .then(a.2.total_cmp(&b.2))
        .is_lt()
}

/// Reroute the head (else tail) end round its node `h.node` to the first point from which the
/// rest of the route is clear, keeping the port. Candidates are ranked by [`Score`]. `None` when
/// nothing clears the node.
fn escape(
    route: &[(f64, f64)],
    head: bool,
    h: EndHint,
    other: usize,
    stub: f64,
    b: &Boxes,
) -> Option<Vec<(f64, f64)>> {
    let node = h.node;
    let mut w: Vec<(f64, f64)> = route.to_vec();
    if !head {
        w.reverse();
    }
    let (cx, cy) = (b.cx[node], b.cy[node]);
    let (hw, hh) = (b.layer_w[node] / 2.0, b.order_w[node] / 2.0);
    let c = stub.max(1.0);
    let ring = Ring::new(node, h.side, w[0], c, b);
    // A leg along the border line would be folded into the end segment by the simplification.
    let cuts = |p: (f64, f64), q: (f64, f64)| seg_hits_one_box(p, q, b, node, OWN_CLEAR);
    let inside =
        |p: (f64, f64)| (p.0 - cx).abs() <= hw + OWN_CLEAR && (p.1 - cy).abs() <= hh + OWN_CLEAR;
    // The far node may have a go-around of its own to keep.
    let cuts_far = |p: (f64, f64), q: (f64, f64)| seg_hits_one_box(p, q, b, other, -1.0);
    // Rejoin at least a stub before the far anchor so the far end keeps its own approach.
    let n = w.len();
    let (z, before_z) = (w[n - 1], w[n - 2]);
    let last_len = fp::hypot(z.0 - before_z.0, z.1 - before_z.1);
    if last_len > c + 1.0 {
        let f = c / last_len;
        w.insert(
            n - 1,
            (z.0 + (before_z.0 - z.0) * f, z.1 + (before_z.1 - z.1) * f),
        );
    }
    // Judge whole routes, so a later rejoin point that saves a long way round can win.
    let ways = ring.ways();
    let mut best: Best = None;
    for j in 1..w.len() - 1 {
        if inside(w[j]) || w[j..].windows(2).any(|v| cuts(v[0], v[1])) {
            continue;
        }
        let t = w[j];
        let next = w.get(j + 1).copied();
        for way in &ways {
            let last = way[way.len() - 1];
            for mid in [(t.0, last.1), (last.0, t.1)] {
                let mut p = way.clone();
                p.push(mid);
                p.push(t);
                if p[1..]
                    .windows(2)
                    .any(|v| cuts(v[0], v[1]) || cuts_far(v[0], v[1]))
                {
                    continue;
                }
                // Arriving against the route's onward direction draws the leg on top of it.
                let fold = next.is_some_and(|n| {
                    let (d1, d2) = ((t.0 - mid.0, t.1 - mid.1), (n.0 - t.0, n.1 - t.1));
                    let dot = d1.0 * d2.0 + d1.1 * d2.1;
                    let cross = d1.0 * d2.1 - d1.1 * d2.0;
                    dot < 0.0 && cross.abs() < 1e-6 * (1.0 + dot.abs())
                });
                p.extend_from_slice(&w[j + 1..]);
                let hits = p
                    .windows(2)
                    .filter(|v| seg_hits_box(v[0], v[1], b, node, other))
                    .count();
                let key = (hits, usize::from(fold), length(&p));
                if best.as_ref().is_none_or(|(bk, _)| better(key, *bk)) {
                    best = Some((key, p));
                }
            }
        }
    }
    best.map(|(_, p)| {
        let mut out = simplify_collinear(p);
        if !head {
            out.reverse();
        }
        out
    })
}

/// A fresh route from `ps` to `pt` round both end nodes, ignoring the waypoints, which were placed
/// for the unhinted route. `None` when no candidate clears both nodes.
fn rebuild(
    (ps, lo): (Port, usize),
    (pt, hi): (Port, usize),
    stub: f64,
    b: &Boxes,
) -> Option<Vec<(f64, f64)>> {
    let c = stub.max(1.0);
    let (ra, rb) = (
        Ring::new(lo, ps.side, ps.anchor, c, b),
        Ring::new(hi, pt.side, pt.anchor, c, b),
    );
    let cuts = |p: (f64, f64), q: (f64, f64)| {
        [lo, hi]
            .iter()
            .any(|&n| seg_hits_one_box(p, q, b, n, OWN_CLEAR))
    };
    let (wa, wb) = (ra.ways(), rb.ways());
    let mut best: Best = None;
    for ga in &wa {
        for gb in &wb {
            let (pa, pb) = (ga[ga.len() - 1], gb[gb.len() - 1]);
            let (mx, my) = ((pa.0 + pb.0) / 2.0, (pa.1 + pb.1) / 2.0);
            for conn in [
                vec![(pb.0, pa.1)],
                vec![(pa.0, pb.1)],
                vec![(mx, pa.1), (mx, pb.1)],
                vec![(pa.0, my), (pb.0, my)],
            ] {
                let mut p = ga.clone();
                p.extend(conn);
                p.extend(gb.iter().rev());
                let m = p.len();
                if p[1..m - 1].windows(2).any(|v| cuts(v[0], v[1])) {
                    continue;
                }
                let hits = p
                    .windows(2)
                    .filter(|v| seg_hits_box(v[0], v[1], b, lo, hi))
                    .count();
                let key = (hits, 0, length(&p));
                if best.as_ref().is_none_or(|(bk, _)| better(key, *bk)) {
                    best = Some((key, p));
                }
            }
        }
    }
    best.map(|(_, p)| simplify_collinear(p))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::graph::PortHint;
    use crate::options::Direction;

    fn boxes() -> Boxes {
        Boxes {
            cx: vec![0.0, 300.0],
            cy: vec![0.0, 0.0],
            layer_w: vec![100.0, 100.0],
            order_w: vec![40.0, 40.0],
            ellipse: vec![false, false],
            n_real: 2,
        }
    }

    /// A source on its backward border whose first drawing runs straight back through it.
    #[test]
    fn a_backward_source_goes_round_its_node() {
        let b = boxes();
        let route = vec![(-50.0, 0.0), (-65.0, 0.0), (250.0, 0.0)];
        assert!(cuts_own_box(&route, true, 0, Side::Left, &b));
        let h = EndHint {
            node: 0,
            side: Side::Left,
            offset: None,
        };
        let out = escape(&route, true, h, 1, 15.0, &b).unwrap();
        assert_eq!(out[0], (-50.0, 0.0), "the port stays: {out:?}");
        assert_eq!(*out.last().unwrap(), (250.0, 0.0));
        assert!(!cuts_own_box(&out, true, 0, Side::Left, &b), "{out:?}");
        for v in out.windows(2) {
            assert!((v[0].0 - v[1].0).abs() < 1e-9 || (v[0].1 - v[1].1).abs() < 1e-9);
        }
    }

    #[test]
    fn a_target_entered_from_the_far_side_goes_round_its_node() {
        let b = boxes();
        let route = vec![(50.0, 0.0), (365.0, 0.0), (350.0, 0.0)];
        assert!(cuts_own_box(&route, false, 1, Side::Right, &b));
        let h = EndHint {
            node: 1,
            side: Side::Right,
            offset: None,
        };
        let out = escape(&route, false, h, 0, 15.0, &b).unwrap();
        assert_eq!(*out.last().unwrap(), (350.0, 0.0));
        assert!(!cuts_own_box(&out, false, 1, Side::Right, &b), "{out:?}");
    }

    /// Edge `0 -> 1` with a `Top` source hint and a `Bottom` offset target hint, optionally
    /// turned by the ranking request and the cycle breaker.
    fn state(backwards: bool, reversed: bool, direction: Direction) -> LayeredState {
        use crate::geometry::shape::NodeShape;
        use crate::layered::prepare::{LayeredInput, LayeredInputNode};
        let node = LayeredInputNode {
            width: 100.0,
            height: 40.0,
            shape: NodeShape::Box,
            constraint: None,
            category: None,
            clear_after: 0.0,
            clear_before: 0.0,
            port_sides: None,
            lane_key: None,
            along_before: 0.0,
            along_after: 0.0,
        };
        // `ingest` turns the arc round when ranked backwards.
        let (from, to) = if backwards { (1, 0) } else { (0, 1) };
        let input = LayeredInput {
            nodes: vec![node.clone(), node],
            edges: vec![(from, to)],
            weights: vec![],
            thickness: vec![],
            edge_label_sizes: vec![],
            seed: vec![],
            pinned: vec![],
            self_loops: vec![],
            ranked_backwards: if backwards { vec![true] } else { vec![] },
            options: crate::options::LayoutOptions {
                direction,
                ..crate::options::LayoutOptions::layered_default()
            },
        };
        let mut st = LayeredState::new(&input);
        st.port_hints = vec![[
            Some(PortHint::Side(Side::Top)),
            Some(PortHint::Offset {
                side: Side::Bottom,
                t: 0.5,
            }),
        ]];
        if reversed {
            let a = &mut st.arcs[0];
            std::mem::swap(&mut a.from, &mut a.to);
            a.reversed = true;
        }
        let (lo, hi) = (st.arcs[0].from, st.arcs[0].to);
        st.nodes[hi].rank = 1;
        st.nodes[hi].along = 300.0;
        st.chains.insert(0, vec![lo, hi]);
        st
    }

    #[test]
    fn hints_follow_the_callers_nodes_through_every_reversal() {
        for backwards in [false, true] {
            for reversed in [false, true] {
                let st = state(backwards, reversed, Direction::LeftRight);
                let h = Hints::of(&st, &st.boxes());
                let low = st.chains[&0][0];
                let (at_source, at_target) = if low == 0 {
                    (h.get(0, true).unwrap(), h.get(0, false).unwrap())
                } else {
                    (h.get(0, false).unwrap(), h.get(0, true).unwrap())
                };
                let case = format!("backwards {backwards}, reversed {reversed}");
                assert_eq!(at_source.node, 0, "{case}");
                assert_eq!(at_source.side, Side::Top, "{case}");
                assert_eq!(at_source.offset, None, "{case}");
                assert_eq!(at_target.node, 1, "{case}");
                assert_eq!(at_target.side, Side::Bottom, "{case}");
                // Half of the 50px half-width of the bottom border.
                assert_eq!(at_target.offset, Some(25.0), "{case}");
                assert!(h.pinned(0, low != 0) && !h.pinned(0, low == 0), "{case}");
                assert_eq!(low == 0, backwards == reversed, "{case}");
            }
        }
    }

    /// Top to bottom, output `Bottom` is flow `Right` and the offset runs along the 100px width.
    #[test]
    fn hints_turn_through_the_axis() {
        let st = state(false, false, Direction::TopBottom);
        let h = Hints::of(&st, &st.boxes());
        assert_eq!(h.get(0, true).unwrap().side, Side::Left);
        let t = h.get(0, false).unwrap();
        assert_eq!(t.side, Side::Right);
        assert_eq!(t.offset, Some(25.0));
    }

    #[test]
    fn a_route_clear_of_its_node_is_not_cut() {
        let b = boxes();
        let route = vec![(50.0, 0.0), (250.0, 0.0)];
        assert!(!cuts_own_box(&route, true, 0, Side::Right, &b));
        assert!(!cuts_own_box(&route, false, 1, Side::Left, &b));
    }
}

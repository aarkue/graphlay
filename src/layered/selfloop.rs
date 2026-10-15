//! Self-loops. [`super::prepare::ingest`] moves every `source == target` edge into a [`LoopSlot`].
//!
//! [`reserve_self_loops`] runs before coordinate assignment and writes the room each node's loops
//! need into its clearance, so placement respects it through [`LayeredState::sep`].
//! [`route_self_loops`] runs after routing and draws each loop as a bump clear of the other ports.
//! [`bump`] works in any 2-D space, so the isolated-node front end calls it in output space.

use std::collections::BTreeMap;

use crate::error::LayoutError;
use crate::geometry::shape::NodeShape;
use crate::geometry::EPS;
use crate::geometry::{Boxes, Side};
use crate::options::{LayoutOptions, LoopSide, SelfLoopOptions};

use super::ports::{project_port, PORT_SPAN};
use super::state::LayeredState;

/// One drawn loop.
pub struct DrawnLoop {
    /// The bump, in the space [`bump`] was handed.
    pub points: Vec<(f64, f64)>,
    /// Centre of the label box reserved outside the bump.
    pub label_anchor: Option<(f64, f64)>,
    /// The caller's edge index.
    pub edge: usize,
}

/// Closest two nested loops' ports may sit, however crowded the border. Two parallel runs closer
/// than about half a pixel read as one.
const MIN_PORT_STEP: f64 = 3.0;

/// One self-loop edge. It carries its own thickness and label because a loop has no slot in the
/// per-arc tables.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoopEdge {
    /// The caller's edge index.
    pub edge: usize,
    /// Stroke width.
    pub thickness: f64,
    /// Label box `(width, height)` in output space, `(0, 0)` for no label.
    pub label: (f64, f64),
}

/// Every self-loop on one node, nested in edge order.
#[derive(Clone, Debug, PartialEq)]
pub struct LoopSlot {
    /// The node.
    pub node: usize,
    /// The loops, in caller edge order.
    pub edges: Vec<LoopEdge>,
    /// Flow-space border the loops bump out of, `None` until [`reserve_self_loops`] sets it.
    pub side: Option<Side>,
    /// Output-space border from port hints, set when every hinted loop agrees. Overrides all
    /// other preferences.
    pub hint: Option<Side>,
}

impl LoopSlot {
    /// An empty slot for `node`.
    pub fn new(node: usize) -> Self {
        LoopSlot {
            node,
            edges: Vec::new(),
            side: None,
            hint: None,
        }
    }

    /// Widest label extent along the loops' outward axis, plus the label gap.
    fn label_reach(&self, side: Side, axis: super::state::Axis, o: &SelfLoopOptions) -> f64 {
        let widest = self
            .edges
            .iter()
            .map(|e| {
                let (along, cross) = axis.extents(e.label.0, e.label.1);
                if side.horizontal() {
                    along
                } else {
                    cross
                }
            })
            .fold(0.0f64, f64::max);
        if widest > 0.0 {
            widest + o.label_gap
        } else {
            0.0
        }
    }

    /// How far the loops and labels reach beyond the border: `size + k * gap + widest label`.
    /// That is one gap more than the bumps use, so the outermost loop has space past it.
    pub fn reach(&self, side: Side, axis: super::state::Axis, o: &SelfLoopOptions) -> f64 {
        if self.edges.is_empty() {
            return 0.0;
        }
        o.size.max(0.0) + self.edges.len() as f64 * o.gap.max(0.0) + self.label_reach(side, axis, o)
    }
}

/// The flow-space border a [`LoopSide`] names, `None` for `Auto`.
fn fixed_side(side: LoopSide) -> Option<Side> {
    match side {
        // Flow space: `Bottom` is +cross, `Top` is -cross, `Right` is +along.
        LoopSide::CrossPositive => Some(Side::Bottom),
        LoopSide::CrossNegative => Some(Side::Top),
        LoopSide::AlongPositive => Some(Side::Right),
        LoopSide::Auto => None,
    }
}

/// Resolves each loop slot's side and reserves the room its loops need, before coordinate
/// assignment. `Auto` resolves here because it needs the layer order (see `auto_side`).
pub fn reserve_self_loops(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    if st.self_loops.is_empty() {
        return Ok(());
    }
    let axis = st.axis;
    let fixed = fixed_side(o.self_loops.side);
    let mut slots = std::mem::take(&mut st.self_loops);
    for slot in &mut slots {
        if slot.edges.is_empty() || slot.node >= st.n_real {
            continue;
        }
        // A port hint wins over the drawing-wide loop side, which wins over the free one.
        let side = slot
            .hint
            .map(|h| axis.to_flow_side(h))
            .or(fixed)
            .unwrap_or_else(|| auto_side(st, slot.node));
        slot.side = Some(side);
        let reach = slot.reach(side, axis, &o.self_loops);
        if reach <= 0.0 {
            continue;
        }
        match side {
            Side::Bottom => {
                let node = &mut st.nodes[slot.node];
                node.clear_after = node.clear_after.max(reach);
            }
            Side::Top => {
                let node = &mut st.nodes[slot.node];
                node.clear_before = node.clear_before.max(reach);
            }
            // A node narrower than its layer already has some room inside the column, so only
            // the excess is reserved. A backward loop in the first layer needs none.
            Side::Right | Side::Left => {
                let rank = st.nodes[slot.node].layer_clamped();
                let gap_at = if side == Side::Right {
                    Some(rank)
                } else {
                    rank.checked_sub(1)
                };
                let slack = st
                    .layers
                    .get(rank)
                    .map(|l| {
                        l.iter()
                            .map(|&n| st.nodes[n].w_along)
                            .fold(0.0f64, f64::max)
                    })
                    .unwrap_or(0.0)
                    - st.nodes[slot.node].w_along;
                if let Some(gap) = gap_at.and_then(|g| st.gap_after.get_mut(g)) {
                    *gap = gap.max(reach - slack.max(0.0) / 2.0);
                }
            }
        }
    }
    st.self_loops = slots;
    Ok(())
}

/// The order-axis border of `node` with no layer neighbour. With neighbours on both sides or
/// none, the positive side unless [`crate::Margins`] already claims more room there.
fn auto_side(st: &LayeredState, node: usize) -> Side {
    let rank = st.nodes[node].layer_clamped();
    let (mut has_neg, mut has_pos) = (false, false);
    if let Some(layer) = st.layers.get(rank) {
        if let Some(pos) = layer.iter().position(|&v| v == node) {
            has_neg = pos > 0;
            has_pos = pos + 1 < layer.len();
        }
    }
    match (has_neg, has_pos) {
        (true, false) => Side::Bottom,
        (false, true) => Side::Top,
        _ => {
            if st.nodes[node].clear_after > st.nodes[node].clear_before {
                Side::Top
            } else {
                Side::Bottom
            }
        }
    }
}

/// Draws every self-loop after routing, clear of the ordinary ports on its border.
pub fn route_self_loops(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    if st.self_loops.is_empty() {
        return Ok(());
    }
    let boxes = st.boxes();
    let slots = std::mem::take(&mut st.self_loops);
    for slot in &slots {
        let Some(side) = slot.side else { continue };
        if slot.edges.is_empty() || slot.node >= st.n_real {
            continue;
        }
        let taken = occupied_span(st, &boxes, slot.node, side);
        for drawn in bump(&boxes, slot, side, taken, o) {
            st.loop_routes.insert(drawn.edge, drawn.points);
            if let Some(a) = drawn.label_anchor {
                st.loop_label_anchors.insert(drawn.edge, a);
            }
        }
    }
    st.self_loops = slots;
    Ok(())
}

/// Offsets from the border's middle spanned by ordinary ports on `side`, or `None`. Read from the
/// settled routes, since the repair passes may have moved endpoints after port assignment.
fn occupied_span(st: &LayeredState, b: &Boxes, node: usize, side: Side) -> Option<(f64, f64)> {
    let (cx, cy) = (b.cx[node], b.cy[node]);
    let (hw, hh) = (b.layer_w[node] / 2.0, b.order_w[node] / 2.0);
    let mut span: Option<(f64, f64)> = None;
    for (&arc, chain) in &st.chains {
        if chain.len() < 2 {
            continue;
        }
        let Some(route) = st.routes.get(arc) else {
            continue;
        };
        if route.len() < 2 {
            continue;
        }
        for (end, at) in [
            (route[0], chain[0]),
            (route[route.len() - 1], chain[chain.len() - 1]),
        ] {
            if at != node {
                continue;
            }
            // Same rule as `layout::side_at`.
            let (dx, dy) = (end.0 - cx, end.1 - cy);
            let (nx, ny) = (
                if hw > 0.0 { (dx / hw).abs() } else { 0.0 },
                if hh > 0.0 { (dy / hh).abs() } else { 0.0 },
            );
            let on = if nx >= ny {
                if dx >= 0.0 {
                    Side::Right
                } else {
                    Side::Left
                }
            } else if dy >= 0.0 {
                Side::Bottom
            } else {
                Side::Top
            };
            if on != side {
                continue;
            }
            let off = if side.horizontal() { dy } else { dx };
            span = Some(match span {
                Some((lo, hi)) => (lo.min(off), hi.max(off)),
                None => (off, off),
            });
        }
    }
    span
}

/// Draw every loop of `slot` as a bump out of its exit port and back into its entry port. Each
/// loop sits one `gap` deeper with ports one step further apart, so nested loops never touch.
fn bump(
    b: &Boxes,
    slot: &LoopSlot,
    side: Side,
    taken: Option<(f64, f64)>,
    o: &LayoutOptions,
) -> Vec<DrawnLoop> {
    let node = slot.node;
    let k = slot.edges.len();
    let (cx, cy) = (b.cx[node], b.cy[node]);
    let (hw, hh) = (b.layer_w[node] / 2.0, b.order_w[node] / 2.0);
    let half = if side.horizontal() { hh } else { hw };
    let limit = half * PORT_SPAN;
    let want = slot
        .edges
        .iter()
        .fold(o.spacing.port_gap, |g, e| {
            g.max(e.thickness + o.spacing.port_stroke_clear)
        })
        .max(1.0);
    // Nest in the wider free run beside the ordinary ports, since straddling them would cross
    // every arc on the border. A run too narrow for the nest widens inwards only as far as needed.
    let full = (-limit, limit);
    let room_of = |iv: (f64, f64)| ((iv.1 - iv.0) / 2.0).max(0.0);
    let picked = match taken {
        None => full,
        Some((lo, hi)) => {
            let neg = (-limit, (lo - want).clamp(-limit, limit));
            let pos = ((hi + want).clamp(-limit, limit), limit);
            if room_of(neg) >= room_of(pos) {
                neg
            } else {
                pos
            }
        }
    };
    let need = MIN_PORT_STEP * k as f64;
    let (free_lo, free_hi) = if room_of(picked) >= need {
        picked
    } else if picked.0 <= -limit + EPS {
        (-limit, (-limit + 2.0 * need).min(limit))
    } else {
        ((limit - 2.0 * need).max(-limit), limit)
    };
    let centre_off = ((free_lo + free_hi) / 2.0).clamp(-limit, limit);
    let room = room_of((free_lo, free_hi));
    let step = (room / k as f64)
        .max(MIN_PORT_STEP)
        .min(want.max(MIN_PORT_STEP));
    let outward = match side {
        Side::Right => (1.0, 0.0),
        Side::Left => (-1.0, 0.0),
        Side::Bottom => (0.0, 1.0),
        Side::Top => (0.0, -1.0),
    };
    let border = if side.horizontal() { hw } else { hh };
    let stub = o.spacing.stub.max(0.0);
    let mut out = Vec::with_capacity(k);
    for (i, e) in slot.edges.iter().enumerate() {
        let depth = o.self_loops.size.max(0.0) + i as f64 * o.self_loops.gap.max(0.0);
        let off_out = (centre_off - (i + 1) as f64 * step).clamp(-limit, limit);
        let off_in = (centre_off + (i + 1) as f64 * step).clamp(-limit, limit);
        let exit = project_port(node, side, off_out, b, stub).anchor;
        let entry = project_port(node, side, off_in, b, stub).anchor;
        let outer = if side.horizontal() {
            cx + outward.0 * (border + depth)
        } else {
            cy + outward.1 * (border + depth)
        };
        let points = if side.horizontal() {
            vec![exit, (outer, exit.1), (outer, entry.1), entry]
        } else {
            vec![exit, (exit.0, outer), (entry.0, outer), entry]
        };
        // Push the label off the outer run by its own half-extent.
        let (lw, lh) = e.label;
        let push = if lw > 0.0 && lh > 0.0 {
            let extent = if side.horizontal() { lw } else { lh };
            o.self_loops.label_gap.max(0.0) + extent / 2.0
        } else {
            0.0
        };
        let anchor = if side.horizontal() {
            (outer + outward.0 * push, (exit.1 + entry.1) / 2.0)
        } else {
            ((exit.0 + entry.0) / 2.0, outer + outward.1 * push)
        };
        out.push(DrawnLoop {
            points,
            label_anchor: Some(anchor),
            edge: e.edge,
        });
    }
    out
}

/// Draw the loops of an isolated node, which is placed outside any [`LayeredState`], in output
/// space. It has no other ports to keep clear of.
pub(crate) fn isolated_loops(
    center: (f64, f64),
    size: (f64, f64),
    shape: NodeShape,
    slot: &LoopSlot,
    side: Side,
    o: &LayoutOptions,
) -> Vec<DrawnLoop> {
    let boxes = Boxes {
        cx: vec![center.0],
        cy: vec![center.1],
        layer_w: vec![size.0],
        order_w: vec![size.1],
        ellipse: vec![shape == NodeShape::Ellipse],
        n_real: 1,
    };
    let local = LoopSlot {
        node: 0,
        edges: slot.edges.clone(),
        side: Some(side),
        hint: None,
    };
    bump(&boxes, &local, side, None, o)
}

/// Every loop route of one run, keyed by caller edge index.
pub type LoopRoutes = BTreeMap<usize, Vec<(f64, f64)>>;
/// Every loop label anchor of one run, keyed by caller edge index.
pub type LoopAnchors = BTreeMap<usize, (f64, f64)>;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::geometry::fp;
    use crate::options::Spacing;

    fn boxes(w: f64, h: f64, ellipse: bool) -> Boxes {
        Boxes {
            cx: vec![0.0],
            cy: vec![0.0],
            layer_w: vec![w],
            order_w: vec![h],
            ellipse: vec![ellipse],
            n_real: 1,
        }
    }

    fn slot(k: usize) -> LoopSlot {
        LoopSlot {
            node: 0,
            edges: (0..k)
                .map(|i| LoopEdge {
                    edge: i,
                    thickness: 2.0,
                    label: (0.0, 0.0),
                })
                .collect(),
            side: Some(Side::Bottom),
            hint: None,
        }
    }

    #[test]
    fn a_bump_leaves_and_re_enters_the_border() {
        let o = LayoutOptions::layered_default();
        let b = boxes(120.0, 52.0, false);
        let drawn = bump(&b, &slot(1), Side::Bottom, None, &o);
        assert_eq!(drawn.len(), 1);
        let (pts, anchor, edge) = (&drawn[0].points, drawn[0].label_anchor, drawn[0].edge);
        assert_eq!(edge, 0);
        assert_eq!(pts.len(), 4);
        assert_eq!(pts[0].1, 26.0);
        assert_eq!(pts[3].1, 26.0);
        assert!(pts[0].0 < 0.0 && pts[3].0 > 0.0, "{pts:?}");
        assert_eq!(pts[1].1, 26.0 + o.self_loops.size);
        assert_eq!(pts[2].1, pts[1].1);
        assert!(anchor.unwrap().1 > 26.0);
    }

    #[test]
    fn stacked_loops_nest_without_touching() {
        let o = LayoutOptions::layered_default();
        let b = boxes(200.0, 52.0, false);
        let drawn = bump(&b, &slot(3), Side::Bottom, None, &o);
        assert_eq!(drawn.len(), 3);
        for w in drawn.windows(2) {
            let (inner, outer) = (&w[0].points, &w[1].points);
            assert!(outer[1].1 > inner[1].1, "outer bump must be deeper");
            assert!(
                outer[0].0 < inner[0].0,
                "outer exit port must be further out"
            );
            assert!(
                outer[3].0 > inner[3].0,
                "outer entry port must be further out"
            );
        }
    }

    #[test]
    fn a_loop_on_an_ellipse_starts_on_the_outline() {
        let o = LayoutOptions::layered_default();
        let b = boxes(60.0, 60.0, true);
        let drawn = bump(&b, &slot(1), Side::Bottom, None, &o);
        let pts = &drawn[0].points;
        for p in [pts[0], pts[3]] {
            let r = fp::hypot(p.0 / 30.0, p.1 / 30.0);
            assert!(
                (r - 1.0).abs() < 1e-9,
                "{p:?} is not on the circle (r = {r})"
            );
        }
    }

    #[test]
    fn loop_ports_clear_the_ports_already_on_the_border() {
        let o = LayoutOptions::layered_default();
        let b = boxes(300.0, 52.0, false);
        for (taken, expect_negative) in [((-20.0, 30.0), true), ((-120.0, -60.0), false)] {
            let drawn = bump(&b, &slot(2), Side::Bottom, Some(taken), &o);
            for one in &drawn {
                let pts = &one.points;
                for p in [pts[0].0, pts[3].0] {
                    let clear = if expect_negative {
                        taken.0 - p
                    } else {
                        p - taken.1
                    };
                    assert!(
                        clear >= o.spacing.port_gap - 1e-9,
                        "port {p} is only {clear} clear of {taken:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn reach_covers_every_bump_and_its_label() {
        let o = SelfLoopOptions::default();
        let axis = super::super::state::Axis::AlongX;
        let mut s = slot(3);
        assert_eq!(s.reach(Side::Bottom, axis, &o), o.size + 3.0 * o.gap);
        s.edges[1].label = (40.0, 14.0);
        assert_eq!(
            s.reach(Side::Bottom, axis, &o),
            o.size + 3.0 * o.gap + 14.0 + o.label_gap
        );
        assert_eq!(LoopSlot::new(0).reach(Side::Bottom, axis, &o), 0.0);
        assert_eq!(Spacing::layered().self_loop_gap, o.gap);
    }
}

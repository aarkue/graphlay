//! Endpoint side selection: which border of its node each arc end attaches to.
//!
//! An end faces where its edge heads, or turns to top or bottom when the edge leaves steeply. The
//! choice is priced against gutter traffic ([`Pricer`]) and the node's port-side preference. A
//! port hint overrides both.

use std::collections::BTreeMap;

use super::gutter::{lanes_cross, GutterSeg};
use super::{diagonal, ArcEnds, RouteCtx, PORT_SPAN};
use crate::geometry::Side;
use crate::graph::PortSides;

/// Slope (cross offset over layer offset) above which a multi-hop source end leaves through its
/// top or bottom border.
const SOURCE_TURN_SLOPE: f64 = 0.7;
/// The same for a target end. It is higher, as in ELK, so a target turns only on a steep approach.
const TARGET_TURN_SLOPE: f64 = 2.0;

/// One arc end. `aim_*` is the centre of the adjacent chain node.
pub(super) struct EdgeEnd {
    pub(super) node: usize,
    pub(super) side: Side,
    pub(super) aim_order: f64,
    pub(super) aim_layer: f64,
}

pub(super) fn assign_sides(ctx: &RouteCtx, side_port_used: &mut [bool]) -> ArcEnds {
    let pricer = Pricer::new(ctx);
    let mut arc_ends = choose_sides(&pricer, side_port_used);
    relieve_perpendicular_targets(&pricer, &mut arc_ends);
    arc_ends
}

/// Side-selection cost model over every gutter's passages. Ends sit at node centres because ports
/// are not spread yet. [`Pricer::end_costs`] allows for the spread where it matters.
struct Pricer<'c, 'a> {
    ctx: &'c RouteCtx<'a>,
    gutter_spans: BTreeMap<usize, Vec<GutterSeg>>,
    /// Upper bound on the ends each facing border carries, out and in.
    out_ends: Vec<usize>,
    in_ends: Vec<usize>,
}

impl<'c, 'a> Pricer<'c, 'a> {
    fn new(ctx: &'c RouteCtx<'a>) -> Self {
        let (cy, n_in, layer_of) = (&ctx.b.cy, ctx.n_in, &ctx.layer_of);
        let mut gutter_spans: BTreeMap<usize, Vec<GutterSeg>> = BTreeMap::new();
        for (arc, chain) in ctx.ordered() {
            for w in chain.windows(2) {
                let l = layer_of[w[0]] as usize;
                gutter_spans.entry(l).or_default().push(GutterSeg {
                    arc,
                    from: cy[w[0]],
                    to: cy[w[1]],
                });
            }
        }
        let (mut out_ends, mut in_ends) = (vec![0usize; n_in], vec![0usize; n_in]);
        for (arc, chain) in ctx.ordered() {
            if let (Some(&first), Some(&last)) = (chain.first(), chain.last()) {
                if chain.len() >= 2 && first < n_in && last < n_in {
                    out_ends[first] += ctx.lane_count(arc);
                    in_ends[last] += ctx.lane_count(arc);
                }
            }
        }
        Pricer {
            ctx,
            gutter_spans,
            out_ends,
            in_ends,
        }
    }

    /// Passages a riser in gutter `g` crosses in both lane orders. A pair that crosses in only one
    /// order becomes a lane constraint in `order_gutter_lanes` and costs nothing.
    fn riser_cost(&self, g: usize, ours: &GutterSeg) -> usize {
        self.gutter_spans.get(&g).map_or(0, |spans| {
            spans
                .iter()
                .filter(|s| s.arc != ours.arc && lanes_cross(ours, s) && lanes_cross(s, ours))
                .count()
        })
    }

    /// Risers in gutter `g` that a straight run at height `h` cuts. One that merely ends at `h`
    /// counts only when `aim` is a real node, whose port spreading will turn the tangency into a
    /// crossing. Towards a dummy the tangency is noise.
    fn run_cost(&self, g: usize, arc: usize, h: f64, aim: usize) -> usize {
        let slack = if aim < self.ctx.n_in { -0.5 } else { 0.5 };
        self.gutter_spans.get(&g).map_or(0, |spans| {
            spans
                .iter()
                .filter(|s| {
                    s.arc != arc && s.from.min(s.to) < h - slack && h + slack < s.from.max(s.to)
                })
                .count()
        })
    }

    /// Dummies of `node`'s layer strictly between `h` and its border, each cut by a riser there.
    fn column_cost(&self, node: usize, h: f64) -> usize {
        let (cx, cy, ow) = (&self.ctx.b.cx, &self.ctx.b.cy, &self.ctx.b.order_w);
        let layer_of = &self.ctx.layer_of;
        let border = cy[node] + (ow[node] / 2.0) * (h - cy[node]).signum();
        let (lo, hi) = (h.min(border), h.max(border));
        (self.ctx.n_in..cx.len())
            .filter(|&d| layer_of[d] == layer_of[node] && cy[d] > lo + 0.5 && cy[d] < hi - 0.5)
            .count()
    }

    /// Whether a facing border would hold more ends than fit at the port gap. Spreading would
    /// squeeze them until they read as one bundle.
    fn crowded(&self, node: usize, is_source: bool) -> bool {
        let load = if is_source {
            self.out_ends[node]
        } else {
            self.in_ends[node]
        };
        load >= 2
            && (load as f64 - 1.0) * self.ctx.min_port_gap
                > 2.0 * self.ctx.b.order_w[node] * PORT_SPAN
    }

    /// `(perpendicular, facing)` cost in crossings and overlaps for the end at `node` to reach
    /// `aim`. Perpendicular pays a gutter run plus a column riser. Facing pays a gutter riser plus
    /// one if the border is crowded. On crossings alone facing is never worse.
    fn end_costs(&self, arc: usize, node: usize, aim: usize, is_source: bool) -> (usize, usize) {
        let (cy, ow, layer_of) = (&self.ctx.b.cy, &self.ctx.b.order_w, &self.ctx.layer_of);
        let h = cy[aim];
        let g = if is_source {
            layer_of[node]
        } else {
            layer_of[aim]
        } as usize;
        let span = ow[node] * PORT_SPAN;
        let port = cy[node] + (h - cy[node]).clamp(-span, span);
        let riser = if is_source {
            GutterSeg {
                arc,
                from: port,
                to: h,
            }
        } else {
            GutterSeg {
                arc,
                from: h,
                to: port,
            }
        };
        (
            self.run_cost(g, arc, h, aim) + self.column_cost(node, h),
            self.riser_cost(g, &riser) + usize::from(self.crowded(node, is_source)),
        )
    }

    /// Whether the node's preference allows the perpendicular border. `FacingPreferred` needs it
    /// free while the facing border costs something.
    fn perp_earned(&self, arc: usize, node: usize, aim: usize, is_source: bool) -> bool {
        let is_long = |arc: usize| self.ctx.chains.get(&arc).is_some_and(|c| c.len() > 2);
        match self.ctx.sides_of(node) {
            PortSides::Any => true,
            PortSides::LongEdgesAny if is_long(arc) => true,
            PortSides::FacingOnly => false,
            PortSides::FacingPreferred | PortSides::LongEdgesAny => {
                let (perp, facing) = self.end_costs(arc, node, aim, is_source);
                perp == 0 && facing > 0
            }
        }
    }

    /// Borders of a direct edge, chosen jointly. A single-bend L needs exactly one perpendicular
    /// end, otherwise both stay facing and the edge takes the gutter Z.
    fn direct_sides(&self, arc: usize, lo: usize, hi: usize) -> (Side, Side) {
        let ctx = self.ctx;
        let (cx, cy, ow) = (&ctx.b.cx, &ctx.b.cy, &ctx.b.order_w);
        let dy = cy[hi] - cy[lo];
        if dy.abs() < 1.0 {
            return (Side::Right, Side::Left);
        }
        // Ends whose order-axis extents overlap have no room for an L.
        let overlap = (cy[lo] - ow[lo] / 2.0).max(cy[hi] - ow[hi] / 2.0)
            < (cy[lo] + ow[lo] / 2.0).min(cy[hi] + ow[hi] / 2.0);
        if overlap {
            return (Side::Right, Side::Left);
        }
        let (s_perp, t_perp) = if dy > 0.0 {
            (Side::Bottom, Side::Top)
        } else {
            (Side::Top, Side::Bottom)
        };
        // B: vertical out of the source, then across into the target's facing border.
        let b_ok = self.perp_earned(arc, lo, hi, true)
            && !ctx.seg_hits((cx[lo], cy[lo]), (cx[lo], cy[hi]), lo, hi)
            && !ctx.seg_hits((cx[lo], cy[hi]), (cx[hi], cy[hi]), lo, hi);
        // A: across at the source's height, then vertical into the target.
        let a_ok = self.perp_earned(arc, hi, lo, false)
            && !ctx.seg_hits((cx[lo], cy[lo]), (cx[hi], cy[lo]), lo, hi)
            && !ctx.seg_hits((cx[hi], cy[lo]), (cx[hi], cy[hi]), lo, hi);
        // Each riser sweeps a layer column and crosses its dummy channels. B wins ties.
        let dummy_cross = |layer_node: usize, y0: f64, y1: f64| -> usize {
            let l = ctx.layer_of[layer_node];
            let (lo_y, hi_y) = (y0.min(y1) + 1.0, y0.max(y1) - 1.0);
            (ctx.n_in..cx.len())
                .filter(|&d| ctx.layer_of[d] == l && cy[d] > lo_y && cy[d] < hi_y)
                .count()
        };
        if b_ok && a_ok && dummy_cross(hi, cy[lo], cy[hi]) < dummy_cross(lo, cy[lo], cy[hi]) {
            return (Side::Right, t_perp);
        }
        if b_ok {
            return (s_perp, Side::Left);
        }
        if a_ok {
            return (Side::Right, t_perp);
        }
        (Side::Right, Side::Left)
    }

    /// [`perp_side`], but a perpendicular border only when [`Pricer::perp_earned`] allows it and
    /// the run along the layer's centre line to the channel is clear. Else the facing border.
    fn guarded_perp(&self, arc: usize, node: usize, aim: usize, is_source: bool) -> Side {
        let (cx, cy) = (&self.ctx.b.cx, &self.ctx.b.cy);
        let side = perp_side(self.ctx, node, aim, is_source);
        let perp_run_clear = || {
            !self
                .ctx
                .seg_hits((cx[node], cy[node]), (cx[node], cy[aim]), node, aim)
        };
        if side.horizontal() || (self.perp_earned(arc, node, aim, is_source) && perp_run_clear()) {
            return side;
        }
        if is_source {
            Side::Right
        } else {
            Side::Left
        }
    }
}

/// Border of a multi-hop end facing the adjacent waypoint. The source turns to top or bottom above
/// [`SOURCE_TURN_SLOPE`], the target above [`TARGET_TURN_SLOPE`].
fn perp_side(ctx: &RouteCtx, node: usize, aim: usize, is_source: bool) -> Side {
    let (cx, cy) = (&ctx.b.cx, &ctx.b.cy);
    let (dx, dy) = (cx[aim] - cx[node], cy[aim] - cy[node]);
    let thresh = if is_source {
        SOURCE_TURN_SLOPE
    } else {
        TARGET_TURN_SLOPE
    };
    if dy.abs() > dx.abs() * thresh {
        if dy > 0.0 {
            Side::Bottom
        } else {
            Side::Top
        }
    } else if is_source {
        Side::Right
    } else {
        Side::Left
    }
}

fn choose_sides(pricer: &Pricer, side_port_used: &mut [bool]) -> ArcEnds {
    let ctx = pricer.ctx;
    let (cx, cy, lw, ow) = (&ctx.b.cx, &ctx.b.cy, &ctx.b.layer_w, &ctx.b.order_w);
    let flow_edges = ctx.flow_edges;
    let mut arc_ends: ArcEnds = BTreeMap::new();
    for (arc, chain) in ctx.ordered() {
        if chain.len() < 2 {
            continue;
        }
        let lo = chain[0];
        let hi = chain[chain.len() - 1];
        let direct = chain.len() == 2;
        let s_aim = chain[1];
        let t_aim = chain[chain.len() - 2];
        let (s_side, t_side) = if flow_edges
            && direct
            && (cx[hi] - cx[lo]).abs() > lw[lo]
            && (cy[hi] - cy[lo]).abs() > ow[lo]
        {
            // `ray_side` would trap a far-apart direct edge in the node band as a staircase.
            (Side::Right, Side::Left)
        } else if flow_edges {
            // A long edge to a far-offset column reads better leaving through the face towards it.
            let plain_side = diagonal::ray_side(ctx, lo, cx[s_aim], cy[s_aim]);
            let s = if ctx.side_port && chain.len() >= 5 && (cy[hi] - cy[lo]).abs() > ow[lo] * 0.9 {
                let side = diagonal::ray_side(ctx, lo, cx[lo], cy[hi]);
                if side != plain_side {
                    // The port table is shared, so this can change any arc's route.
                    side_port_used.fill(true);
                }
                side
            } else {
                plain_side
            };
            let t = diagonal::ray_side(ctx, hi, cx[t_aim], cy[t_aim]);
            // Diagonal style has no lane model to price a border with, so `FacingPreferred`
            // reads as `Any` here.
            let facing_only = |node: usize| ctx.sides_of(node) == PortSides::FacingOnly;
            (
                if facing_only(lo) { Side::Right } else { s },
                if facing_only(hi) { Side::Left } else { t },
            )
        } else if direct {
            pricer.direct_sides(arc, lo, hi)
        } else {
            (
                pricer.guarded_perp(arc, lo, s_aim, true),
                pricer.guarded_perp(arc, hi, t_aim, false),
            )
        };
        let (s_side, t_side) = hinted_sides(ctx, arc, direct, (s_side, t_side));
        arc_ends.insert(
            arc,
            (
                EdgeEnd {
                    node: lo,
                    side: s_side,
                    aim_order: cy[s_aim],
                    aim_layer: cx[s_aim],
                },
                EdgeEnd {
                    node: hi,
                    side: t_side,
                    aim_order: cy[t_aim],
                    aim_layer: cx[t_aim],
                },
            ),
        );
    }
    arc_ends
}

/// Lay `arc`'s port hints over the chosen borders. When a hint makes one end of a direct edge
/// perpendicular, the free end goes back to facing so the L keeps one perpendicular end.
fn hinted_sides(ctx: &RouteCtx, arc: usize, direct: bool, (s, t): (Side, Side)) -> (Side, Side) {
    let (hs, ht) = (ctx.hints.get(arc, true), ctx.hints.get(arc, false));
    match (hs, ht) {
        (None, None) => (s, t),
        (Some(hs), Some(ht)) => (hs.side, ht.side),
        (Some(hs), None) => {
            let t = if direct && !hs.side.horizontal() && !t.horizontal() {
                Side::Left
            } else {
                t
            };
            (hs.side, t)
        }
        (None, Some(ht)) => {
            let s = if direct && !ht.side.horizontal() && !s.horizontal() {
                Side::Right
            } else {
                s
            };
            (s, ht.side)
        }
    }
}

/// Move a long edge's target end to a free perpendicular border when it approaches from well
/// above or below, saving a bend, if the node allows it and the move is free.
fn relieve_perpendicular_targets(pricer: &Pricer, arc_ends: &mut ArcEnds) {
    let ctx = pricer.ctx;
    let (cx, cy, ow, n_in) = (&ctx.b.cx, &ctx.b.cy, &ctx.b.order_w, ctx.n_in);
    let mut deg: Vec<usize> = vec![0; n_in];
    for (_, chain) in ctx.ordered() {
        if chain.len() >= 2 {
            let (a, b) = (chain[0], chain[chain.len() - 1]);
            if a < n_in {
                deg[a] += 1;
            }
            if b < n_in {
                deg[b] += 1;
            }
        }
    }
    let mut occ: BTreeMap<(usize, u8), usize> = BTreeMap::new();
    for (s, t) in arc_ends.values() {
        *occ.entry((s.node, s.side.index())).or_default() += 1;
        *occ.entry((t.node, t.side.index())).or_default() += 1;
    }
    let mut flips: Vec<(usize, Side)> = vec![];
    for (arc, chain) in ctx.ordered() {
        if chain.len() <= 2 {
            continue; // `direct_sides` already chose these
        }
        let t = &arc_ends[&arc].1;
        if t.side != Side::Left || ctx.hints.get(arc, false).is_some() {
            continue;
        }
        let hi = chain[chain.len() - 1];
        let sides = ctx.sides_of(hi);
        if sides == PortSides::FacingOnly {
            continue;
        }
        let t_aim = chain[chain.len() - 2];
        let drop = cy[t_aim] - cy[hi];
        // Within the port span the channel enters straight. Busy nodes need a jog wider than a
        // port gap before the perpendicular border is worth it.
        let span = ow[hi] * PORT_SPAN;
        if drop.abs() <= span || (deg[hi] > 4 && drop.abs() <= span + ctx.min_port_gap) {
            continue;
        }
        // An approach at the border's own height would arrive tangentially.
        if drop.abs() < ow[hi] / 2.0 + ctx.min_port_gap {
            continue;
        }
        let perp = if drop > 0.0 { Side::Bottom } else { Side::Top };
        if *occ.get(&(hi, perp.index())).unwrap_or(&0) != 0 {
            continue;
        }
        // Keep the edge with others entering the facing border from the same direction.
        let clustered = arc_ends.iter().any(|(&other, (s2, t2))| {
            other != arc
                && ((s2.node == hi
                    && s2.side == Side::Left
                    && (s2.aim_order - cy[hi]) * drop > 0.0)
                    || (t2.node == hi
                        && t2.side == Side::Left
                        && (t2.aim_order - cy[hi]) * drop > 0.0))
        });
        if clustered {
            continue;
        }
        // Another node of the target's layer may stand on the approach line.
        let bx = cx[hi];
        if ctx.seg_hits((cx[t_aim], cy[t_aim]), (bx, cy[t_aim]), chain[0], hi)
            || ctx.seg_hits((bx, cy[t_aim]), (bx, cy[hi]), chain[0], hi)
        {
            continue;
        }
        // The perpendicular border must be free, because a saved bend is not worth a crossing.
        let (perp_cost, facing_cost) = pricer.end_costs(arc, hi, t_aim, false);
        // Only long edges get here, so `LongEdgesAny` reads as `Any`.
        if perp_cost > 0 || (sides == PortSides::FacingPreferred && facing_cost == 0) {
            continue;
        }
        flips.push((arc, perp));
    }
    for (arc, perp) in flips {
        let Some(ends) = arc_ends.get_mut(&arc) else {
            continue; // unreachable: `flips` only holds arcs from `arc_ends`
        };
        if let Some(c) = occ.get_mut(&(ends.1.node, ends.1.side.index())) {
            *c = c.saturating_sub(1);
        }
        *occ.entry((ends.1.node, perp.index())).or_default() += 1;
        ends.1.side = perp;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::super::route;
    use super::*;
    use crate::geometry::hit::seg_hits_box;
    use crate::layered::prepare::{LayeredInput, LayeredInputNode};
    use crate::layered::state::{LArc, LayeredState};
    use crate::options::{LayoutOptions, Routing};

    /// Source -> dummy -> target over ranks 0-2, plus a rank-0 blocker at `blocker_cross`.
    fn multi_hop_past_a_blocker(blocker_cross: f64) -> LayeredState {
        let node = || LayeredInputNode {
            width: 120.0,
            height: 52.0,
            shape: crate::geometry::shape::NodeShape::Box,
            constraint: None,
            category: None,
            clear_after: 0.0,
            clear_before: 0.0,
            port_sides: None,
            lane_key: None,
            along_before: 0.0,
            along_after: 0.0,
        };
        let input = LayeredInput {
            nodes: vec![node(), node(), node()],
            edges: vec![(0, 2)],
            weights: vec![],
            thickness: vec![],
            edge_label_sizes: vec![],
            seed: vec![],
            pinned: vec![],
            self_loops: vec![],
            ranked_backwards: Vec::new(),
            options: LayoutOptions {
                direction: crate::options::Direction::LeftRight,
                ..LayoutOptions::layered_default()
            },
        };
        let mut st = LayeredState::new(&input);
        let d = st.add_dummy(1);
        st.nodes[2].rank = 2;
        for (i, (along, cross)) in [(60.0, 0.0), (60.0, blocker_cross), (660.0, 400.0)]
            .into_iter()
            .enumerate()
        {
            st.nodes[i].along = along;
            st.nodes[i].cross = cross;
        }
        st.nodes[d].along = 360.0;
        st.nodes[d].cross = 400.0;
        st.arcs = vec![
            LArc {
                from: 0,
                to: d,
                edge: 0,
                reversed: false,
            },
            LArc {
                from: d,
                to: 2,
                edge: 0,
                reversed: false,
            },
        ];
        st.chains.insert(0, vec![0, d, 2]);
        st.layers = vec![vec![0, 1], vec![d], vec![2]];
        st.reindex();
        st
    }

    #[test]
    fn a_blocked_in_rank_run_takes_the_layer_forward_face() {
        // Use plain routing defaults so the preset's port-side preference does not apply.
        let o = LayoutOptions {
            routing: Routing::default(),
            ..LayoutOptions::layered_default()
        };

        let mut clear = multi_hop_past_a_blocker(-400.0);
        route(&mut clear, &o).unwrap();
        let r = &clear.routes[0];
        assert!(
            (r[0].1 - (clear.nodes[0].cross + 26.0)).abs() < 1e-6,
            "clear rank: expected the bottom face, got {:?}",
            r[0]
        );

        let mut blocked = multi_hop_past_a_blocker(200.0);
        route(&mut blocked, &o).unwrap();
        let r = &blocked.routes[0];
        assert!(
            (r[0].0 - (blocked.nodes[0].along + 60.0)).abs() < 1e-6,
            "blocked rank: expected the forward face, got {:?}",
            r[0]
        );
        let b = blocked.boxes();
        assert!(
            !r.windows(2).any(|w| seg_hits_box(w[0], w[1], &b, 0, 2)),
            "route still cuts a box: {r:?}"
        );
    }

    #[derive(Clone, Copy)]
    enum Company {
        /// The target has this one edge.
        None,
        /// Another chain whose riser and run a perpendicular entry would cut.
        RiserAcross,
        /// Three more straight edges into the target. Four ends do not fit a 52px border at an
        /// 18px gap.
        CrowdedBorder,
    }

    /// Source and dummy at cross 400, target at cross 0, so edge 0 approaches from far below.
    fn far_off_approach(company: Company) -> LayeredState {
        let node = || LayeredInputNode {
            width: 120.0,
            height: 52.0,
            shape: crate::geometry::shape::NodeShape::Box,
            constraint: None,
            category: None,
            clear_after: 0.0,
            clear_before: 0.0,
            port_sides: None,
            lane_key: None,
            along_before: 0.0,
            along_after: 0.0,
        };
        // Real nodes: 0 is the source, 1 the target, then the company's.
        let (n_real, edges): (usize, Vec<(usize, usize)>) = match company {
            Company::None => (2, vec![(0, 1)]),
            Company::RiserAcross => (4, vec![(0, 1), (2, 3)]),
            Company::CrowdedBorder => (5, vec![(0, 1), (2, 1), (3, 1), (4, 1)]),
        };
        let input = LayeredInput {
            nodes: (0..n_real).map(|_| node()).collect(),
            edges,
            weights: vec![],
            thickness: vec![],
            edge_label_sizes: vec![],
            seed: vec![],
            pinned: vec![],
            self_loops: vec![],
            ranked_backwards: Vec::new(),
            options: LayoutOptions {
                direction: crate::options::Direction::LeftRight,
                ..LayoutOptions::layered_default()
            },
        };
        let mut st = LayeredState::new(&input);
        let place = |st: &mut LayeredState, i: usize, rank: i32, along: f64, cross: f64| {
            st.nodes[i].rank = rank;
            st.nodes[i].along = along;
            st.nodes[i].cross = cross;
        };
        place(&mut st, 0, 0, 60.0, 400.0);
        place(&mut st, 1, 2, 660.0, 0.0);
        let d = st.add_dummy(1);
        place(&mut st, d, 1, 360.0, 400.0);
        let mut arcs = vec![
            LArc {
                from: 0,
                to: d,
                edge: 0,
                reversed: false,
            },
            LArc {
                from: d,
                to: 1,
                edge: 0,
                reversed: false,
            },
        ];
        st.chains.insert(0, vec![0, d, 1]);
        let mut layers = vec![vec![0], vec![d], vec![1]];
        match company {
            Company::None => {}
            Company::RiserAcross => {
                // 2 (rank 1, cross 500) -> dummy e (rank 2, cross 200) -> 3 (rank 3, cross 200).
                place(&mut st, 2, 1, 360.0, 500.0);
                place(&mut st, 3, 3, 960.0, 200.0);
                let e = st.add_dummy(2);
                place(&mut st, e, 2, 660.0, 200.0);
                arcs.push(LArc {
                    from: 2,
                    to: e,
                    edge: 1,
                    reversed: false,
                });
                arcs.push(LArc {
                    from: e,
                    to: 3,
                    edge: 1,
                    reversed: false,
                });
                st.chains.insert(1, vec![2, e, 3]);
                layers[1].push(2);
                layers[2].push(e);
                layers.push(vec![3]);
            }
            Company::CrowdedBorder => {
                // At or just above the target's height, so none clusters with the relieved edge.
                for (i, cross) in [(2usize, 0.0), (3, -5.0), (4, -10.0)] {
                    place(&mut st, i, 1, 360.0, cross);
                    arcs.push(LArc {
                        from: i,
                        to: 1,
                        edge: i - 1,
                        reversed: false,
                    });
                    st.chains.insert(i - 1, vec![i, 1]);
                    layers[1].push(i);
                }
            }
        }
        st.arcs = arcs;
        st.layers = layers;
        st.reindex();
        st
    }

    /// The border of node 1 (120x52 at (660, 0)) that edge 0 ends on.
    fn target_border(st: &LayeredState) -> Side {
        let end = st.routes[0].last().copied().unwrap_or((0.0, 0.0));
        if (end.1 - 26.0).abs() < 1e-6 {
            Side::Bottom
        } else if (end.1 + 26.0).abs() < 1e-6 {
            Side::Top
        } else if (end.0 - 600.0).abs() < 1e-6 {
            Side::Left
        } else {
            Side::Right
        }
    }

    /// A free relief enters through the bottom border unless `FacingOnly` is set. The node's own
    /// setting beats the drawing's.
    #[test]
    fn a_lone_far_off_approach_takes_the_perpendicular_border_unless_facing_only() {
        // Use plain routing defaults so the preset's port-side preference does not apply.
        let o = LayoutOptions {
            routing: Routing::default(),
            ..LayoutOptions::layered_default()
        };
        let mut st = far_off_approach(Company::None);
        route(&mut st, &o).unwrap();
        assert_eq!(target_border(&st), Side::Bottom);

        let mut st = far_off_approach(Company::None);
        st.nodes[1].port_sides = Some(PortSides::FacingOnly);
        route(&mut st, &o).unwrap();
        assert_eq!(target_border(&st), Side::Left);

        let mut facing = o;
        facing.routing.port_sides = PortSides::FacingOnly;
        let mut st = far_off_approach(Company::None);
        route(&mut st, &facing).unwrap();
        assert_eq!(target_border(&st), Side::Left);

        let mut st = far_off_approach(Company::None);
        st.nodes[1].port_sides = Some(PortSides::Any);
        route(&mut st, &facing).unwrap();
        assert_eq!(target_border(&st), Side::Bottom);

        let mut st = far_off_approach(Company::None);
        st.nodes[1].port_sides = Some(PortSides::LongEdgesAny);
        route(&mut st, &facing).unwrap();
        assert_eq!(target_border(&st), Side::Bottom);
    }

    /// A perpendicular entry that would cut a neighbour is not free, so the edge stays facing.
    #[test]
    fn a_perpendicular_entry_that_cuts_a_riser_stays_on_the_facing_border() {
        for sides in [
            PortSides::Any,
            PortSides::FacingPreferred,
            PortSides::FacingOnly,
            PortSides::LongEdgesAny,
        ] {
            let o = LayoutOptions {
                routing: Routing {
                    port_sides: sides,
                    ..Routing::default()
                },
                ..LayoutOptions::layered_default()
            };
            let mut st = far_off_approach(Company::RiserAcross);
            route(&mut st, &o).unwrap();
            assert_eq!(target_border(&st), Side::Left, "{sides:?}");
            let b = st.boxes();
            assert!(!st.routes[1]
                .windows(2)
                .any(|w| seg_hits_box(w[0], w[1], &b, 2, 3)));
        }
    }

    /// `FacingPreferred` stays facing until the facing border is full.
    #[test]
    fn facing_preferred_takes_the_perpendicular_border_only_to_relieve_the_facing_one() {
        let o = LayoutOptions {
            routing: Routing {
                port_sides: PortSides::FacingPreferred,
                ..Routing::default()
            },
            ..LayoutOptions::layered_default()
        };
        let mut st = far_off_approach(Company::None);
        route(&mut st, &o).unwrap();
        assert_eq!(target_border(&st), Side::Left);

        let mut st = far_off_approach(Company::CrowdedBorder);
        route(&mut st, &o).unwrap();
        assert_eq!(target_border(&st), Side::Bottom);
        for arc in 1..4 {
            let end = st.routes[arc].last().copied().unwrap();
            assert!((end.0 - 600.0).abs() < 1e-6, "arc {arc} ends at {end:?}");
        }
    }
}

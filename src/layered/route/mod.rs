//! Routing. [`route_edges`] turns ports and dummy chains into one polyline per arc, either
//! orthogonal staircases through gutter lanes or flow-style diagonals.
//!
//! Stages share one [`RouteCtx`] and run in order: `hints` resolves port hints, `sides` picks each
//! end's border, `spread` places ports along it (`diagonal` for radial fans on ellipses), `align`
//! makes local port fixes, `gutter` lays spines and lanes, `orthogonal` emits polylines, `repair`,
//! `separate` and `centre` clean them up, and `lanes` draws a bundle's lanes off its corridor.
//!
//! A bundle arc ([`super::bundle`]) is routed once but takes one port per lane. Every stage that
//! moves a port respects the caller's port hints ([`hints::Hints`]).

use std::collections::BTreeMap;

use super::ports::Port;
use super::state::LayeredState;
use crate::error::LayoutError;
use crate::geometry::hit::seg_hits_box;
use crate::geometry::Boxes;
use crate::graph::PortSides;
use crate::options::{LayoutOptions, RoutingStyle};
use crate::sort::SortDyn;

mod align;
mod centre;
mod diagonal;
mod geom;
mod gutter;
pub(in crate::layered) mod hints;
mod lanes;
mod orthogonal;
mod repair;
mod separate;
mod sides;
mod spread;

/// Max distance of a spread port from its border's middle, as a fraction of the border length.
/// Ports share the middle 90% and leave the corners free.
const PORT_SPAN: f64 = 0.45;

/// The port of every routed arc end, keyed by `(arc, is_source)`.
type PortMap = BTreeMap<(usize, bool), Port>;

/// The two ends (source, target) of every routed arc, by arc.
type ArcEnds = BTreeMap<usize, (sides::EdgeEnd, sides::EdgeEnd)>;

/// Read-only inputs shared by every stage of [`route_edges`], plus lookups cached for hot loops.
struct RouteCtx<'a> {
    st: &'a LayeredState,
    b: &'a Boxes,
    o: &'a LayoutOptions,
    /// Flow (diagonal) style. A port sits where the centre ray leaves the node.
    flow_edges: bool,
    /// The diagonal style's side-port variant (see [`repair::repair_side_ports`]).
    side_port: bool,
    chains: &'a BTreeMap<usize, Vec<usize>>,
    layer_of: Vec<i32>,
    /// The number of real nodes. Every index from here on is a routing dummy.
    n_in: usize,
    /// Every chained arc, ascending. Iterating in this order keeps port and lane tie-breaks
    /// deterministic.
    arc_keys: Vec<usize>,
    min_port_gap: f64,
    port_stroke_clear: f64,
    stub_len: f64,
    hints: hints::Hints,
}

impl<'a> RouteCtx<'a> {
    fn new(
        st: &'a LayeredState,
        b: &'a Boxes,
        o: &'a LayoutOptions,
        flow_edges: bool,
        side_port: bool,
    ) -> Self {
        let layer_of: Vec<i32> = st.nodes.iter().map(|nd| nd.rank).collect();
        let mut arc_keys: Vec<usize> = st.chains.keys().copied().collect();
        arc_keys.sort_unstable_by_dyn(Ord::cmp);
        RouteCtx {
            st,
            b,
            o,
            flow_edges,
            side_port,
            chains: &st.chains,
            layer_of,
            n_in: b.n_real,
            arc_keys,
            min_port_gap: o.spacing.port_gap,
            port_stroke_clear: o.spacing.port_stroke_clear,
            stub_len: o.spacing.stub,
            hints: hints::Hints::of(st, b),
        }
    }

    fn ordered(&self) -> impl Iterator<Item = (usize, &'a Vec<usize>)> + '_ {
        let chains = self.chains;
        self.arc_keys
            .iter()
            .filter_map(move |&a| chains.get(&a).map(|c| (a, c)))
    }

    /// Lane stroke widths of `arc` in corridor order, empty unless it is a bundle. Empty in
    /// diagonal style, which has no axis-aligned end to fan lanes from.
    fn lane_strokes(&self, arc: usize) -> &'a [f64] {
        if self.o.routing.style == RoutingStyle::Diagonal {
            return &[];
        }
        self.st
            .lanes
            .get(arc)
            .map(|v| v.as_slice())
            .filter(|v| v.len() > 1)
            .unwrap_or(&[])
    }

    /// Ports one end of `arc` asks for. A bundle wants one per lane, so border load counts lanes.
    fn lane_count(&self, arc: usize) -> usize {
        self.lane_strokes(arc).len().max(1)
    }

    fn thickness(&self, arc: usize) -> f64 {
        self.st.thickness.get(arc).copied().unwrap_or(2.0)
    }

    fn sides_of(&self, node: usize) -> PortSides {
        self.st
            .nodes
            .get(node)
            .and_then(|n| n.port_sides)
            .unwrap_or(self.o.routing.port_sides)
    }

    fn seg_hits(&self, p: (f64, f64), q: (f64, f64), s1: usize, s2: usize) -> bool {
        seg_hits_box(p, q, self.b, s1, s2)
    }

    fn poly_hits(&self, pts: &[(f64, f64)], s1: usize, s2: usize) -> bool {
        pts.windows(2).any(|w| self.seg_hits(w[0], w[1], s1, s2))
    }

    /// Flow edges on an ellipse spread around one radial fan instead of four borders.
    fn radial(&self, node: usize) -> bool {
        self.flow_edges && node < self.n_in && self.b.ellipse[node]
    }
}

/// One polyline per arc, plus one per lane for bundle arcs.
struct Routed {
    routes: Vec<Vec<(f64, f64)>>,
    lanes: Vec<Vec<Vec<(f64, f64)>>>,
}

/// Route every arc in flow space (layers left->right), one layer-ascending polyline per arc.
/// `side_port_used` (per arc, or empty) marks arcs whose route the `side_port` setting changed.
/// Ports are assigned jointly, so one change marks every arc.
fn route_edges(
    st: &LayeredState,
    b: &Boxes,
    o: &LayoutOptions,
    flow_edges: bool,
    side_port: bool,
    side_port_used: &mut [bool],
) -> Routed {
    let ctx = RouteCtx::new(st, b, o, flow_edges, side_port);

    let arc_ends = sides::assign_sides(&ctx, side_port_used);
    let (mut port, lane_offsets) = spread::spread_ports(&ctx, &arc_ends);
    align::uncross_bundles(&ctx, &mut port);
    align::align_channels(&ctx, &arc_ends, &mut port);
    align::align_facing(&ctx, &arc_ends, &mut port);
    align::relax_cross_side(&ctx, &arc_ends, &mut port);
    align::pair_direct(&ctx, &arc_ends, &mut port);

    let waypoints = gutter::waypoints(&ctx, &port);
    let lane = gutter::assign_lanes(&ctx, &waypoints);
    let mut routes = orthogonal::emit_routes(&ctx, &port, &waypoints, &lane, side_port_used);
    hints::leave_own_boxes(&ctx, &port, &mut routes);

    repair::flatten_steps(&ctx, &port, &mut routes);
    repair::nest_parallel_bundle_channels(
        &mut routes,
        ctx.chains,
        &b.cy,
        o.tuning.routing.bundle_nest_gap,
        |arc| ctx.hints.hinted(arc),
    );
    separate::separate_axis_runs(
        &mut routes,
        ctx.chains,
        b,
        o.spacing.port_gap,
        &o.tuning.routing,
        &ctx.hints,
    );
    repair::settle_endpoints_on_outline(&mut routes, ctx.chains, b);
    if o.routing.style == RoutingStyle::Orthogonal {
        // All arcs of an edge are turned round together.
        let mut reversed = vec![false; st.routes.len()];
        for a in st.arcs.iter().filter(|a| a.reversed) {
            if let Some(r) = reversed.get_mut(a.edge) {
                *r = true;
            }
        }
        // A hinted edge stays put so its other end keeps the route its go-around was built for.
        centre::centre_ports(
            &mut routes,
            ctx.chains,
            b,
            o,
            |arc| ctx.lane_strokes(arc).is_empty() && !ctx.hints.hinted(arc),
            |arc| reversed[arc],
            |arc| ctx.thickness(arc),
        );
    }

    // Lanes come last because the repairs above move whole runs and would leave them behind.
    let lanes = lanes::draw_lanes(&ctx, &port, &lane_offsets, &routes);
    Routed { routes, lanes }
}

/// Route every edge for the current style into [`LayeredState::routes`]. Diagonal style routes
/// with side ports, then [`repair::repair_side_ports`] reverts the edges that need plain ports.
pub fn route(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    let boxes = st.boxes();
    let routed = if o.routing.style == RoutingStyle::Diagonal {
        let mut used = vec![false; st.routes.len()];
        let mut routed = route_edges(st, &boxes, o, true, true, &mut used);
        repair::repair_side_ports(st, &boxes, o, &used, &mut routed.routes);
        routed
    } else {
        route_edges(st, &boxes, o, false, false, &mut [])
    };
    st.routes = routed.routes;
    st.lane_routes = routed.lanes;
    Ok(())
}

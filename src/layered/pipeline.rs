//! The layered phases and the order they run in.
//!
//! Every phase is a [`PhaseFn`], so the driver is a list, and [`Pipeline::layered`] is the
//! engine's order. Ports are chosen inside [`Phase::Route`] and bundling re-enters the driver, so
//! neither is a phase.

use crate::error::LayoutError;
use crate::options::{Compaction, LayoutOptions};

use super::compact::priority_compact;
use super::coord::{
    assign_layer_positions, centre_terminals, place_cross, reserve_lane_room, straighten_long_edges,
};
use super::dummy::{build_edge_chains, insert_dummies, reserve_stroke_width};
use super::group::{align_lanes, enforce_groups};
use super::label::{place_edge_labels, reserve_label_gaps};
use super::order::minimize_crossings;
use super::rank::assign_ranks;
use super::route::route;
use super::selfloop::{reserve_self_loops, route_self_loops};
use super::state::LayeredState;

/// The signature every phase has.
pub type PhaseFn = fn(&mut LayeredState, &LayoutOptions) -> Result<(), LayoutError>;

/// One step of a layered layout.
#[derive(Clone, Copy)]
pub enum Phase {
    /// Makes the graph acyclic, reversing as little weight as possible.
    BreakCycles,
    /// Assigns each node a layer.
    AssignRanks,
    /// Cuts every multi-layer arc into a chain of waypoints and rebuilds the layers.
    InsertDummies,
    /// Orders each layer to minimise crossings.
    MinimizeCrossings,
    /// Pulls same-group nodes into consistent lanes where that adds no crossings.
    EnforceGroups,
    /// Collects the waypoint chain of each caller edge.
    BuildChains,
    /// Swaps a chain's waypoints past layer neighbours where that adds no crossings and lets the
    /// chain run on one lane. The coordinate phases cannot reorder.
    OrderChains,
    /// Mirrors branches off the main path to balance each layer, where that adds no crossing.
    BalanceSides,
    /// Sizes each layer gap to hold the edge labels crossing it.
    ReserveLabels,
    /// Picks each node's self-loop border and reserves room in its clearance. Runs before
    /// placement so separation widens the right slot.
    ReserveSelfLoops,
    /// Places every node on the layer axis.
    PlaceAlong,
    /// Places every node on the order axis with [`crate::options::CoordOptions::placement`] or
    /// the caller's seeds, then straightens long edges.
    PlaceCross,
    /// Hops a long edge's waypoints over a node between them and their endpoint line.
    ChannelPullIn,
    /// Aligns each fork/join block with one branch straight or the two mirrored.
    AlignSplits,
    /// Centres first and last-layer terminals over their dominant flow.
    CentreTerminals,
    /// Removes slack from the order axis without changing the order.
    Compact,
    /// Pulls each group's band onto one slot of the order axis so it draws as one strip.
    AlignLanes,
    /// Straightens the heaviest source-to-sink flow where that costs little room.
    AlignMainPath,
    /// Pulls every long edge's waypoints back onto one lane. Runs after the last pass that moves
    /// nodes, because those passes shift a layer's dummies unevenly and leave staircases.
    StraightenChains,
    /// For chains the straightener could not fix, pins the chain on its preferred lane and moves
    /// the blocking nodes aside when that pays.
    FlattenChains,
    /// Widens each layer gap to fit its risers `lane_gap` apart and re-spaces the layers.
    ReserveLanes,
    /// Turns the placement into polylines.
    Route,
    /// Draws each self-loop on its reserved border, clear of the other edges' ports.
    RouteSelfLoops,
    /// Draws forks and joins on shared trunks if [`crate::options::Routing::merge_edges`] asks.
    MergeEdges,
    /// Wraps a drawing much wider than the viewport into rows if [`crate::options::WrapOptions`]
    /// asks.
    Wrap,
    /// Puts each edge label in its reserved gap.
    PlaceLabels,
    /// Measures the rectangles of strict swimlanes ([`crate::options::GroupLanes::Strict`]).
    MeasureLanes,
    /// For re-routing, reads layers, waypoints and order off the caller's positions.
    RankFromPositions,
    /// For re-routing, pins real nodes where they were dropped and places waypoints around them.
    PlaceOverPositions,
    /// A caller's own phase.
    #[cfg_attr(not(test), allow(dead_code))]
    Custom(PhaseFn),
}

impl Phase {
    /// The function this phase runs.
    pub fn function(self) -> PhaseFn {
        match self {
            Phase::BreakCycles => phase_break_cycles,
            Phase::AssignRanks => phase_assign_ranks,
            Phase::InsertDummies => phase_insert_dummies,
            Phase::MinimizeCrossings => phase_minimize_crossings,
            Phase::EnforceGroups => enforce_groups,
            Phase::BuildChains => phase_build_chains,
            Phase::OrderChains => super::chainorder::order_chains,
            Phase::BalanceSides => super::balance::balance_sides,
            Phase::ReserveLabels => phase_reserve_labels,
            Phase::ReserveSelfLoops => reserve_self_loops,
            Phase::PlaceAlong => phase_place_along,
            Phase::PlaceCross => phase_place_cross,
            Phase::ChannelPullIn => super::pullin::channel_pullin,
            Phase::AlignSplits => super::splits::align_splits,
            Phase::CentreTerminals => phase_centre_terminals,
            Phase::Compact => phase_compact,
            Phase::AlignLanes => align_lanes,
            Phase::AlignMainPath => super::mainpath::align_main_path,
            Phase::StraightenChains => straighten_long_edges,
            Phase::FlattenChains => super::flatten::flatten_chains,
            Phase::ReserveLanes => phase_reserve_lanes,
            Phase::Route => route,
            Phase::RouteSelfLoops => route_self_loops,
            Phase::MergeEdges => phase_merge_edges,
            Phase::Wrap => super::wrap::wrap_rows,
            Phase::PlaceLabels => phase_place_labels,
            Phase::MeasureLanes => super::group::measure_lanes,
            Phase::RankFromPositions => super::reroute::rank_from_positions,
            Phase::PlaceOverPositions => super::reroute::place_over_positions,
            Phase::Custom(f) => f,
        }
    }

    /// Run this phase on `st`.
    pub fn run(self, st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
        (self.function())(st, o)
    }

    /// The phase's snake_case name.
    #[cfg_attr(not(any(test, feature = "fixtures")), allow(dead_code))]
    pub fn name(self) -> &'static str {
        match self {
            Phase::BreakCycles => "break_cycles",
            Phase::AssignRanks => "assign_ranks",
            Phase::InsertDummies => "insert_dummies",
            Phase::MinimizeCrossings => "minimize_crossings",
            Phase::EnforceGroups => "enforce_groups",
            Phase::BuildChains => "build_chains",
            Phase::OrderChains => "order_chains",
            Phase::BalanceSides => "balance_sides",
            Phase::ReserveLabels => "reserve_labels",
            Phase::ReserveSelfLoops => "reserve_self_loops",
            Phase::PlaceAlong => "place_along",
            Phase::PlaceCross => "place_cross",
            Phase::ChannelPullIn => "channel_pullin",
            Phase::AlignSplits => "align_splits",
            Phase::CentreTerminals => "centre_terminals",
            Phase::Compact => "compact",
            Phase::AlignLanes => "align_lanes",
            Phase::AlignMainPath => "align_main_path",
            Phase::StraightenChains => "straighten_chains",
            Phase::FlattenChains => "flatten_chains",
            Phase::ReserveLanes => "reserve_lanes",
            Phase::Route => "route",
            Phase::RouteSelfLoops => "route_self_loops",
            Phase::MergeEdges => "merge_edges",
            Phase::Wrap => "wrap",
            Phase::PlaceLabels => "place_labels",
            Phase::MeasureLanes => "measure_lanes",
            Phase::RankFromPositions => "rank_from_positions",
            Phase::PlaceOverPositions => "place_over_positions",
            Phase::Custom(_) => "custom",
        }
    }
}

/// An ordered list of phases.
#[derive(Clone)]
pub struct Pipeline {
    /// The phases, in the order they run.
    pub phases: Vec<Phase>,
}

impl Pipeline {
    /// The phase order for a fresh layout.
    pub fn layered() -> Self {
        Pipeline {
            phases: vec![
                Phase::BreakCycles,
                Phase::AssignRanks,
                Phase::InsertDummies,
                Phase::MinimizeCrossings,
                Phase::EnforceGroups,
                Phase::BuildChains,
                Phase::OrderChains,
                Phase::BalanceSides,
                Phase::ReserveLabels,
                Phase::ReserveSelfLoops,
                Phase::PlaceAlong,
                Phase::PlaceCross,
                Phase::ChannelPullIn,
                Phase::AlignSplits,
                Phase::CentreTerminals,
                Phase::Compact,
                Phase::AlignLanes,
                Phase::AlignMainPath,
                Phase::StraightenChains,
                Phase::FlattenChains,
                Phase::ReserveLanes,
                Phase::Route,
                Phase::RouteSelfLoops,
                Phase::MergeEdges,
                Phase::Wrap,
                Phase::PlaceLabels,
                Phase::MeasureLanes,
            ],
        }
    }

    /// The phase order for [`crate::reroute`], which takes structure and coordinates from the
    /// caller's positions.
    pub fn reroute() -> Self {
        Pipeline {
            phases: vec![
                Phase::RankFromPositions,
                Phase::ReserveSelfLoops,
                Phase::PlaceOverPositions,
                Phase::Route,
                Phase::RouteSelfLoops,
                Phase::MergeEdges,
            ],
        }
    }

    /// Runs every phase in order, stopping at the first error.
    pub fn run(&self, st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
        for phase in &self.phases {
            phase.run(st, o)?;
        }
        Ok(())
    }
}

fn phase_break_cycles(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    if o.seed_mode == crate::options::SeedMode::Structure && super::stable::seeded_layers(st) {
        st.mode = super::state::RunMode::KeepStructure;
        return Ok(());
    }
    super::cycle::break_cycles_with(st, super::cycle::breaker_for(o.cycles.strategy))
}

fn phase_assign_ranks(st: &mut LayeredState, _o: &LayoutOptions) -> Result<(), LayoutError> {
    if st.keeps_structure() {
        return Ok(());
    }
    assign_ranks(st)
}

/// Waypoints become layer members only once the layers are rebuilt.
fn phase_insert_dummies(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    check_dummy_limit(st, o)?;
    insert_dummies(st)?;
    reserve_stroke_width(st, o.spacing.dummy_gap);
    st.rebuild_layers();
    Ok(())
}

/// Enforces [`crate::options::Limits::max_dummies`] before any waypoint is made, so an over-limit
/// drawing fails before the expensive phases.
pub(super) fn check_dummy_limit(st: &LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    let Some(limit) = o.limits.max_dummies else {
        return Ok(());
    };
    let got: usize = st
        .arcs
        .iter()
        .filter(|a| a.from != a.to)
        .map(|a| (st.nodes[a.to].rank - st.nodes[a.from].rank).unsigned_abs() as usize)
        .map(|span| span.saturating_sub(1))
        .sum();
    if got > limit {
        return Err(LayoutError::TooLarge {
            what: "routing waypoints",
            got,
            limit,
        });
    }
    Ok(())
}

fn phase_merge_edges(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    use crate::options::{MergeEdges, RoutingStyle};
    let (forks, joins) = match o.routing.merge_edges {
        MergeEdges::None => return Ok(()),
        MergeEdges::Forks => (true, false),
        MergeEdges::Joins => (false, true),
        MergeEdges::Both => (true, true),
    };
    if o.routing.style != RoutingStyle::Orthogonal {
        return Ok(());
    }
    // Bundled and hinted edges stay out. An arc is turned if exactly one of the cycle breaker
    // and the caller's ranking request reversed it.
    let none = (usize::MAX, usize::MAX, true);
    let mut ends = vec![none; st.routes.len()];
    let mut turned = vec![false; st.routes.len()];
    for arc in &st.arcs {
        if let Some(t) = turned.get_mut(arc.edge) {
            *t = arc.reversed != st.ranked_backwards.get(arc.edge).copied().unwrap_or(false);
        }
    }
    for (&edge, chain) in &st.chains {
        let (a, z) = (chain[0], chain[chain.len() - 1]);
        let bundled = st.lane_routes.get(edge).is_some_and(|l| !l.is_empty())
            || st.lanes.get(edge).is_some_and(|l| l.len() > 1);
        let hinted = st
            .port_hints
            .get(edge)
            .is_some_and(|h| h.iter().any(Option::is_some));
        if a != z && a < st.n_real && z < st.n_real && !bundled && !hinted {
            if let Some(slot) = ends.get_mut(edge) {
                *slot = (a, z, !turned[edge]);
            }
        }
    }
    let b = st.boxes();
    let mut routes = std::mem::take(&mut st.routes);
    let junctions =
        super::merge::merge_routes(&mut routes, &ends, &b, o.spacing.stub, forks, joins);
    st.routes = routes;
    st.junctions.extend(junctions);
    Ok(())
}

fn phase_reserve_lanes(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    reserve_lane_room(st, o)?;
    if st.keeps_structure() {
        super::stable::hold_layers(st)?;
    }
    Ok(())
}

fn phase_minimize_crossings(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    if st.keeps_structure() {
        super::stable::seeded_order(st, o);
        return Ok(());
    }
    minimize_crossings(st, o)
}

fn phase_build_chains(st: &mut LayeredState, _o: &LayoutOptions) -> Result<(), LayoutError> {
    build_edge_chains(st)
}

/// Labels are reserved only in the gap each one sits in (see `label::reserve_label_gaps`).
fn phase_reserve_labels(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    let sizes = std::mem::take(&mut st.label_sizes);
    st.gap_after = reserve_label_gaps(st, &sizes, o);
    st.label_sizes = sizes;
    Ok(())
}

fn phase_place_along(st: &mut LayeredState, _o: &LayoutOptions) -> Result<(), LayoutError> {
    let gap = std::mem::take(&mut st.gap_after);
    let result = assign_layer_positions(st, &gap);
    st.gap_after = gap;
    result
}

fn phase_place_cross(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    place_cross(st, o)?;
    if st.has_seeds() {
        // A pinned node keeps its seed on the layer axis too, off its column. There is no
        // re-layering, so only its own edges move.
        for node in st.nodes.iter_mut().take(st.n_real) {
            if node.pinned {
                if let Some((along, _)) = node.seed {
                    node.along = along;
                }
            }
        }
    }
    straighten_long_edges(st, o)
}

/// Centres terminals on their neighbours' median cross position when
/// [`crate::CoordOptions::terminal_centering`] is on.
fn phase_centre_terminals(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    if o.coords.terminal_centering && !st.keeps_structure() {
        centre_terminals(st, o)?;
    }
    Ok(())
}

/// Runs after the passes that could re-expand the layout. It keeps the order, so crossings hold.
fn phase_compact(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    if o.compaction.mode == Compaction::Priority && !st.keeps_structure() {
        priority_compact(st, o)?;
    }
    Ok(())
}

/// Gap widths do not depend on the cross axis, but label anchors do, so they are placed last.
fn phase_place_labels(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    let sizes = std::mem::take(&mut st.label_sizes);
    let gap = std::mem::take(&mut st.gap_after);
    st.label_anchors = place_edge_labels(st, &sizes, &gap, o);
    st.label_sizes = sizes;
    st.gap_after = gap;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Fails when a phase is added, removed or reordered.
    #[test]
    fn the_layered_pipeline_is_the_documented_order() {
        let names: Vec<&str> = Pipeline::layered()
            .phases
            .iter()
            .map(|p| p.name())
            .collect();
        assert_eq!(
            names,
            vec![
                "break_cycles",
                "assign_ranks",
                "insert_dummies",
                "minimize_crossings",
                "enforce_groups",
                "build_chains",
                "order_chains",
                "balance_sides",
                "reserve_labels",
                "reserve_self_loops",
                "place_along",
                "place_cross",
                "channel_pullin",
                "align_splits",
                "centre_terminals",
                "compact",
                "align_lanes",
                "align_main_path",
                "straighten_chains",
                "flatten_chains",
                "reserve_lanes",
                "route",
                "route_self_loops",
                "merge_edges",
                "wrap",
                "place_labels",
                "measure_lanes",
            ]
        );
    }

    /// A custom phase can be spliced into the list.
    #[test]
    fn a_custom_phase_runs_like_any_other() {
        fn shift(st: &mut LayeredState, _o: &LayoutOptions) -> Result<(), LayoutError> {
            for node in &mut st.nodes {
                node.cross += 1000.0;
            }
            Ok(())
        }
        let mut pipeline = Pipeline::layered();
        pipeline.phases.push(Phase::Custom(shift));
        assert_eq!(pipeline.phases.len(), 28);
    }
}

//! Egress turns the engine's flat result vectors into the public [`Layout`]. Nothing here
//! changes a coordinate.

use crate::graph::{EdgeId, Graph};
use crate::layout::{EdgeRoute, Layout, LayoutStats, Placement, PortInfo, RouteKind};
use crate::options::LayoutOptions;

use super::prepare::Prepared;
use super::selfloop::{LoopAnchors, LoopRoutes};

/// A strict swimlane: group id (`None` for the ungrouped lane) and `(min, max)` corners.
pub(crate) type OutputLane = (Option<u32>, (f64, f64), (f64, f64));

/// One pipeline run's result in output coordinates, indexed by engine node and edge. Engine edges
/// are the self-loop-free subsequence of the caller's, see [`Prepared::engine_edge`].
#[derive(Debug, Clone, Default)]
pub(crate) struct LayeredOutput {
    pub centers: Vec<(f64, f64)>,
    /// Border to border, source first.
    pub routes: Vec<Vec<(f64, f64)>>,
    pub label_anchors: Vec<Option<(f64, f64)>>,
    /// `None` for a node placed outside the layered drawing.
    pub ranks: Vec<Option<i32>>,
    pub orders: Vec<u32>,
    pub spans: Vec<u32>,
    pub reversed: Vec<bool>,
    pub lanes: Vec<Option<u32>>,
    /// Per bundled edge, one polyline per lane in corridor order. Empty for a single-line edge.
    pub lane_routes: Vec<Vec<Vec<(f64, f64)>>>,
    pub layer_count: u32,
    pub dummy_count: u32,
    /// Keyed by caller edge index, since self-loops have no engine edge.
    pub loop_routes: LoopRoutes,
    pub loop_label_anchors: LoopAnchors,
    pub junctions: Vec<(f64, f64)>,
    pub swimlanes: Vec<OutputLane>,
}

impl LayeredOutput {
    pub fn empty(n: usize, m: usize) -> Self {
        LayeredOutput {
            centers: vec![(0.0, 0.0); n],
            routes: vec![vec![]; m],
            label_anchors: vec![None; m],
            ranks: vec![None; n],
            orders: vec![0; n],
            spans: vec![0; m],
            reversed: vec![false; m],
            lanes: vec![None; m],
            lane_routes: vec![vec![]; m],
            layer_count: 0,
            dummy_count: 0,
            loop_routes: LoopRoutes::new(),
            loop_label_anchors: LoopAnchors::new(),
            junctions: vec![],
            swimlanes: vec![],
        }
    }
}

/// Self-loops have no engine arc, so their bumps come from `out.loop_routes` and go back into
/// their caller edge slots here, keeping `layout.edges` index-parallel to `graph.edges`.
pub(crate) fn emit(g: &Graph, _o: &LayoutOptions, out: &LayeredOutput, prep: &Prepared) -> Layout {
    let nodes: Vec<Placement> = g
        .nodes
        .iter()
        .enumerate()
        .map(|(i, nd)| Placement {
            center: out.centers.get(i).copied().unwrap_or((0.0, 0.0)).into(),
            size: nd.size,
            rank: out.ranks.get(i).copied().flatten(),
            order: out.orders.get(i).copied().unwrap_or(0),
        })
        .collect();

    let mut reversed: Vec<EdgeId> = Vec::new();
    let mut edges: Vec<EdgeRoute> = Vec::with_capacity(g.edges.len());
    for (id, e) in g.edges() {
        let Some(engine) = prep.engine_edge.get(id.index()).copied().flatten() else {
            let points: Vec<crate::geometry::Point> = out
                .loop_routes
                .get(&id.index())
                .map(|r| r.iter().map(|&p| p.into()).collect())
                .unwrap_or_default();
            let anchor = out
                .loop_label_anchors
                .get(&id.index())
                .copied()
                .map(Into::into);
            edges.push(match nodes.get(e.source.index()) {
                Some(at) if points.len() >= 2 => report_hinted_sides(
                    EdgeRoute::routed(points, RouteKind::SelfLoop, at, at, anchor),
                    prep,
                    id,
                    (at, at),
                ),
                // No bump (a degenerate node, or the phase was skipped), so none is invented.
                _ => EdgeRoute {
                    kind: RouteKind::SelfLoop,
                    label_anchor: anchor,
                    ..EdgeRoute::default()
                },
            });
            continue;
        };
        // `out.reversed` is the engine's reversal and `prep.pre_reversed` is the one `ingest` made
        // for an edge ranked against its direction. The edge is reversed when the two disagree.
        let pre = prep.pre_reversed.get(id.index()).copied().unwrap_or(false);
        let flipped = out.reversed.get(engine).copied().unwrap_or(false) != pre;
        if flipped {
            reversed.push(id);
        }
        let mut points: Vec<crate::geometry::Point> = out
            .routes
            .get(engine)
            .map(|r| r.iter().map(|&p| p.into()).collect())
            .unwrap_or_default();
        // The engine emits source first, and under `pre_reversed` that is the caller's target.
        if pre {
            points.reverse();
        }
        // A bundled lane says more about the drawing than the span count, so it wins.
        let kind = match (
            out.lanes.get(engine).copied().flatten(),
            out.spans.get(engine).copied().unwrap_or(0),
        ) {
            (Some(lane), _) => RouteKind::Bundled { lane },
            (None, spans) if spans > 1 => RouteKind::Chained { spans },
            _ => RouteKind::Direct,
        };
        let label_anchor = out
            .label_anchors
            .get(engine)
            .copied()
            .flatten()
            .map(Into::into);
        let source = nodes.get(e.source.index());
        let target = nodes.get(e.target.index());
        edges.push(match (source, target) {
            (Some(s), Some(t)) => report_hinted_sides(
                EdgeRoute::routed(points, kind, s, t, label_anchor),
                prep,
                id,
                (s, t),
            ),
            _ => EdgeRoute::default(),
        });
    }

    let mut layout = Layout {
        nodes,
        edges,
        bounds: Default::default(),
        layer_count: out.layer_count,
        reversed,
        stats: LayoutStats {
            dummy_count: out.dummy_count,
        },
        junctions: out.junctions.iter().map(|&p| p.into()).collect(),
        lanes: out
            .swimlanes
            .iter()
            .map(|&(group, min, max)| crate::layout::Lane {
                group: group.map(crate::graph::GroupId),
                rect: crate::geometry::Rect {
                    min: min.into(),
                    max: max.into(),
                },
            })
            .collect(),
    };
    layout.recompute_bounds();
    layout
}

/// Reports each hinted port on its hinted border. [`EdgeRoute::routed`] reads the border off the
/// point, which misreads a port far along an ellipse's top as being on the side.
fn report_hinted_sides(
    mut route: EdgeRoute,
    prep: &Prepared,
    id: EdgeId,
    (source_at, target_at): (&Placement, &Placement),
) -> EdgeRoute {
    let [source, target] = prep
        .hinted_sides
        .get(id.index())
        .copied()
        .unwrap_or_default();
    for (port, hint, at) in [
        (route.source_port.as_mut(), source, source_at),
        (route.target_port.as_mut(), target, target_at),
    ] {
        if let (Some(port), Some(side)) = (port, hint) {
            *port = PortInfo::on(port.point, side, at);
        }
    }
    route
}

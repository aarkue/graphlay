//! Parallel-arc bundling. Arcs sharing a (source, target) pair are laid out as one edge with the
//! summed weight and a reserved corridor, so they stay together.
//!
//! [`super::route`] gives each lane its own port and returns one polyline per lane. A diagonal
//! route has no axis-aligned end segment to fan out of, so its bundle falls back to
//! `offset_route` copies.

use std::collections::BTreeMap;

use crate::error::LayoutError;
use crate::geometry::polyline::offset_route;
use crate::geometry::shape::Outline;
use crate::options::{LayoutOptions, Routing};

use super::finish::LayeredOutput;
use super::prepare::{EdgePortHints, LayeredInput};

use super::layout_layered_inner;

/// Returns `None` if there is no bundle to collapse. An arc with a port hint is never bundled,
/// since a lane of a shared corridor cannot honour the caller's port placement.
pub(crate) fn bundle_parallel_arcs(
    input: &LayeredInput,
    hints: &[EdgePortHints],
) -> Result<Option<LayeredOutput>, LayoutError> {
    let n_arcs = input.edges.len();
    let hinted = |arc: usize| {
        hints
            .get(arc)
            .is_some_and(|h| h.iter().any(Option::is_some))
    };
    // The arc itself joins the key for a hinted arc, which keeps it a bundle of one.
    let mut of_pair: BTreeMap<(usize, usize, Option<usize>), usize> = BTreeMap::new();
    let mut bundles: Vec<Vec<usize>> = vec![];
    let mut bundle_of: Vec<usize> = Vec::with_capacity(n_arcs);
    for (arc, &(a, b)) in input.edges.iter().enumerate() {
        let key = (a, b, hinted(arc).then_some(arc));
        let idx = *of_pair.entry(key).or_insert_with(|| {
            bundles.push(vec![]);
            bundles.len() - 1
        });
        bundles[idx].push(arc);
        bundle_of.push(idx);
    }
    if bundles.iter().all(|b| b.len() < 2) {
        return Ok(None);
    }

    let t_of = |arc: usize| {
        if input.thickness.len() == n_arcs {
            input.thickness[arc]
        } else {
            2.0
        }
    };
    let w_of = |arc: usize| {
        if input.weights.len() == n_arcs {
            input.weights[arc]
        } else {
            1.0
        }
    };

    // Lane offsets are centred on the bundle route.
    let mut lane_off: Vec<f64> = vec![0.0; n_arcs];
    let mut corridor: Vec<f64> = Vec::with_capacity(bundles.len());
    for arcs in &bundles {
        let mut pos = vec![0.0f64; arcs.len()];
        for i in 1..arcs.len() {
            pos[i] = pos[i - 1]
                + (t_of(arcs[i - 1]) + t_of(arcs[i])) / 2.0
                + input.options.spacing.bundle_lane_gap;
        }
        let span = pos[arcs.len() - 1];
        for (i, &arc) in arcs.iter().enumerate() {
            lane_off[arc] = pos[i] - span / 2.0;
        }
        corridor.push(span + (t_of(arcs[0]) + t_of(arcs[arcs.len() - 1])) / 2.0);
    }

    // Lane `i` of a routed bundle is `arcs[i]`, so the router's lanes map straight back to arcs.
    let lane_strokes: Vec<Vec<f64>> = bundles
        .iter()
        .map(|arcs| arcs.iter().map(|&a| t_of(a)).collect())
        .collect();

    let b_edges: Vec<(usize, usize)> = bundles.iter().map(|arcs| input.edges[arcs[0]]).collect();
    let b_weights: Vec<f64> = bundles
        .iter()
        .map(|arcs| arcs.iter().map(|&a| w_of(a)).sum())
        .collect();
    let b_labels: Vec<(f64, f64)> = if input.edge_label_sizes.len() == n_arcs {
        bundles
            .iter()
            .map(|arcs| {
                arcs.iter().fold((0.0f64, 0.0f64), |(w, h), &a| {
                    let (aw, ah) = input.edge_label_sizes[a];
                    (w.max(aw), h.max(ah))
                })
            })
            .collect()
    } else {
        vec![]
    };
    let bundled = LayeredInput {
        nodes: input.nodes.clone(),
        edges: b_edges,
        weights: b_weights,
        thickness: corridor,
        edge_label_sizes: b_labels,
        seed: input.seed.clone(),
        pinned: input.pinned.clone(),
        self_loops: input.self_loops.clone(),
        // The first arc speaks for the corridor's direction, as it does for its endpoints.
        ranked_backwards: if input.ranked_backwards.is_empty() {
            Vec::new()
        } else {
            bundles
                .iter()
                .map(|arcs| input.ranked_backwards[arcs[0]])
                .collect()
        },
        // The inner run must not bundle again, and placing labels against corridors that become
        // k lanes would be wasted. It only reserves the room and stops at the gap centre.
        options: LayoutOptions {
            routing: Routing {
                bundle_parallel: false,
                ..input.options.routing
            },
            labels: crate::options::LabelOptions {
                placement: crate::options::LabelPlacement::GapCentre,
                ..input.options.labels
            },
            ..input.options
        },
    };
    // A real bundle never contains a hinted arc, so only a bundle of one keeps hints.
    let b_hints: Vec<EdgePortHints> = if hints.is_empty() {
        Vec::new()
    } else {
        bundles
            .iter()
            .map(|arcs| match arcs[..] {
                [arc] => hints.get(arc).copied().unwrap_or_default(),
                _ => [None, None],
            })
            .collect()
    };
    let out = layout_layered_inner(&bundled, &lane_strokes, &b_hints)?;

    let mut lane_of: Vec<usize> = vec![0; n_arcs];
    for arcs in &bundles {
        for (lane, &arc) in arcs.iter().enumerate() {
            lane_of[arc] = lane;
        }
    }
    let routes: Vec<Vec<(f64, f64)>> = (0..n_arcs)
        .map(|arc| {
            let bl = bundle_of[arc];
            if let Some(lane) = out
                .lane_routes
                .get(bl)
                .filter(|lanes| lanes.len() == bundles[bl].len())
                .and_then(|lanes| lanes.get(lane_of[arc]))
            {
                return lane.clone();
            }
            // A diagonal route has no axis-aligned end segment, so the router drew no lanes for it.
            let route = &out.routes[bl];
            if route.len() < 2 || lane_off[arc] == 0.0 {
                return route.clone();
            }
            let (from, to) = input.edges[arc];
            let outline = |i: usize| Outline {
                center: out.centers[i],
                size: (input.nodes[i].width, input.nodes[i].height),
                shape: input.nodes[i].shape,
            };
            offset_route(route, lane_off[arc], outline(from), outline(to))
        })
        .collect();
    // The inner run reserved one label slot per bundle, not per arc.
    let reserved: Vec<Option<(f64, f64)>> = (0..n_arcs)
        .map(|arc| out.label_anchors.get(bundle_of[arc]).copied().flatten())
        .collect();
    // One slot for k arcs would stack k labels, so placement runs again over the expanded lanes, in
    // output space.
    let label_anchors = if input.edge_label_sizes.len() == n_arcs
        && input.options.labels.placement == crate::options::LabelPlacement::Candidates
    {
        let nodes: Vec<((f64, f64), (f64, f64))> = out
            .centers
            .iter()
            .zip(&input.nodes)
            .map(|(&c, n)| (c, (n.width / 2.0, n.height / 2.0)))
            .collect();
        let halves: Vec<Option<(f64, f64)>> = input
            .edge_label_sizes
            .iter()
            .map(|&(w, h)| (w > 0.0 && h > 0.0).then_some((w / 2.0, h / 2.0)))
            .collect();
        let thickness: Vec<f64> = (0..n_arcs).map(t_of).collect();
        let scene = super::label::LabelScene {
            nodes: &nodes,
            routes: &routes,
            loops: out.loop_routes.iter().map(|(&e, r)| (e, &r[..])).collect(),
            thickness: &thickness,
            labels: &halves,
            reserved: &reserved,
        };
        super::label::place_labels(&scene, &input.options)
    } else {
        reserved
    };
    // A bundle's span and reversal apply to every lane. A group of one is not a bundle.
    let mut spans = vec![0u32; n_arcs];
    let mut reversed = vec![false; n_arcs];
    let mut lanes: Vec<Option<u32>> = vec![None; n_arcs];
    for (bl, arcs) in bundles.iter().enumerate() {
        for (lane, &arc) in arcs.iter().enumerate() {
            spans[arc] = out.spans.get(bl).copied().unwrap_or(0);
            reversed[arc] = out.reversed.get(bl).copied().unwrap_or(false);
            if arcs.len() > 1 {
                lanes[arc] = Some(lane as u32);
            }
        }
    }
    Ok(Some(LayeredOutput {
        centers: out.centers,
        routes,
        label_anchors,
        ranks: out.ranks,
        orders: out.orders,
        spans,
        reversed,
        lanes,
        // The lanes are drawn, so nothing downstream expands a bundle again.
        lane_routes: vec![vec![]; n_arcs],
        layer_count: out.layer_count,
        dummy_count: out.dummy_count,
        loop_routes: out.loop_routes,
        loop_label_anchors: out.loop_label_anchors,
        junctions: out.junctions,
        swimlanes: out.swimlanes,
    }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use crate::geometry::fp;
    use crate::geometry::Size;
    use crate::graph::{Edge, Graph, Node, NodeId};
    use crate::layout::RouteKind;
    use crate::options::LayoutOptions;

    /// `n` parallel edges plus one plain edge. The boxes are tall enough for four ports at the
    /// default port gap.
    fn parallel(n: usize) -> Graph {
        let node = || Node {
            size: Size {
                width: 160.0,
                height: 140.0,
            },
            ..Node::default()
        };
        Graph {
            nodes: vec![node(), node(), node()],
            edges: (0..n)
                .map(|_| Edge::new(NodeId(0), NodeId(1)))
                .chain([Edge::new(NodeId(0), NodeId(2))])
                .collect(),
        }
    }

    /// Each lane leaves its own port with an axis-aligned segment and at least a port gap of space.
    #[test]
    fn every_parallel_edge_gets_a_port_of_its_own() {
        let o = LayoutOptions::layered_default();
        let g = parallel(4);
        let layout = crate::layout(&g, &o).unwrap();
        let mut starts: Vec<crate::geometry::Point> = vec![];
        let mut ends: Vec<crate::geometry::Point> = vec![];
        for (i, e) in layout.edges.iter().enumerate().take(4) {
            assert!(
                matches!(e.kind, RouteKind::Bundled { .. }),
                "edge {i} is not a lane: {:?}",
                e.kind
            );
            let pts = &e.points;
            assert!(pts.len() >= 2, "edge {i} unrouted");
            let n = pts.len();
            for (p, q) in [(pts[0], pts[1]), (pts[n - 2], pts[n - 1])] {
                assert!(
                    (p.x - q.x).abs() < 0.5 || (p.y - q.y).abs() < 0.5,
                    "edge {i} leaves its port at a slant: {p:?} -> {q:?}"
                );
            }
            starts.push(pts[0]);
            ends.push(pts[n - 1]);
        }
        // The same rule the `min_port_gap` metric measures.
        for ports in [&starts, &ends] {
            for (i, p) in ports.iter().enumerate() {
                for q in ports.iter().skip(i + 1) {
                    assert!(
                        fp::hypot(p.x - q.x, p.y - q.y) >= o.spacing.port_gap - 1e-6,
                        "two lanes share a port: {ports:?}"
                    );
                }
            }
        }
    }

    /// The isolated-node split re-enters the driver, so the lane table has to travel with it.
    #[test]
    fn the_isolated_node_split_keeps_the_lanes() {
        let o = LayoutOptions::layered_default();
        let mut g = parallel(4);
        g.nodes.push(g.nodes[0].clone());
        let layout = crate::layout(&g, &o).unwrap();
        let mut ports: Vec<crate::geometry::Point> = vec![];
        for e in layout.edges.iter().take(4) {
            assert!(matches!(e.kind, RouteKind::Bundled { .. }), "{:?}", e.kind);
            ports.push(e.points[0]);
        }
        for (i, p) in ports.iter().enumerate() {
            for q in ports.iter().skip(i + 1) {
                assert!(
                    fp::hypot(p.x - q.x, p.y - q.y) >= o.spacing.port_gap - 1e-6,
                    "two lanes share a port past the split: {ports:?}"
                );
            }
        }
    }

    /// The bundle reserves label room as one edge, so each arc needs its own box after expansion.
    #[test]
    fn every_lane_of_a_bundle_gets_its_own_label_box() {
        let o = LayoutOptions::layered_default();
        let mut g = parallel(4);
        for e in g.edges.iter_mut() {
            e.label_size = Some(Size {
                width: 60.0,
                height: 14.0,
            });
        }
        let layout = crate::layout(&g, &o).unwrap();
        let boxes: Vec<crate::geometry::Point> = layout
            .edges
            .iter()
            .take(4)
            .filter_map(|e| e.label_anchor)
            .collect();
        assert_eq!(boxes.len(), 4, "a lane lost its label: {boxes:?}");
        for (i, p) in boxes.iter().enumerate() {
            for q in boxes.iter().skip(i + 1) {
                assert!(
                    (p.x - q.x).abs() >= 60.0 || (p.y - q.y).abs() >= 14.0,
                    "two lanes' labels overlap: {boxes:?}"
                );
            }
        }
    }

    #[test]
    fn a_single_edge_is_not_a_bundle() {
        let o = LayoutOptions::layered_default();
        let layout = crate::layout(&parallel(1), &o).unwrap();
        for e in &layout.edges {
            assert!(!matches!(e.kind, RouteKind::Bundled { .. }), "{:?}", e.kind);
        }
    }
}

//! The layered (Sugiyama-style) engine and its driver.
//!
//! There is one module per phase: cycle breaking ([`cycle`]), layering ([`rank`]), dummy insertion
//! ([`dummy`]), crossing minimisation ([`order`]) with group lanes ([`group`]) and long-edge lanes
//! ([`chainorder`]), coordinate assignment ([`coord`], or [`simplex`] with the optimal-face walk
//! [`face`]), channel pull-in ([`pullin`]), fork/join block alignment ([`splits`]), optional
//! compaction ([`compact`]), then ports ([`ports`]) and routing ([`route`]). [`pipeline`] lists
//! the order they run in. [`bundle`] and [`reroute`] are alternative front ends over the same
//! phases. [`prepare`] builds the input and [`finish`] emits the result.
//!
//! Every phase works in flow space, with the layer axis as x and the order axis as y. The driver
//! transposes once on output, so no phase needs to know the layout direction.

pub mod balance;
pub mod bundle;
pub mod chainorder;
pub mod compact;
pub mod coord;
pub mod cycle;
pub mod dummy;
pub mod face;
pub mod finish;
pub mod flatten;
pub mod group;
pub mod label;
pub mod mainpath;
pub mod merge;
pub mod order;
pub mod pipeline;
pub mod ports;
pub mod prepare;
#[cfg(feature = "fixtures")]
pub mod profile;
pub mod pullin;
pub mod rank;
pub mod reroute;
pub mod route;
pub mod selfloop;
pub mod simplex;
pub mod splits;
pub mod stable;
pub mod state;
pub mod steps;
pub mod wrap;

use std::collections::BTreeMap;

use crate::error::LayoutError;

use bundle::bundle_parallel_arcs;
use finish::LayeredOutput;
use pipeline::Pipeline;
use prepare::{EdgePortHints, LayeredInput, LayeredInputNode};
use selfloop::LoopSlot;
use state::LayeredState;

/// Runs the layered pipeline on a [`LayeredInput`] and returns node centres and edge routes in
/// output coordinates. `hints` is [`prepare::Prepared::port_hints`], or empty.
pub(crate) fn layout_layered(
    input: &LayeredInput,
    hints: &[EdgePortHints],
) -> Result<LayeredOutput, LayoutError> {
    input.validate()?;
    if input.nodes.is_empty() {
        return Ok(LayeredOutput::empty(0, input.edges.len()));
    }
    layout_layered_inner(input, &[], hints)
}

/// Lays out `input`. Parallel-arc bundling and the isolated-node split call back into this.
///
/// `lanes` holds, per edge, the stroke widths of the parallel arcs a bundle stands for, in
/// corridor order, or is empty. `hints` holds the port hints per edge, or is empty.
pub(crate) fn layout_layered_inner(
    input: &LayeredInput,
    lanes: &[Vec<f64>],
    hints: &[EdgePortHints],
) -> Result<LayeredOutput, LayoutError> {
    let n_in = input.nodes.len();
    let n_edges = input.edges.len();
    if n_in == 0 {
        return Ok(LayeredOutput::empty(0, n_edges));
    }

    // Parallel arcs routed one by one scatter, so they are laid out as one edge with lanes.
    if input.options.routing.bundle_parallel {
        if let Some(out) = bundle_parallel_arcs(input, hints)? {
            return Ok(out);
        }
    }

    // Edgeless nodes have no layering information and would pile into one column, so they are
    // packed into a grid below the connected layout.
    let mut degree = vec![0usize; n_in];
    for &(a, b) in &input.edges {
        degree[a] += 1;
        degree[b] += 1;
    }
    let isolated: Vec<usize> = (0..n_in)
        .filter(|&i| {
            degree[i] == 0
                && !matches!(
                    input.nodes[i].constraint,
                    Some(state::LayerConstraint::Fixed(_))
                )
        })
        .collect();
    // A fully seeded structure-mode layout keeps each orphan at its seed instead of the grid.
    let keep_in_place = input.options.seed_mode == crate::options::SeedMode::Structure
        && input.seed.len() == n_in
        && input.seed.iter().all(Option::is_some);
    // Strict swimlanes hold every node, so an edgeless node stays in the run to land in its lane.
    let strict = input.options.order.group_lanes == crate::options::GroupLanes::Strict;
    if !isolated.is_empty() && isolated.len() < n_in && !strict {
        let active: Vec<usize> = (0..n_in).filter(|&i| degree[i] > 0).collect();
        // `BTreeMap::from_iter` would pull a stable sort into the binary (see `cycle::Peel::new`).
        let mut remap: BTreeMap<usize, usize> = BTreeMap::new();
        for (new, &old) in active.iter().enumerate() {
            remap.insert(old, new);
        }
        let sub = LayeredInput {
            nodes: active
                .iter()
                .map(|&i| LayeredInputNode {
                    width: input.nodes[i].width,
                    height: input.nodes[i].height,
                    shape: input.nodes[i].shape,
                    constraint: input.nodes[i].constraint,
                    category: input.nodes[i].category,
                    clear_after: input.nodes[i].clear_after,
                    clear_before: input.nodes[i].clear_before,
                    port_sides: input.nodes[i].port_sides,
                    lane_key: input.nodes[i].lane_key,
                    along_before: input.nodes[i].along_before,
                    along_after: input.nodes[i].along_after,
                })
                .collect(),
            edges: input
                .edges
                .iter()
                .map(|&(a, b)| (remap[&a], remap[&b]))
                .collect(),
            weights: input.weights.clone(),
            thickness: input.thickness.clone(),
            edge_label_sizes: input.edge_label_sizes.clone(),
            seed: if input.seed.is_empty() {
                vec![]
            } else {
                active.iter().map(|&i| input.seed[i]).collect()
            },
            pinned: if input.pinned.is_empty() {
                vec![]
            } else {
                active.iter().map(|&i| input.pinned[i]).collect()
            },
            // Orphans' loops are drawn below, where the orphans are placed.
            self_loops: input
                .self_loops
                .iter()
                .filter_map(|slot| {
                    remap.get(&slot.node).map(|&node| LoopSlot {
                        node,
                        ..slot.clone()
                    })
                })
                .collect(),
            ranked_backwards: input.ranked_backwards.clone(),
            options: input.options,
        };
        // The edge list is unchanged, so a bundle's lane table still lines up.
        let sub_out = layout_layered_inner(&sub, lanes, hints)?;
        let mut centers = vec![(0.0, 0.0); n_in];
        let (mut min_x, mut max_x, mut max_y) =
            (f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for (new, &old) in active.iter().enumerate() {
            centers[old] = sub_out.centers[new];
            let (hw, hh) = (input.nodes[old].width / 2.0, input.nodes[old].height / 2.0);
            min_x = min_x.min(centers[old].0 - hw);
            max_x = max_x.max(centers[old].0 + hw);
            max_y = max_y.max(centers[old].1 + hh);
        }
        if !min_x.is_finite() {
            (min_x, max_x, max_y) = (0.0, 0.0, 0.0);
        }
        // An orphan can still carry self-loops, so a grid cell must also fit the widest loop.
        let axis = state::Axis::of(input.options.direction);
        // A `Vec` saves a `BTreeMap` instantiation in the binary.
        let mut loops_of: Vec<Option<&selfloop::LoopSlot>> = vec![None; n_in];
        for slot in &input.self_loops {
            if let Some(cell) = loops_of.get_mut(slot.node) {
                *cell = Some(slot);
            }
        }
        // Orphans are placed after the transpose, so this works in output space.
        let orphan_side = |slot: &LoopSlot| {
            slot.hint
                .unwrap_or_else(|| axis.to_output_side(orphan_loop_side(&input.options)))
        };
        let loop_reach = isolated
            .iter()
            .filter_map(|&i| loops_of.get(i).copied().flatten())
            .map(|slot| {
                slot.reach(
                    axis.to_flow_side(orphan_side(slot)),
                    axis,
                    &input.options.self_loops,
                )
            })
            .fold(0.0f64, f64::max);
        // Width and height are sized separately so wide, short boxes get short rows.
        let extent = |size: fn(&LayeredInputNode) -> f64| {
            isolated
                .iter()
                .map(|&i| size(&input.nodes[i]))
                .fold(0.0, f64::max)
                + 2.0 * loop_reach
                + input.options.spacing.isolated_gap
        };
        let (cell_w, cell_h) = (extent(|n| n.width), extent(|n| n.height));
        // Wrap to the drawing's width, but use at least a square grid so a narrow drawing does
        // not get a tall tower of orphans.
        let fit = ((max_x - min_x) / cell_w).floor().max(0.0) as usize + 1;
        let square = (isolated.len() as f64).sqrt().ceil() as usize;
        let cols = fit
            .max(square)
            .min(isolated.len())
            .clamp(1, input.options.spacing.isolated_max_columns);
        let row_w = cols as f64 * cell_w;
        let start = (min_x + max_x) / 2.0 - row_w / 2.0 + cell_w / 2.0;
        let top = max_y + input.options.spacing.isolated_gap + cell_h / 2.0;
        let mut loop_routes = sub_out.loop_routes.clone();
        let mut loop_label_anchors = sub_out.loop_label_anchors.clone();
        // An orphan kept in place moves by the connected nodes' mean shift from their seeds. If
        // that spot collides with something it takes the next grid cell instead.
        let mut slot_of: Vec<Option<(f64, f64)>> = vec![None; n_in];
        let mut grid_top = top;
        if keep_in_place {
            let k = active.len().max(1) as f64;
            let (dx, dy) = active.iter().fold((0.0, 0.0), |(x, y), &i| {
                let s = input.seed[i].unwrap_or(centers[i]);
                (x + (centers[i].0 - s.0) / k, y + (centers[i].1 - s.1) / k)
            });
            let half = |i: usize| (input.nodes[i].width / 2.0, input.nodes[i].height / 2.0);
            let clear = loop_reach + input.options.spacing.isolated_gap / 2.0;
            let mut taken: Vec<(f64, f64, f64, f64)> = active
                .iter()
                .map(|&i| (centers[i].0, centers[i].1, half(i).0, half(i).1))
                .collect();
            for &node in &isolated {
                let Some((x, y)) = input.seed.get(node).copied().flatten() else {
                    continue;
                };
                let (cx, cy) = (x + dx, y + dy);
                let (hw, hh) = half(node);
                let collides = taken.iter().any(|&(ox, oy, ow, oh)| {
                    (cx - ox).abs() < hw + ow + clear && (cy - oy).abs() < hh + oh + clear
                });
                if !collides {
                    slot_of[node] = Some((cx, cy));
                    taken.push((cx, cy, hw, hh));
                }
            }
            let bottom = taken
                .iter()
                .map(|&(_, y, _, h)| y + h)
                .fold(max_y, f64::max);
            grid_top = bottom + input.options.spacing.isolated_gap + cell_h / 2.0;
        }
        let mut cell = 0usize;
        for &node in &isolated {
            centers[node] = match slot_of[node] {
                Some(at) => at,
                None => {
                    let (r, c) = (cell / cols, cell % cols);
                    cell += 1;
                    (start + c as f64 * cell_w, grid_top + r as f64 * cell_h)
                }
            };
            // The pipeline never saw this node, so its loops are drawn here in output space.
            let Some(slot) = loops_of.get(node).copied().flatten() else {
                continue;
            };
            let drawn = selfloop::isolated_loops(
                centers[node],
                (input.nodes[node].width, input.nodes[node].height),
                input.nodes[node].shape,
                slot,
                orphan_side(slot),
                &input.options,
            );
            for one in drawn {
                loop_routes.insert(one.edge, one.points);
                if let Some(a) = one.label_anchor {
                    loop_label_anchors.insert(one.edge, a);
                }
            }
        }
        // An isolated node has no rank. Per-edge data carries over because the edge list is
        // unchanged.
        let mut ranks: Vec<Option<i32>> = vec![None; n_in];
        let mut orders: Vec<u32> = vec![0; n_in];
        for (new, &old) in active.iter().enumerate() {
            ranks[old] = sub_out.ranks.get(new).copied().flatten();
            orders[old] = sub_out.orders.get(new).copied().unwrap_or(0);
        }
        return Ok(LayeredOutput {
            centers,
            routes: sub_out.routes,
            label_anchors: sub_out.label_anchors,
            ranks,
            orders,
            loop_routes,
            loop_label_anchors,
            ..sub_out
        });
    }

    let o = &input.options;
    let mut st = LayeredState::new(input);
    st.lanes = lanes.to_vec();
    st.port_hints = hints.to_vec();

    Pipeline::layered().run(&mut st, o)?;

    Ok(output(&mut st, n_in, n_edges))
}

/// Reads the settled state into a [`LayeredOutput`] in output coordinates without moving
/// anything.
pub(super) fn output(st: &mut LayeredState, n_in: usize, n_edges: usize) -> LayeredOutput {
    // A `Vec<bool>` avoids a `BTreeSet` instantiation in the binary.
    let mut reversed_flags = vec![false; n_edges];
    for arc in st.arcs.iter().filter(|a| a.reversed) {
        if let Some(flag) = reversed_flags.get_mut(arc.edge) {
            *flag = true;
        }
    }

    let ranks: Vec<Option<i32>> = st.nodes[..n_in].iter().map(|nd| Some(nd.rank)).collect();
    let orders: Vec<u32> = (0..n_in)
        .map(|i| st.order_of.get(i).copied().unwrap_or(0) as u32)
        .collect();
    let spans: Vec<u32> = (0..n_edges)
        .map(|e| {
            st.chains
                .get(&e)
                .map_or(0, |chain| chain.len().saturating_sub(1) as u32)
        })
        .collect();
    let layer_count = st.layers.len() as u32;
    let dummy_count = st.len().saturating_sub(st.n_real) as u32;

    let axis = st.axis;
    let centers: Vec<(f64, f64)> = st.nodes[..n_in]
        .iter()
        .map(|nd| axis.to_output((nd.along, nd.cross)))
        .collect();

    let routes: Vec<Vec<(f64, f64)>> = (0..n_edges)
        .map(|arc| {
            let mut pts = st.routes[arc].clone();
            if pts.len() < 2 {
                return vec![];
            }
            if reversed_flags.get(arc).copied().unwrap_or(false) {
                pts.reverse();
            }
            pts.into_iter().map(|p| axis.to_output(p)).collect()
        })
        .collect();

    let lane_routes: Vec<Vec<Vec<(f64, f64)>>> = (0..n_edges)
        .map(|arc| {
            let flip = reversed_flags.get(arc).copied().unwrap_or(false);
            st.lane_routes
                .get(arc)
                .map(|lanes| {
                    lanes
                        .iter()
                        .map(|pts| {
                            let mut pts: Vec<(f64, f64)> =
                                pts.iter().map(|&p| axis.to_output(p)).collect();
                            if flip {
                                pts.reverse();
                            }
                            pts
                        })
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect();

    let label_anchors: Vec<Option<(f64, f64)>> = st
        .label_anchors
        .iter()
        .copied()
        .map(|a| a.map(|p| axis.to_output(p)))
        .collect();

    // Transposed in place because `collect` into a `BTreeMap` pulls in a stable sort per map type.
    let mut loop_routes = std::mem::take(&mut st.loop_routes);
    for pts in loop_routes.values_mut() {
        for p in pts.iter_mut() {
            *p = axis.to_output(*p);
        }
    }
    let mut loop_label_anchors = std::mem::take(&mut st.loop_label_anchors);
    for p in loop_label_anchors.values_mut() {
        *p = axis.to_output(*p);
    }

    LayeredOutput {
        centers,
        routes,
        label_anchors,
        ranks,
        orders,
        spans,
        reversed: reversed_flags,
        lanes: vec![None; n_edges],
        lane_routes,
        layer_count,
        dummy_count,
        loop_routes,
        loop_label_anchors,
        junctions: st.junctions.iter().map(|&p| axis.to_output(p)).collect(),
        swimlanes: st
            .swimlanes
            .iter()
            .map(|&(group, (a0, a1, c0, c1))| {
                let (p, q) = (axis.to_output((a0, c0)), axis.to_output((a1, c1)));
                (
                    group,
                    (p.0.min(q.0), p.1.min(q.1)),
                    (p.0.max(q.0), p.1.max(q.1)),
                )
            })
            .collect(),
    }
}

/// The border an orphan's loops bump out of. An orphan has no layer neighbours, so
/// [`crate::LoopSide::Auto`] uses the positive order side, which `Margins::cross_pos` reserves.
fn orphan_loop_side(o: &crate::options::LayoutOptions) -> crate::geometry::Side {
    use crate::geometry::Side;
    match o.self_loops.side {
        crate::options::LoopSide::CrossNegative => Side::Top,
        crate::options::LoopSide::AlongPositive => Side::Right,
        crate::options::LoopSide::CrossPositive | crate::options::LoopSide::Auto => Side::Bottom,
    }
}

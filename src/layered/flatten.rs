//! Chain flattening. A long edge gets the lane it wants by pushing other nodes aside.
//!
//! [`super::coord::straighten_long_edges`] keeps each waypoint inside the slot its neighbours
//! leave, so a node on the chain's lane forces a staircase. This phase pins the chain on one lane
//! (or one jog) and re-projects each touched layer with [`project_gaps_pinned`], which keeps the
//! order and crossings. `CoordTuning::flatten_room` caps the layer width one removed step may buy.

use crate::error::LayoutError;
use crate::geometry::monotone::project_gaps_pinned;
use crate::options::LayoutOptions;

use super::coord::{chain_jogs, gaps_hold, heaviest_first, LEVEL_PX};
use super::state::LayeredState;
use crate::sort::SortDyn;

pub fn flatten_chains(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    // A seeded drawing that keeps its structure leaves real nodes on their seeds.
    if o.tuning.coords.flatten_room == 0 || st.keeps_structure() {
        return Ok(());
    }
    st.reindex();
    let layers = std::mem::take(&mut st.layers);
    let chains = std::mem::take(&mut st.chains);

    // Lets a displaced waypoint's cost be charged to its chain. A waypoint has exactly one chain.
    let mut chain_of: Vec<Option<usize>> = vec![None; st.nodes.len()];
    for (&arc, chain) in &chains {
        for &v in &chain[1..chain.len().saturating_sub(1)] {
            if st.nodes[v].is_dummy() {
                chain_of[v] = Some(arc);
            }
        }
    }

    // Lets a move be charged for the straight arcs it breaks, e.g., a fork drawn as a block.
    let mut incident: Vec<Vec<usize>> = vec![vec![]; st.nodes.len()];
    for (i, arc) in st.arcs.iter().enumerate() {
        if arc.from == arc.to {
            continue;
        }
        incident[arc.from].push(i);
        incident[arc.to].push(i);
    }

    let mut arcs: Vec<(usize, f64)> = chains
        .iter()
        .filter(|(_, c)| c.len() >= 3 && c[1..c.len() - 1].iter().any(|&n| st.nodes[n].is_dummy()))
        .map(|(&a, _)| (a, st.weight.get(a).copied().unwrap_or(1.0)))
        .collect();
    arcs.sort_unstable_by_dyn(heaviest_first);

    let mut cross: Vec<f64> = st.nodes.iter().map(|n| n.cross).collect();
    for (arc, _) in &arcs {
        let chain = &chains[arc];
        if chain_jogs(&cross, chain).1 == 0 {
            continue;
        }
        let dummies: Vec<usize> = chain[1..chain.len() - 1]
            .iter()
            .copied()
            .filter(|&n| st.nodes[n].is_dummy())
            .collect();
        if dummies.is_empty() {
            continue;
        }
        if let Some(better) = best_move(
            st, o, &layers, &chains, &chain_of, &incident, &cross, *arc, chain, &dummies,
        ) {
            cross = better;
        }
    }

    st.layers = layers;
    for (node, &c) in st.nodes.iter_mut().zip(&cross) {
        node.cross = c;
    }
    st.chains = chains;
    Ok(())
}

/// The cheapest staircase-free plan for one chain that pays for itself, as the full `cross`
/// vector it produces.
#[allow(clippy::too_many_arguments)]
fn best_move(
    st: &LayeredState,
    o: &LayoutOptions,
    layers: &[Vec<usize>],
    chains: &std::collections::BTreeMap<usize, Vec<usize>>,
    chain_of: &[Option<usize>],
    incident: &[Vec<usize>],
    cross: &[f64],
    arc: usize,
    chain: &[usize],
    dummies: &[usize],
) -> Option<Vec<f64>> {
    let (head, tail) = (cross[chain[0]], cross[chain[chain.len() - 1]]);
    let before = score_of(st, layers, chains, chain_of, cross, dummies, arc, chain);

    let current: Vec<f64> = dummies.iter().map(|&d| cross[d]).collect();
    let mut best: Option<(f64, Vec<f64>)> = None;
    for plan in plans(head, tail, &current) {
        let Some(moved) = apply_plan(st, o, layers, cross, dummies, &plan) else {
            continue;
        };
        let after = score_of(st, layers, chains, chain_of, &moved, dummies, arc, chain);
        let steps_saved = before.steps as f64 - after.steps as f64;
        if steps_saved <= 0.0 {
            continue;
        }
        // The last term stops a long edge from buying its lane by breaking a fork/join block or a
        // node's alignment.
        let widened = after.width - before.width;
        let handed_on = (after.other_steps as f64 - before.other_steps as f64).max(0.0);
        let unaligned = unaligned_by(st, incident, cross, &moved) as f64;
        let room = f64::from(o.tuning.coords.flatten_room);
        let budget = room * steps_saved;
        let cost = widened + (handed_on + unaligned) * room;
        if cost > budget {
            continue;
        }
        // Cheapest wins, fewer jogs on a tie.
        let key = cost + after.jogs as f64 * LEVEL_PX;
        if best.as_ref().is_none_or(|(k, _)| key < *k) {
            best = Some((key, moved));
        }
    }
    best.map(|(_, moved)| moved)
}

/// One lane at `head`, one at `tail`, every one-jog plan between them, and one lane at each height
/// the chain already uses, where the layers already gave it room.
fn plans(head: f64, tail: f64, current: &[f64]) -> Vec<Vec<f64>> {
    let m = current.len();
    let mut out = vec![vec![head; m]];
    if (tail - head).abs() >= LEVEL_PX {
        out.push(vec![tail; m]);
        for split in 1..m {
            let mut p = vec![head; m];
            for slot in p.iter_mut().skip(split) {
                *slot = tail;
            }
            out.push(p);
        }
    }
    let mut treads: Vec<f64> = current.to_vec();
    treads.sort_unstable_by_dyn(f64::total_cmp);
    treads.dedup_by(|a, b| (*a - *b).abs() < LEVEL_PX);
    for tread in treads {
        if (tread - head).abs() >= LEVEL_PX && (tread - tail).abs() >= LEVEL_PX {
            out.push(vec![tread; m]);
        }
    }
    out
}

/// Returns `None` when a layer cannot hold the pin, since the projection would then put nodes
/// closer than [`LayeredState::sep`] allows.
fn apply_plan(
    st: &LayeredState,
    o: &LayoutOptions,
    layers: &[Vec<usize>],
    cross: &[f64],
    dummies: &[usize],
    plan: &[f64],
) -> Option<Vec<f64>> {
    let mut out = cross.to_vec();
    for (&d, &lane) in dummies.iter().zip(plan) {
        let layer = &layers[st.nodes[d].layer_clamped()];
        if layer.len() < 2 {
            out[d] = lane;
            continue;
        }
        let desired: Vec<f64> = layer.iter().map(|&v| out[v]).collect();
        let gap: Vec<f64> = (0..layer.len() - 1)
            .map(|i| st.sep(layer[i], layer[i + 1], o))
            .collect();
        let fixed: Vec<Option<f64>> = layer.iter().map(|&v| (v == d).then_some(lane)).collect();
        let placed = project_gaps_pinned(&desired, &gap, &fixed);
        if !gaps_hold(&placed, &gap) {
            return None;
        }
        for (i, &v) in layer.iter().enumerate() {
            out[v] = placed[i];
        }
    }
    Some(out)
}

/// Counts arcs straight in `before` and not in `after`. Only arcs of moved nodes can change.
fn unaligned_by(
    st: &LayeredState,
    incident: &[Vec<usize>],
    before: &[f64],
    after: &[f64],
) -> usize {
    let mut seen: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    for v in 0..before.len() {
        if (before[v] - after[v]).abs() < LEVEL_PX {
            continue;
        }
        for &a in &incident[v] {
            seen.insert(a);
        }
    }
    seen.into_iter()
        .filter(|&a| {
            let arc = &st.arcs[a];
            let was = (before[arc.from] - before[arc.to]).abs() < LEVEL_PX;
            let now = (after[arc.from] - after[arc.to]).abs() < LEVEL_PX;
            was && !now
        })
        .count()
}

struct Score {
    steps: usize,
    /// Breaks ties between plans of equal cost.
    jogs: usize,
    /// Steps on other chains whose waypoints this placement moved.
    other_steps: usize,
    /// Total width of the layers the chain passes through.
    width: f64,
}

/// `other_steps` counts only chains sharing a layer with this one. Walking every chain per
/// candidate would be quadratic on drawings with thousands of chains.
#[allow(clippy::too_many_arguments)]
fn score_of(
    st: &LayeredState,
    layers: &[Vec<usize>],
    chains: &std::collections::BTreeMap<usize, Vec<usize>>,
    chain_of: &[Option<usize>],
    cross: &[f64],
    dummies: &[usize],
    arc: usize,
    chain: &[usize],
) -> Score {
    let (jogs, steps) = chain_jogs(cross, chain);
    // A pinned re-projection can only move chains sharing a layer with this one.
    let mut neighbours: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    let mut width = 0.0;
    for &d in dummies {
        let rank = st.nodes[d].layer_clamped();
        let Some(layer) = layers.get(rank) else {
            continue;
        };
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for &v in layer {
            lo = lo.min(cross[v]);
            hi = hi.max(cross[v]);
            if let Some(other) = chain_of[v] {
                neighbours.insert(other);
            }
        }
        if hi >= lo {
            width += hi - lo;
        }
    }
    let mut other_steps = 0;
    for other in neighbours {
        if other == arc {
            continue;
        }
        if let Some(c) = chains.get(&other) {
            other_steps += chain_jogs(cross, c).1;
        }
    }
    Score {
        steps,
        jogs,
        other_steps,
        width,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::geometry::shape::NodeShape;
    use crate::layered::prepare::{LayeredInput, LayeredInputNode};
    use crate::options::Direction;

    fn input_node(width: f64, height: f64) -> LayeredInputNode {
        LayeredInputNode {
            width,
            height,
            shape: NodeShape::Box,
            constraint: None,
            category: None,
            clear_after: 0.0,
            clear_before: 0.0,
            port_sides: None,
            lane_key: None,
            along_before: 0.0,
            along_after: 0.0,
        }
    }

    #[test]
    fn every_plan_is_staircase_free_and_the_treads_are_offered() {
        let current = [0.0, 0.0, 40.0, 40.0];
        let out = plans(0.0, 90.0, &current);
        for plan in &out {
            let ids: Vec<usize> = (0..plan.len()).collect();
            assert_eq!(chain_jogs(plan, &ids).1, 0, "plan {plan:?} is a staircase");
        }
        assert!(out.iter().any(|p| p.iter().all(|&y| y == 40.0)), "{out:?}");
        assert!(out.iter().any(|p| p.iter().all(|&y| y == 0.0)));
        assert!(out.iter().any(|p| p.iter().all(|&y| y == 90.0)));
    }

    #[test]
    fn breaking_a_straight_arc_is_what_gets_charged() {
        let input = LayeredInput {
            nodes: vec![input_node(80.0, 36.0), input_node(80.0, 36.0)],
            edges: vec![(0, 1)],
            weights: vec![],
            thickness: vec![],
            edge_label_sizes: vec![],
            seed: vec![],
            pinned: vec![],
            self_loops: vec![],
            ranked_backwards: Vec::new(),
            options: LayoutOptions {
                direction: Direction::TopBottom,
                ..LayoutOptions::default()
            },
        };
        let mut st = LayeredState::new(&input);
        st.nodes[1].rank = 1;
        st.rebuild_layers();
        let incident = vec![vec![0usize], vec![0usize]];

        assert_eq!(
            unaligned_by(&st, &incident, &[10.0, 10.0], &[10.0, 90.0]),
            1
        );
        assert_eq!(
            unaligned_by(&st, &incident, &[10.0, 50.0], &[10.0, 90.0]),
            0
        );
        // Moved as one, so still straight.
        assert_eq!(
            unaligned_by(&st, &incident, &[10.0, 10.0], &[70.0, 70.0]),
            0
        );
    }
}

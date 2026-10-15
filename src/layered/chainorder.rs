//! Chain lanes, decided in order space. Local reorders let a long edge's waypoints share one
//! height wherever that costs no crossings.
//!
//! The straightener can only pick a lane inside the slots the order leaves, so a chain above one
//! node and below an overlapping one in the next layer needs a reorder. On a provisional
//! `PlaceCross` placement this pass walks the offending dummies toward the best lane one swap at a
//! time. A walk is kept only if every swap is crossing-neutral and the lane then fits one more
//! layer. Real nodes never pass each other, so the [`super::group`] lanes hold.

use super::coord::{
    admits, best_lane, chain_lane_target, covered, dummy_slot, lane_levels, place_cross,
};
use super::order::adjacent_swap_delta;
use super::state::LayeredState;
use crate::error::LayoutError;
use crate::geometry::EPS;
use crate::options::LayoutOptions;
use crate::sort::SortDyn;

pub fn order_chains(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    let rounds = o.tuning.order.chain_lane_rounds;
    if rounds == 0 || st.layers.len() < 3 {
        return Ok(());
    }
    let chains = std::mem::take(&mut st.chains);
    // Same order as the straightener (`coord::heaviest_first`), so it finds the lane decided here.
    let mut arcs: Vec<(usize, f64)> = chains
        .iter()
        .filter(|(_, c)| c.len() >= 3 && c[1..c.len() - 1].iter().any(|&n| st.nodes[n].is_dummy()))
        .map(|(&a, _)| (a, st.weight.get(a).copied().unwrap_or(1.0)))
        .collect();
    arcs.sort_unstable_by_dyn(super::coord::heaviest_first);
    if arcs.is_empty() {
        st.chains = chains;
        return Ok(());
    }
    // Work grows with chain span, so on big graphs the longest chains are left to the straightener.
    let n_dummies = st.len() - st.n_real;
    let max_span = if n_dummies > o.tuning.order.chain_lane_size_threshold {
        o.tuning.order.chain_lane_max_span
    } else {
        usize::MAX
    };

    let n = st.len();
    let (pred, succ) = st.adjacency();
    let saved: Vec<f64> = st.nodes.iter().map(|nd| nd.cross).collect();

    for _ in 0..rounds {
        // Place the current order as `PlaceCross` would, without the straightener.
        place_cross(st, o)?;
        st.reindex();
        let mut layers = std::mem::take(&mut st.layers);
        let mut settled: Vec<bool> = (0..n).map(|v| !st.nodes[v].is_dummy()).collect();
        let mut moved = false;
        for &(arc, _) in &arcs {
            let chain = &chains[&arc];
            let dummies: Vec<usize> = chain[1..chain.len() - 1]
                .iter()
                .copied()
                .filter(|&d| st.nodes[d].is_dummy())
                .collect();
            if dummies.is_empty() {
                continue;
            }
            let mut slots: Vec<(f64, f64)> = dummies
                .iter()
                .map(|&d| dummy_slot(st, &layers, &settled, d, o))
                .collect();
            let target = chain_lane_target(st, chain, &dummies, &slots);
            if dummies.len() <= max_span && !has_common_height(&slots) {
                let mut lane = best_lane(&slots, target);
                let mut n_admit = covered(&slots, lane);
                for (k, &d) in dummies.iter().enumerate() {
                    if admits(slots[k], lane) {
                        continue;
                    }
                    let up = lane > slots[k].1;
                    let mut walked: Vec<(usize, usize, usize, usize)> = vec![];
                    let mut outcome = None;
                    for _ in 0..o.tuning.order.chain_lane_max_steps {
                        let Some((i, j, nb)) = swap_partner(&layers, st, d, up) else {
                            break;
                        };
                        let (lower, upper) = if up { (d, nb) } else { (nb, d) };
                        let delta = adjacent_swap_delta(lower, upper, &pred, &st.order_of)
                            + adjacent_swap_delta(lower, upper, &succ, &st.order_of);
                        if delta > 0 {
                            break;
                        }
                        let l = st.nodes[d].layer();
                        layers[l].swap(i, j);
                        st.order_of[d] = j;
                        st.order_of[nb] = i;
                        walked.push((l, i, j, nb));
                        // Only this layer's slot changed. A neighbour chain that already had
                        // its turn is settled at its lane and bounds this slot like a real node.
                        let mut trial = slots.clone();
                        trial[k] = dummy_slot(st, &layers, &settled, d, o);
                        // One step can open a better lane than the one aimed at.
                        let trial_lane = best_lane(&trial, target);
                        let trial_admit = covered(&trial, trial_lane);
                        if has_common_height(&trial) || trial_admit > n_admit {
                            outcome = Some((trial, trial_lane, trial_admit));
                            break;
                        }
                    }
                    if let Some((trial, trial_lane, trial_admit)) = outcome {
                        slots = trial;
                        lane = trial_lane;
                        n_admit = trial_admit;
                        moved = true;
                        if has_common_height(&slots) {
                            break;
                        }
                    } else {
                        for &(l, i, j, nb) in walked.iter().rev() {
                            layers[l].swap(i, j);
                            st.order_of[d] = i;
                            st.order_of[nb] = j;
                        }
                    }
                }
            }
            // Settle the chain on its lane, so its lane bounds the later chains.
            let lanes = lane_levels(&slots, target);
            for (&d, (&(lo, hi), &lane)) in dummies.iter().zip(slots.iter().zip(&lanes)) {
                if lo <= hi {
                    st.nodes[d].cross = lane.clamp(lo, hi);
                }
                settled[d] = true;
            }
        }
        st.layers = layers;
        if !moved {
            break;
        }
    }

    for (node, &c) in st.nodes.iter_mut().zip(&saved) {
        node.cross = c;
    }
    st.chains = chains;
    st.reindex();
    Ok(())
}

fn has_common_height(slots: &[(f64, f64)]) -> bool {
    let lo = slots.iter().map(|s| s.0).fold(f64::NEG_INFINITY, f64::max);
    let hi = slots.iter().map(|s| s.1).fold(f64::INFINITY, f64::min);
    lo <= hi + EPS
}

/// Returns `(d's index, the neighbour's index, the neighbour)`, or `None` at the layer's end.
fn swap_partner(
    layers: &[Vec<usize>],
    st: &LayeredState,
    d: usize,
    up: bool,
) -> Option<(usize, usize, usize)> {
    let layer = layers.get(st.nodes[d].layer())?;
    let i = st.order_of[d];
    let j = if up { i + 1 } else { i.checked_sub(1)? };
    let nb = *layer.get(j)?;
    Some((i, j, nb))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::geometry::shape::NodeShape;
    use crate::layered::dummy::{build_edge_chains, insert_dummies};
    use crate::layered::order::count_crossings;
    use crate::layered::prepare::{LayeredInput, LayeredInputNode};
    use crate::options::LayoutOptions;

    fn node() -> LayeredInputNode {
        LayeredInputNode {
            width: 120.0,
            height: 52.0,
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

    /// Dummies are numbered from `ranks.len()` on, in edge order then layer order.
    fn state(
        ranks: &[i32],
        edges: &[(usize, usize)],
        weights: &[f64],
        layers: &[Vec<usize>],
    ) -> LayeredState {
        let input = LayeredInput {
            nodes: vec![node(); ranks.len()],
            edges: edges.to_vec(),
            weights: weights.to_vec(),
            thickness: vec![],
            edge_label_sizes: vec![],
            seed: vec![],
            pinned: vec![],
            self_loops: vec![],
            ranked_backwards: Vec::new(),
            options: LayoutOptions::layered_default(),
        };
        let mut st = LayeredState::new(&input);
        for (nd, &r) in st.nodes.iter_mut().zip(ranks) {
            nd.rank = r;
        }
        insert_dummies(&mut st).unwrap();
        st.rebuild_layers();
        build_edge_chains(&mut st).unwrap();
        assert_eq!(
            st.layers.iter().map(Vec::len).collect::<Vec<_>>(),
            layers.iter().map(Vec::len).collect::<Vec<_>>(),
            "the given order must list every node of every layer"
        );
        st.layers = layers.to_vec();
        st.reindex();
        st
    }

    fn pred_of(st: &LayeredState) -> Vec<Vec<usize>> {
        st.adjacency().0
    }

    /// Edges whose chain has no common height, judged on the same estimate the pass uses.
    fn forced_chains(st: &mut LayeredState, o: &LayoutOptions) -> Vec<usize> {
        let saved: Vec<f64> = st.nodes.iter().map(|nd| nd.cross).collect();
        place_cross(st, o).unwrap();
        let mut arcs: Vec<(usize, f64)> = st.chains.keys().map(|&a| (a, st.weight[a])).collect();
        arcs.sort_unstable_by_dyn(crate::layered::coord::heaviest_first);
        let mut settled: Vec<bool> = st.nodes.iter().map(|nd| !nd.is_dummy()).collect();
        let mut forced = vec![];
        for (arc, _) in arcs {
            let chain = st.chains[&arc].clone();
            let dummies: Vec<usize> = chain[1..chain.len() - 1].to_vec();
            if dummies.is_empty() {
                continue;
            }
            let slots: Vec<(f64, f64)> = dummies
                .iter()
                .map(|&d| dummy_slot(st, &st.layers, &settled, d, o))
                .collect();
            if !has_common_height(&slots) {
                forced.push(arc);
            }
            let target = chain_lane_target(st, &chain, &dummies, &slots);
            let lanes = lane_levels(&slots, target);
            for (&d, (&(lo, hi), &lane)) in dummies.iter().zip(slots.iter().zip(&lanes)) {
                if lo <= hi {
                    st.nodes[d].cross = lane.clamp(lo, hi);
                }
                settled[d] = true;
            }
        }
        for (nd, &c) in st.nodes.iter_mut().zip(&saved) {
            nd.cross = c;
        }
        forced
    }

    /// A (0 -> 3, heavier) and B (1 -> 2) cross between layers 1 and 2. A's waypoints are 4 and 5,
    /// B's are 6 and 7. A gets one lane, so B has to step round it.
    fn two_crossing_chains() -> LayeredState {
        state(
            &[0, 0, 3, 3],
            &[(0, 3), (1, 2)],
            &[2.0, 1.0],
            &[vec![0, 1], vec![4, 6], vec![7, 5], vec![2, 3]],
        )
    }

    /// Lifting B's layer-2 waypoint over A's moves the crossing to the gap between layers 2 and 3,
    /// where B drops to its target anyway. The swap is crossing-neutral and B runs level.
    #[test]
    fn a_forced_step_is_repaired_by_a_crossing_neutral_swap() {
        let o = LayoutOptions::layered_default();
        let mut st = two_crossing_chains();
        let pred = pred_of(&st);
        assert_eq!(count_crossings(&st.layers, &pred, &st.order_of), 1);
        assert_eq!(
            forced_chains(&mut st, &o),
            vec![1],
            "B must start forced to step"
        );

        order_chains(&mut st, &o).unwrap();

        assert_eq!(count_crossings(&st.layers, &pred, &st.order_of), 1);
        assert_eq!(
            st.layers,
            vec![vec![0, 1], vec![4, 6], vec![5, 7], vec![2, 3]]
        );
        assert!(forced_chains(&mut st, &o).is_empty());
        // The pass only reorders, so every coordinate is untouched.
        assert!(st.nodes.iter().all(|nd| nd.cross == 0.0));
    }

    /// Real nodes 4 and 5 sit between the waypoints in layers 1 and 2. Every swap that would give
    /// B a lane crosses both edges of one of them, so nothing moves.
    #[test]
    fn a_repair_that_would_cost_a_crossing_is_refused() {
        let o = LayoutOptions::layered_default();
        // Real nodes 0..=9. A = 0 -> 9 has waypoints 10 and 11, B = 3 -> 6 has 12 and 13.
        let st0 = state(
            &[0, 0, 0, 0, 1, 2, 3, 3, 3, 3],
            &[(0, 9), (3, 6), (1, 4), (2, 4), (5, 7), (5, 8)],
            &[2.0, 1.0, 1.0, 1.0, 1.0, 1.0],
            &[
                vec![0, 1, 2, 3],
                vec![10, 4, 12],
                vec![13, 5, 11],
                vec![6, 7, 8, 9],
            ],
        );
        let mut st = state(
            &[0, 0, 0, 0, 1, 2, 3, 3, 3, 3],
            &[(0, 9), (3, 6), (1, 4), (2, 4), (5, 7), (5, 8)],
            &[2.0, 1.0, 1.0, 1.0, 1.0, 1.0],
            &st0.layers,
        );
        let pred = pred_of(&st);
        assert_eq!(count_crossings(&st.layers, &pred, &st.order_of), 1);
        assert!(
            forced_chains(&mut st, &o).contains(&1),
            "B must start forced to step"
        );

        order_chains(&mut st, &o).unwrap();

        assert_eq!(st.layers, st0.layers);
        assert_eq!(count_crossings(&st.layers, &pred, &st.order_of), 1);
        assert!(forced_chains(&mut st, &o).contains(&1));
    }

    #[test]
    fn zero_rounds_is_a_no_op() {
        let mut o = LayoutOptions::layered_default();
        o.tuning.order.chain_lane_rounds = 0;
        let mut st = two_crossing_chains();
        let order_before = st.layers.clone();
        order_chains(&mut st, &o).unwrap();
        assert_eq!(st.layers, order_before);
    }
}

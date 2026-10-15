//! Channel pull-in. A long edge's channel is clamped by its worst layer and can end up far outside
//! the graph. This pass swaps such waypoints past a neighbouring real node, toward the line between
//! the edge's endpoints, when the swap adds no crossings.
//!
//! A seeded (warm-started) placement is skipped because it must keep the caller's coordinates.

use crate::error::LayoutError;
use crate::options::LayoutOptions;

use super::coord::{place_cross, straighten_long_edges};
use super::state::LayeredState;

/// Hops long-edge waypoints toward their endpoint line for up to `o.tuning.coords.pullin_rounds`
/// rounds, stopping early when a round changes nothing. Does nothing for a seeded placement.
pub(crate) fn channel_pullin(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    if st.has_seeds() || o.tuning.coords.pullin_rounds == 0 {
        return Ok(());
    }
    for _ in 0..o.tuning.coords.pullin_rounds {
        let n_all = st.len();
        let mut pos_in_layer = vec![0usize; n_all];
        for l in &st.layers {
            for (i, &v) in l.iter().enumerate() {
                pos_in_layer[v] = i;
            }
        }
        let mut pred: Vec<Vec<usize>> = vec![vec![]; n_all];
        let mut succ: Vec<Vec<usize>> = vec![vec![]; n_all];
        for arc in &st.arcs {
            let (a, bb) = (arc.from, arc.to);
            if a == bb {
                continue;
            }
            let (up, dn) = if st.nodes[a].rank < st.nodes[bb].rank {
                (a, bb)
            } else {
                (bb, a)
            };
            pred[dn].push(up);
            succ[up].push(dn);
        }
        // Crossing change if u (lower order) and v swap, over one adjacency direction.
        let delta_one = |u: usize, v: usize, nbv: &[Vec<usize>], pos: &[usize]| -> i32 {
            let mut d = 0i32;
            for &nu in &nbv[u] {
                for &nv in &nbv[v] {
                    if pos[nu] > pos[nv] {
                        d -= 1;
                    } else if pos[nu] < pos[nv] {
                        d += 1;
                    }
                }
            }
            d
        };
        let mut changed = false;
        let chains = std::mem::take(&mut st.chains);
        let mut layers = std::mem::take(&mut st.layers);
        for (_, chain) in chains.iter().filter(|(_, c)| c.len() >= 3) {
            let lane = (st.nodes[chain[0]].cross + st.nodes[chain[chain.len() - 1]].cross) / 2.0;
            for &d in &chain[1..chain.len() - 1] {
                if !st.nodes[d].is_dummy() {
                    continue;
                }
                let l = st.nodes[d].layer();
                let p = pos_in_layer[d];
                let toward = lane - st.nodes[d].cross;
                if toward.abs() < 1.0 {
                    continue;
                }
                let np = if toward < 0.0 {
                    p.checked_sub(1)
                } else {
                    Some(p + 1)
                };
                let Some(np) = np.filter(|&np| np < layers[l].len()) else {
                    continue;
                };
                let nb = layers[l][np];
                // Dummy-dummy order encodes channel bundling, so only real nodes are hopped.
                if st.nodes[nb].is_dummy() {
                    continue;
                }
                if (st.nodes[nb].cross - st.nodes[d].cross).signum() != toward.signum() {
                    continue;
                }
                let (u, v) = if np < p { (nb, d) } else { (d, nb) };
                if delta_one(u, v, &pred, &pos_in_layer) + delta_one(u, v, &succ, &pos_in_layer) > 0
                {
                    continue;
                }
                layers[l].swap(p, np);
                pos_in_layer[d] = np;
                pos_in_layer[nb] = p;
                changed = true;
            }
        }
        st.chains = chains;
        st.layers = layers;
        st.reindex();
        if !changed {
            break;
        }
        place_cross(st, o)?;
        straighten_long_edges(st, o)?;
    }
    Ok(())
}

//! Order-preserving cross-axis compaction for dense hub-and-spoke graphs. Each layer is packed to
//! its minimum width and shifted whole to its weighted-median edge alignment, then the real nodes
//! slide toward their neighbour median inside the packed envelope.
//!
//! Per-layer packing bends long edges wherever two layer offsets differ. So a tied offset goes to
//! the one with more level hops ([`level_hops`]), and waypoints go to [`straighten_long_edges`] as
//! whole chains. Width and crossings do not change.

use super::coord::{straighten_long_edges, LEVEL_PX};
use super::state::LayeredState;
use crate::error::LayoutError;
use crate::geometry::stats::{pair_order, weighted_median};
use crate::geometry::EPS;
use crate::options::LayoutOptions;
use crate::sort::SortDyn;

/// Share of the layer objective within which two offsets count as tied. It only absorbs float
/// noise.
const RELATIVE_TIE: f64 = 1e-3;

pub fn priority_compact(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    let n = st.len();
    st.reindex();
    let layers = std::mem::take(&mut st.layers);
    let mut lyr = vec![0usize; n];
    for (i, l) in layers.iter().enumerate() {
        for &v in l {
            lyr[v] = i;
        }
    }
    let mut nbr: Vec<Vec<(usize, f64)>> = vec![vec![]; n];
    for arc in &st.arcs {
        let (a, b) = (arc.from, arc.to);
        if a == b {
            continue;
        }
        let w = st.weight.get(arc.edge).copied().unwrap_or(1.0);
        nbr[a].push((b, w));
        nbr[b].push((a, w));
    }

    // 1. Pack each layer to minimum width.
    let mut x = vec![0.0f64; n];
    for layer in &layers {
        let mut acc = 0.0;
        for (i, &v) in layer.iter().enumerate() {
            if i > 0 {
                acc += st.sep(layer[i - 1], v, o);
            }
            x[v] = acc;
        }
    }

    // 2. Shift each whole layer to the weighted median alignment of its edges.
    let mut offset = vec![0.0f64; layers.len()];
    let mut samples: Vec<(f64, f64)> = vec![];
    for round in 0..o.tuning.compaction.offset_rounds {
        let order: Vec<usize> = if round % 2 == 0 {
            (0..layers.len()).collect()
        } else {
            (0..layers.len()).rev().collect()
        };
        let mut changed = false;
        for &li in &order {
            samples.clear();
            for &v in &layers[li] {
                for &(u, w) in &nbr[v] {
                    samples.push((x[u] + offset[lyr[u]] - x[v], w));
                }
            }
            if samples.is_empty() {
                continue;
            }
            let no = level_offset(&mut samples);
            if (no - offset[li]).abs() > 0.5 {
                offset[li] = no;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    for v in 0..n {
        st.nodes[v].cross = x[v] + offset[lyr[v]];
    }

    // 3a. Chains go first, so real nodes refine against the lanes long edges took.
    st.layers = layers;
    straighten_long_edges(st, o)?;
    let layers = std::mem::take(&mut st.layers);

    // 3b. Clamping to same-layer separations keeps the packed width. Waypoints are skipped, since
    // one pulled to its own neighbour median bends its chain again.
    let prio: Vec<u64> = (0..n).map(|v| nbr[v].len() as u64).collect();
    let mut cy: Vec<f64> = st.nodes.iter().map(|nd| nd.cross).collect();
    for round in 0..o.tuning.compaction.refine_rounds {
        let order: Vec<usize> = if round % 2 == 0 {
            (0..layers.len()).collect()
        } else {
            (0..layers.len()).rev().collect()
        };
        for &li in &order {
            let layer = &layers[li];
            let mut idxs: Vec<usize> = (0..layer.len()).collect();
            idxs.sort_unstable_by_dyn(|&i, &j| prio[layer[j]].cmp(&prio[layer[i]]).then(i.cmp(&j)));
            for &pos in &idxs {
                let v = layer[pos];
                if nbr[v].is_empty() || st.nodes[v].is_dummy() {
                    continue;
                }
                let lo = if pos > 0 {
                    cy[layer[pos - 1]] + st.sep(layer[pos - 1], v, o)
                } else {
                    f64::NEG_INFINITY
                };
                let hi = if pos + 1 < layer.len() {
                    cy[layer[pos + 1]] - st.sep(v, layer[pos + 1], o)
                } else {
                    f64::INFINITY
                };
                if lo > hi {
                    continue;
                }
                let mut samples: Vec<(f64, f64)> =
                    nbr[v].iter().map(|&(u, w)| (cy[u], w)).collect();
                cy[v] = weighted_median(&mut samples).clamp(lo, hi);
            }
        }
    }
    for (node, &c) in st.nodes.iter_mut().zip(cy.iter()) {
        node.cross = c;
    }
    st.layers = layers;
    Ok(())
}

/// The weighted-median offset for one layer, ties broken toward the most level edge hops. The
/// objective is often flat over an interval, where [`weighted_median`] returns the lower end.
fn level_offset(samples: &mut [(f64, f64)]) -> f64 {
    let best = weighted_median(samples);
    if samples.len() < 2 {
        return best;
    }
    // `weighted_median` sorted the samples, which the one-pass sweep relies on.
    debug_assert!(samples.windows(2).all(|w| pair_order(&w[0], &w[1]).is_le()));
    let cost = |offset: f64| -> f64 {
        samples
            .iter()
            .map(|&(v, w)| w * (v - offset).abs())
            .sum::<f64>()
    };
    let floor = cost(best);
    // Relative, since an absolute epsilon would mean different things on small and large drawings.
    let tolerance = floor.abs() * RELATIVE_TIE + EPS;
    let mut pick = (level_hops(samples, best), -(best.abs()), best);
    for i in 0..samples.len() {
        let candidate = samples[i].0;
        if (candidate - best).abs() < LEVEL_PX || cost(candidate) > floor + tolerance {
            continue;
        }
        // More edges level, then the offset nearer zero (the smallest move), then the value itself.
        let key = (
            level_hops(samples, candidate),
            -(candidate.abs()),
            candidate,
        );
        if key.0 > pick.0 || (key.0 == pick.0 && key.1 > pick.1) {
            pick = key;
        }
    }
    pick.2
}

/// Counts the samples within `LEVEL_PX` of `offset`. Each sample is one hop.
fn level_hops(samples: &[(f64, f64)], offset: f64) -> usize {
    samples
        .iter()
        .filter(|&&(v, _)| (v - offset).abs() < LEVEL_PX)
        .count()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn level_offset_breaks_a_tie_toward_the_level_hops() {
        // Every offset in [0, 100] costs 200 and both ends level two hops. The smaller move wins.
        let mut s = vec![(0.0, 1.0), (0.0, 1.0), (100.0, 1.0), (100.0, 1.0)];
        assert_eq!(level_offset(&mut s), 0.0);
        // Three at 100 against one at 0 is no tie, so the median wins.
        let mut s = vec![(0.0, 1.0), (100.0, 1.0), (100.0, 1.0), (100.0, 1.0)];
        assert_eq!(level_offset(&mut s), 100.0);
    }

    #[test]
    fn level_offset_never_pays_for_the_tie_break() {
        // 0 is the weighted median (weight 3 of 5) at cost 20. Offset 10 levels two hops at 30.
        let mut s = vec![(0.0, 3.0), (10.0, 1.0), (10.0, 1.0)];
        assert_eq!(level_offset(&mut s), 0.0);
    }

    #[test]
    fn level_offset_handles_the_single_sample() {
        assert_eq!(level_offset(&mut [(42.0, 1.0)]), 42.0);
    }
}

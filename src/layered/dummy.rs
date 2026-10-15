//! Dummy insertion. An edge spanning more than one layer becomes a chain of zero-size waypoints,
//! one per intermediate layer, so every arc the later phases see connects adjacent layers.

use std::collections::BTreeMap;

use super::state::{LArc, LayeredState};
use crate::error::LayoutError;
use crate::sort::SortDyn;

pub fn insert_dummies(st: &mut LayeredState) -> Result<(), LayoutError> {
    let orig_arcs = std::mem::take(&mut st.arcs);

    for arc in orig_arcs {
        let (from, to) = (arc.from, arc.to);
        let span = st.nodes[to].rank - st.nodes[from].rank;
        if span <= 1 {
            st.arcs.push(arc);
        } else {
            let mut prev = from;
            for k in 1..span {
                let d = st.add_dummy(st.nodes[from].rank + k);
                st.arcs.push(LArc {
                    from: prev,
                    to: d,
                    ..arc
                });
                prev = d;
            }
            st.arcs.push(LArc {
                from: prev,
                to,
                ..arc
            });
        }
    }
    Ok(())
}

/// A dummy is a point kept `dummy_gap` from node borders, so a stroke wider than the gap would be
/// drawn through the node. The excess width becomes the dummy's cross width.
pub fn reserve_stroke_width(st: &mut LayeredState, dummy_gap: f64) {
    for i in 0..st.arcs.len() {
        let arc = st.arcs[i];
        let t = st.thickness.get(arc.edge).copied().unwrap_or(0.0);
        for v in [arc.from, arc.to] {
            if st.nodes[v].is_dummy() {
                st.nodes[v].w_cross = st.nodes[v].w_cross.max(t - dummy_gap);
            }
        }
    }
}

/// Records each edge's node chain in layer order, source and target included.
pub fn build_edge_chains(st: &mut LayeredState) -> Result<(), LayoutError> {
    let mut segs: BTreeMap<usize, Vec<(i32, usize, usize)>> = BTreeMap::new();
    for arc in &st.arcs {
        segs.entry(arc.edge)
            .or_default()
            .push((st.nodes[arc.from].rank, arc.from, arc.to));
    }
    let mut result: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (arc_idx, mut chain) in segs {
        // One segment per layer gap makes the triple unique, so an unstable sort is deterministic.
        chain.sort_unstable_by_dyn(Ord::cmp);
        let mut nodes: Vec<usize> = vec![];
        for (i, &(_, from, to)) in chain.iter().enumerate() {
            if i == 0 {
                nodes.push(from);
            }
            nodes.push(to);
        }
        result.insert(arc_idx, nodes);
    }
    st.chains = result;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::geometry::shape::NodeShape;
    use crate::layered::prepare::{LayeredInput, LayeredInputNode};
    use crate::options::LayoutOptions;

    /// The 50 px stroke exceeds the 30 px gap by 20 px. The 2 px stroke stays within it.
    #[test]
    fn only_a_stroke_wider_than_the_gap_widens_its_dummy() {
        let node = LayeredInputNode {
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
        };
        let input = LayeredInput {
            nodes: vec![node; 4],
            edges: vec![(0, 1), (2, 3)],
            weights: vec![],
            thickness: vec![2.0, 50.0],
            edge_label_sizes: vec![],
            seed: vec![],
            pinned: vec![],
            self_loops: vec![],
            ranked_backwards: Vec::new(),
            options: LayoutOptions::default(),
        };
        let mut st = LayeredState::new(&input);
        for (v, rank) in [(0, 0), (1, 2), (2, 0), (3, 2)] {
            st.nodes[v].rank = rank;
        }
        insert_dummies(&mut st).unwrap();
        reserve_stroke_width(&mut st, 30.0);
        let widths: Vec<f64> = st.nodes[4..].iter().map(|n| n.w_cross).collect();
        assert_eq!(widths, vec![0.0, 20.0]);
    }
}

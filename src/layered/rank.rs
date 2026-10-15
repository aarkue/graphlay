//! Layering. Longest-path ranking gives a feasible assignment, then network simplex minimises the
//! total weighted edge span, which also minimises the number of dummy nodes.

use super::simplex::{solve, Budget, Constraint};
use super::state::LayerConstraint;
use super::state::LayeredState;
use crate::error::LayoutError;
use crate::sort::SortDyn;

/// Longest-path ranking, where every edge spans at least one layer.
fn longest_path(st: &LayeredState) -> Vec<i32> {
    let n = st.len();
    let mut in_deg = vec![0i32; n];
    let mut adj: Vec<Vec<usize>> = vec![vec![]; n];
    for arc in &st.arcs {
        let (from, to) = (arc.from, arc.to);
        if from == to {
            continue;
        }
        in_deg[to] += 1;
        adj[from].push(to);
    }
    let mut layer = vec![-1i32; n];
    let mut queue: Vec<usize> = (0..n).filter(|&i| in_deg[i] == 0).collect();
    for &s in &queue {
        layer[s] = 0;
    }
    let mut head = 0;
    while head < queue.len() {
        let u = queue[head];
        head += 1;
        for &v in &adj[u] {
            if layer[u] + 1 > layer[v] {
                layer[v] = layer[u] + 1;
            }
            in_deg[v] -= 1;
            if in_deg[v] == 0 {
                queue.push(v);
            }
        }
    }
    for l in layer.iter_mut() {
        if *l < 0 {
            *l = 0;
        }
    }
    layer
}

/// Assigns every internal node its layer in [`LNode::rank`], normalised so the lowest is 0.
///
/// [`LNode::rank`]: super::state::LNode::rank
pub fn assign_ranks(st: &mut LayeredState) -> Result<(), LayoutError> {
    orient_by_fixed_layers(st);
    let mut layer = longest_path(st);
    network_simplex(st, &mut layer);

    let constraints: Vec<Option<LayerConstraint>> =
        st.nodes.iter().map(|nd| nd.constraint).collect();
    if constraints.iter().any(|c| c.is_some()) {
        let max_layer = *layer.iter().max().unwrap_or(&0);
        for (i, c) in constraints.iter().enumerate().take(st.len()) {
            match c {
                Some(LayerConstraint::First) => layer[i] = 0,
                Some(LayerConstraint::Last) => layer[i] = max_layer,
                Some(LayerConstraint::Fixed(_)) | None => {}
            }
        }
    }
    let min = *layer.iter().min().unwrap_or(&0);
    for l in layer.iter_mut() {
        *l -= min;
    }
    for (node, &r) in st.nodes.iter_mut().zip(layer.iter()) {
        node.rank = r;
    }
    Ok(())
}

/// Network-simplex ranking (Gansner et al.) on [`super::simplex::solve`], the same solver the
/// coordinate phase uses. `rank` must hold a feasible ranking on entry and is left unchanged if the
/// solver runs out of budget.
fn network_simplex(st: &LayeredState, rank: &mut [i32]) {
    let fixed = fixed_layers(st);
    if !fixed.is_empty() && solve_ranks(st, rank, &fixed) {
        return;
    }
    // Fixed layers the edges contradict are dropped.
    solve_ranks(st, rank, &[]);
}

/// The nodes with a [`LayerConstraint::Fixed`] layer, as `(node, k)` sorted by `k`.
fn fixed_layers(st: &LayeredState) -> Vec<(usize, i32)> {
    let mut fixed: Vec<(usize, i32)> = st
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(i, nd)| match nd.constraint {
            Some(LayerConstraint::Fixed(k)) => Some((i, k)),
            _ => None,
        })
        .collect();
    fixed.sort_unstable_by_key_dyn(|&(i, k)| (k, i));
    fixed
}

/// Reverses every arc that runs from a later fixed layer to an earlier one.
fn orient_by_fixed_layers(st: &mut LayeredState) {
    let mut k_of: Vec<Option<i32>> = vec![None; st.len()];
    for (i, k) in fixed_layers(st) {
        k_of[i] = Some(k);
    }
    for arc in st.arcs.iter_mut() {
        if let (Some(a), Some(b)) = (k_of[arc.from], k_of[arc.to]) {
            if a > b {
                std::mem::swap(&mut arc.from, &mut arc.to);
                arc.reversed = !arc.reversed;
            }
        }
    }
}

/// Minimises the weighted span with `fixed` layers held. All nodes of one fixed layer share a
/// variable. Returns `false` and leaves `rank` untouched when there is no solution.
fn solve_ranks(st: &LayeredState, rank: &mut [i32], fixed: &[(usize, i32)]) -> bool {
    let n = st.len();
    let mut var: Vec<usize> = (0..n).collect();
    let mut layers: Vec<(usize, i32)> = Vec::new();
    for &(i, k) in fixed {
        match layers.last() {
            Some(&(first, lk)) if lk == k => var[i] = first,
            _ => layers.push((i, k)),
        }
    }
    let mut constraints: Vec<Constraint> = st
        .arcs
        .iter()
        .filter(|a| a.from != a.to && var[a.from] != var[a.to])
        .map(|a| Constraint {
            tail: var[a.from],
            head: var[a.to],
            min_len: 1,
            weight: st.weight.get(a.edge).copied().unwrap_or(1.0),
        })
        .collect();
    for w in layers.windows(2) {
        constraints.push(Constraint {
            tail: w[0].0,
            head: w[1].0,
            min_len: i64::from(w[1].1) - i64::from(w[0].1),
            weight: 0.0,
        });
    }
    if constraints.is_empty() {
        return true;
    }
    // Generous because ranking is cheap. The cap only bounds pathological inputs.
    let size = n.saturating_add(constraints.len());
    let budget = Budget {
        max_pivots: 64usize.saturating_mul(size),
        max_tree_work: 64usize.saturating_mul(size),
    };
    match solve(n, &constraints, budget) {
        Some(x) => {
            for (r, &v) in rank.iter_mut().zip(&var) {
                *r = x[v] as i32;
            }
            true
        }
        None => false,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::geometry::shape::NodeShape;
    use crate::layered::prepare::{LayeredInput, LayeredInputNode};
    use crate::options::LayoutOptions;

    fn state(n: usize, edges: &[(usize, usize)]) -> LayeredState {
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
        LayeredState::new(&LayeredInput {
            nodes: vec![node; n],
            edges: edges.to_vec(),
            weights: vec![],
            thickness: vec![],
            edge_label_sizes: vec![],
            seed: vec![],
            pinned: vec![],
            self_loops: vec![],
            ranked_backwards: Vec::new(),
            options: LayoutOptions::default(),
        })
    }

    /// A second component keeps its own layering when the first one makes the simplex pivot.
    #[test]
    fn a_second_component_is_layered_too() {
        let edges = [(0, 2), (2, 3), (2, 3), (1, 3), (0, 1), (4, 5), (5, 6)];
        let mut st = state(7, &edges);
        assign_ranks(&mut st).unwrap();
        let rank: Vec<i32> = st.nodes.iter().map(|n| n.rank).collect();
        assert_eq!(&rank[4..], &[0, 1, 2]);
        for &(u, v) in &edges {
            assert!(rank[v] > rank[u], "{u} -> {v} in {rank:?}");
        }
    }
}

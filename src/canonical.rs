//! Renumbers a graph by its node keys ([`crate::Node::key`]) before layout, so the drawing does
//! not depend on input order even though every phase breaks ties by index. Only edges identical
//! in every attribute keep their input order, and swapping those does not change the picture.

use std::cmp::Ordering;

use crate::graph::{EdgeId, Graph, NodeId};
use crate::layout::Layout;
use crate::sort::SortDyn;

/// A graph renumbered into canonical order, and how to map its layout back.
pub(crate) struct Canonical {
    pub graph: Graph,
    /// `node_at[k]` is the caller's index of canonical node `k`.
    node_at: Vec<usize>,
    /// `edge_at[k]` is the caller's index of canonical edge `k`.
    edge_at: Vec<usize>,
}

/// `g` renumbered by its node keys, or `None` when a key is missing or duplicated.
pub(crate) fn canonical(g: &Graph) -> Option<Canonical> {
    let keys: Vec<&str> = g
        .nodes
        .iter()
        .map(|n| n.key.as_deref())
        .collect::<Option<_>>()?;
    let mut node_at: Vec<usize> = (0..g.nodes.len()).collect();
    node_at.sort_unstable_by_dyn(|&a, &b| keys[a].cmp(keys[b]));
    if node_at.windows(2).any(|w| keys[w[0]] == keys[w[1]]) {
        return None;
    }
    let mut new_of = vec![0u32; g.nodes.len()];
    for (k, &old) in node_at.iter().enumerate() {
        new_of[old] = k as u32;
    }
    let endpoint = |id: NodeId| keys.get(id.index()).copied().unwrap_or("");
    let mut edge_at: Vec<usize> = (0..g.edges.len()).collect();
    edge_at.sort_by_dyn(|&a, &b| {
        let (x, y) = (&g.edges[a], &g.edges[b]);
        endpoint(x.source)
            .cmp(endpoint(y.source))
            .then_with(|| endpoint(x.target).cmp(endpoint(y.target)))
            .then_with(|| x.weight.total_cmp(&y.weight))
            .then_with(|| x.thickness.total_cmp(&y.thickness))
            .then_with(|| x.reverse_for_ranking.cmp(&y.reverse_for_ranking))
            .then_with(|| x.group.map(|g| g.0).cmp(&y.group.map(|g| g.0)))
            .then_with(|| size_order(x.label_size, y.label_size))
            .then_with(|| hint_order(x.source_port, y.source_port))
            .then_with(|| hint_order(x.target_port, y.target_port))
    });
    let renumber = |id: NodeId| NodeId(new_of.get(id.index()).copied().unwrap_or(id.0));
    let nodes = node_at
        .iter()
        .map(|&old| {
            let mut n = g.nodes[old].clone();
            n.parent = n.parent.map(renumber);
            n
        })
        .collect();
    let edges = edge_at
        .iter()
        .map(|&old| {
            let mut e = g.edges[old].clone();
            e.source = renumber(e.source);
            e.target = renumber(e.target);
            e
        })
        .collect();
    Some(Canonical {
        graph: Graph { nodes, edges },
        node_at,
        edge_at,
    })
}

fn size_order(a: Option<crate::Size>, b: Option<crate::Size>) -> Ordering {
    let key = |s: Option<crate::Size>| s.map(|s| (s.width, s.height));
    match (key(a), key(b)) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some((aw, ah)), Some((bw, bh))) => aw.total_cmp(&bw).then(ah.total_cmp(&bh)),
    }
}

/// Total order on port hints: none first, then by side, a bare side before its offsets.
fn hint_order(a: Option<crate::PortHint>, b: Option<crate::PortHint>) -> Ordering {
    let key = |h: Option<crate::PortHint>| {
        h.map(|h| (h.side().index(), h.offset().unwrap_or(f64::NEG_INFINITY)))
    };
    match (key(a), key(b)) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some((sa, ta)), Some((sb, tb))) => sa.cmp(&sb).then(ta.total_cmp(&tb)),
    }
}

/// Maps `laid`, a layout of `c.graph`, back to the caller's numbering.
pub(crate) fn restore(c: &Canonical, mut laid: Layout) -> Layout {
    let mut nodes = vec![None; laid.nodes.len()];
    for (k, p) in laid.nodes.drain(..).enumerate() {
        if let Some(&old) = c.node_at.get(k) {
            nodes[old] = Some(p);
        }
    }
    let mut edges = vec![None; laid.edges.len()];
    for (k, e) in laid.edges.drain(..).enumerate() {
        if let Some(&old) = c.edge_at.get(k) {
            edges[old] = Some(e);
        }
    }
    laid.nodes = nodes.into_iter().flatten().collect();
    laid.edges = edges.into_iter().flatten().collect();
    for id in &mut laid.reversed {
        if let Some(&old) = c.edge_at.get(id.index()) {
            *id = EdgeId(old as u32);
        }
    }
    laid.reversed.sort_unstable_by_dyn(|a, b| a.0.cmp(&b.0));
    laid
}

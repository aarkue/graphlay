//! Phase `AlignSplits`: draws each fork/join block in the shape [`crate::options::BranchStyle`]
//! asks for.
//!
//! Brandes-Kopf aligns a node with one of its median neighbours, so which branch of a fork runs
//! straight is close to arbitrary. This pass picks it on purpose, moving nodes only on the order
//! axis through [`project_gaps_pinned`], so crossings stay the same. A block is committed only when
//! it aligns more blocks than it breaks, by the [`crate::splits::is_aligned`] the metric uses. It
//! runs after [`crate::layered::pipeline::Phase::ChannelPullIn`], which would pull a straightened
//! chain's waypoints back off the line.

use std::collections::{BTreeMap, BTreeSet};

use crate::error::LayoutError;
use crate::geometry::monotone::project_gaps_pinned;
use crate::geometry::{EPS, SLACK};
use crate::options::{BranchStyle, LayoutOptions};
use crate::splits::{self, SplitPair, MAX_DEPTH};

use super::coord::gaps_hold;
use super::state::LayeredState;
use crate::sort::SortDyn;

const NONE: usize = usize::MAX;

/// Farthest a block may move a node from its assigned position, in node gaps. A longer drag turns
/// short straight edges into long diagonals, which costs more than the alignment gains.
const MAX_SHIFT_GAPS: f64 = 24.0;

/// Extra cross-axis edge offset a move may add, in node gaps per block gained. Zero would only
/// accept blocks that were already nearly aligned.
const SLOPE_SLACK_GAPS: f64 = 6.0;

/// Pins one branch of every fork/join block onto a straight line, or both onto mirrored lines,
/// per [`crate::options::BranchStyle`]. Does nothing for [`BranchStyle::Free`].
pub fn align_splits(st: &mut LayeredState, o: &LayoutOptions) -> Result<(), LayoutError> {
    let style = o.coords.branch_style;
    if style == BranchStyle::Free
        || st.n_real == 0
        || o.tuning.coords.branch_rounds == 0
        || st.keeps_structure()
    {
        return Ok(());
    }
    // A main-path drawing already placed its blocks (see `block_pairs`), and pinning them layer by
    // layer would bend the branch bands.
    if style == BranchStyle::StraightMain && super::balance::frames_the_drawing(st, o) {
        return Ok(());
    }
    st.reindex();
    let blocks = Blocks::build(st);
    if blocks.pairs.is_empty() {
        return Ok(());
    }
    // Each layer is re-projected from `base` plus the `held` lines, so a node pushed aside for one
    // block returns when there is room again and the drawing does not widen with every block.
    let base: Vec<f64> = st.nodes.iter().map(|nd| nd.cross).collect();
    let mut cross = base.clone();
    let mut held: Vec<Option<f64>> = vec![None; st.len()];
    // Visited marks stamped with the attempt number, so they never need clearing.
    let mut seen: Vec<u32> = vec![0; blocks.pairs.len()];
    let mut attempt = 0u32;
    for _ in 0..o.tuning.coords.branch_rounds {
        let mut committed = 0usize;
        for &k in &blocks.order {
            let pair = blocks.pairs[k];
            if satisfied(st, &cross, &pair, style) {
                continue;
            }
            // Candidate lines come cheapest first, and the first to pass every guard wins.
            for targets in blocks.targets(st, o, &cross, &pair, style) {
                if targets
                    .iter()
                    .any(|&(v, y)| held[v].is_some_and(|h| (h - y).abs() > EPS))
                {
                    continue;
                }
                let Some(staged) = stage(st, o, &base, &held, &cross, &targets) else {
                    continue;
                };
                let limit = MAX_SHIFT_GAPS * o.spacing.node_gap;
                if staged.iter().any(|&(v, y)| (y - base[v]).abs() > limit) {
                    continue;
                }
                attempt += 1;
                let judge = Judge {
                    st,
                    blocks: &blocks,
                    o,
                    style,
                };
                if commit(&judge, &mut cross, &mut seen, attempt, &staged) {
                    for &(v, y) in &targets {
                        held[v] = Some(y);
                    }
                    committed += 1;
                    break;
                }
            }
        }
        if committed == 0 {
            break;
        }
    }
    for (nd, &y) in st.nodes.iter_mut().zip(cross.iter()) {
        nd.cross = y;
    }
    Ok(())
}

/// The `(fork, join)` pair of every block, deduplicated. These are the blocks the metric finds.
pub(super) fn block_pairs(st: &LayeredState) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Blocks::build(st)
        .pairs
        .iter()
        .map(|p| (p.fork, p.join))
        .collect();
    out.sort_unstable_by_dyn(Ord::cmp);
    out.dedup();
    out
}

/// Whether the block is already drawn the way `style` asks.
fn satisfied(st: &LayeredState, cross: &[f64], pair: &SplitPair, style: BranchStyle) -> bool {
    let pos = |i: usize| (st.nodes[i].along, cross[i]);
    match style {
        BranchStyle::Free => true,
        BranchStyle::StraightMain => splits::is_straight_main(pos, pair),
        BranchStyle::Symmetric => splits::is_mirrored(pos, pair),
    }
}

/// Whether the metric counts the block as aligned. No move may lose one.
fn aligned(st: &LayeredState, cross: &[f64], pair: &SplitPair) -> bool {
    splits::is_aligned(|i| (st.nodes[i].along, cross[i]), pair)
}

/// The fork/join blocks of one layout and the lookups to place them.
struct Blocks {
    /// In [`splits::find_pairs`] order.
    pairs: Vec<SplitPair>,
    /// Indices into `pairs`, innermost (shortest layer span) first.
    order: Vec<usize>,
    /// Caller-level successors with parallel edges collapsed, as the metric sees them.
    adj: Vec<Vec<usize>>,
    /// Internal node chain of each caller edge, source first, waypoints included.
    chain_of: BTreeMap<(usize, usize), Vec<usize>>,
    /// Blocks per real node, so a move is only re-scored against the blocks it touches.
    at: Vec<Vec<u32>>,
    /// Caller weight per `(from, to)`, parallel edges summed. Picks the main branch.
    weight_of: BTreeMap<(usize, usize), f64>,
    /// Internal arcs at each internal node, for a move's straightness cost.
    arcs_at: Vec<Vec<u32>>,
}

impl Blocks {
    /// Finds the blocks of a placed state. The adjacency is rebuilt in the caller's orientation
    /// and edge order so the list matches the metric's.
    fn build(st: &LayeredState) -> Blocks {
        let n = st.n_real;
        let mut reversed = vec![false; st.routes.len()];
        for arc in &st.arcs {
            if arc.reversed {
                if let Some(flag) = reversed.get_mut(arc.edge) {
                    *flag = true;
                }
            }
        }
        let mut edges: Vec<(usize, usize)> = Vec::new();
        let mut chain_of: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
        let mut weight_of: BTreeMap<(usize, usize), f64> = BTreeMap::new();
        for (&edge, chain) in &st.chains {
            if chain.len() < 2 {
                continue;
            }
            let (first, last) = (chain[0], chain[chain.len() - 1]);
            if first >= n || last >= n || first == last {
                continue;
            }
            let flip = reversed.get(edge).copied().unwrap_or(false);
            let (from, to) = if flip { (last, first) } else { (first, last) };
            edges.push((from, to));
            *weight_of.entry((from, to)).or_insert(0.0) +=
                st.weight.get(edge).copied().unwrap_or(1.0);
            chain_of.entry((from, to)).or_insert_with(|| {
                if flip {
                    chain.iter().rev().copied().collect()
                } else {
                    chain.clone()
                }
            });
        }
        let adj = splits::successors(n, edges.iter().copied());
        let pairs = splits::find_pairs(&adj);
        let mut at: Vec<Vec<u32>> = vec![Vec::new(); n];
        for (k, p) in pairs.iter().enumerate() {
            for v in [p.fork, p.b1, p.b2, p.join] {
                if at[v].last() != Some(&(k as u32)) {
                    at[v].push(k as u32);
                }
            }
        }
        // Innermost first, so an outer block sees where its inner blocks went.
        let mut order: Vec<usize> = (0..pairs.len()).collect();
        // The pair index keeps the order total if two blocks share all four nodes.
        order.sort_unstable_by_key_dyn(|&k| {
            let p = pairs[k];
            (
                st.nodes[p.join].rank - st.nodes[p.fork].rank,
                p.fork,
                p.join,
                p.b1,
                p.b2,
                k,
            )
        });
        let mut arcs_at: Vec<Vec<u32>> = vec![Vec::new(); st.len()];
        for (i, arc) in st.arcs.iter().enumerate() {
            if arc.from == arc.to {
                continue;
            }
            arcs_at[arc.from].push(i as u32);
            arcs_at[arc.to].push(i as u32);
        }
        Blocks {
            pairs,
            order,
            adj,
            chain_of,
            at,
            weight_of,
            arcs_at,
        }
    }

    /// Candidate placements for the block, best guess first.
    fn targets(
        &self,
        st: &LayeredState,
        o: &LayoutOptions,
        cross: &[f64],
        pair: &SplitPair,
        style: BranchStyle,
    ) -> Vec<Vec<(usize, f64)>> {
        match style {
            BranchStyle::Free => vec![],
            BranchStyle::StraightMain => self.straight_main(st, cross, pair),
            BranchStyle::Symmetric => self.symmetric(st, o, cross, pair).into_iter().collect(),
        }
    }

    /// The fork, the main branch's whole chain and the join on one line.
    fn straight_main(
        &self,
        st: &LayeredState,
        cross: &[f64],
        pair: &SplitPair,
    ) -> Vec<Vec<(usize, f64)>> {
        let main = self.main_branch(st, pair);
        let Some(chain) = self.spine(st, pair.fork, main, pair.join) else {
            return vec![];
        };
        let mut ys: Vec<f64> = chain.iter().map(|&v| cross[v]).collect();
        ys.sort_unstable_by_dyn(f64::total_cmp);
        // The median moves the chain least, while the fork's or join's line moves the rest of
        // the graph least.
        let mut lines = vec![ys[ys.len() / 2], cross[pair.fork], cross[pair.join]];
        lines.dedup_by(|a, b| (*a - *b).abs() < EPS);
        lines
            .into_iter()
            .map(|line| chain.iter().map(|&v| (v, line)).collect())
            .collect()
    }

    /// Both branches mirrored about a straight fork-join line. Only the four block nodes move,
    /// and both branches must be in one layer.
    fn symmetric(
        &self,
        st: &LayeredState,
        o: &LayoutOptions,
        cross: &[f64],
        pair: &SplitPair,
    ) -> Option<Vec<(usize, f64)>> {
        let (f, j) = (pair.fork, pair.join);
        let rank = st.nodes[pair.b1].rank;
        if st.nodes[pair.b2].rank != rank
            || st.nodes[f].rank == rank
            || st.nodes[j].rank == rank
            || st.nodes[f].rank == st.nodes[j].rank
        {
            return None;
        }
        if [f, j, pair.b1, pair.b2].iter().any(|&v| st.nodes[v].pinned) {
            return None;
        }
        let line = (cross[f] + cross[j]) / 2.0;
        // The mirror must leave room for the nodes between the two branches.
        let (lo, hi) = if st.order_of[pair.b1] <= st.order_of[pair.b2] {
            (pair.b1, pair.b2)
        } else {
            (pair.b2, pair.b1)
        };
        let layer = st.layers.get(st.nodes[pair.b1].layer_clamped())?;
        let (a, b) = (st.order_of[lo], st.order_of[hi]);
        let need: f64 = (a..b).map(|i| st.sep(layer[i], layer[i + 1], o)).sum();
        let half = (need / 2.0).max((cross[hi] - cross[lo]) / 2.0);
        Some(vec![
            (f, line),
            (j, line),
            (lo, line - half),
            (hi, line + half),
        ])
    }

    /// The branch that runs straight: the heaviest, then the one whose first node is longer on
    /// the layer axis (a thin bar is a shortcut), then the lower index.
    fn main_branch(&self, st: &LayeredState, pair: &SplitPair) -> usize {
        let key = |b: usize| {
            let w = self.weight_of.get(&(pair.fork, b)).copied().unwrap_or(0.0);
            (w, st.nodes[b].w_along, -(b as f64))
        };
        let (k1, k2) = (key(pair.b1), key(pair.b2));
        if k2 > k1 {
            pair.b2
        } else {
            pair.b1
        }
    }

    /// Internal nodes from `fork` through `branch` to `join`, waypoints included. `None` if there
    /// is no such path, it does not advance one layer at a time, or it touches a pinned node.
    fn spine(
        &self,
        st: &LayeredState,
        fork: usize,
        branch: usize,
        join: usize,
    ) -> Option<Vec<usize>> {
        let mut nodes = self.chain_of.get(&(fork, branch))?.clone();
        for w in self.walk(branch, join)?.windows(2) {
            let seg = self.chain_of.get(&(w[0], w[1]))?;
            nodes.extend_from_slice(&seg[1..]);
        }
        let mut rank = i32::MIN;
        for &v in &nodes {
            if st.nodes[v].rank <= rank || (v < st.n_real && st.nodes[v].pinned) {
                return None;
            }
            rank = st.nodes[v].rank;
        }
        Some(nodes)
    }

    /// Shortest caller-level path from `branch` to `join`, ties broken by adjacency order.
    fn walk(&self, branch: usize, join: usize) -> Option<Vec<usize>> {
        if branch == join {
            return Some(vec![branch]);
        }
        let mut pred = vec![NONE; self.adj.len()];
        let mut depth = vec![0usize; self.adj.len()];
        let mut queue = vec![branch];
        pred[branch] = branch;
        let mut head = 0;
        while head < queue.len() {
            let v = queue[head];
            head += 1;
            if depth[v] >= MAX_DEPTH {
                continue;
            }
            for &w in &self.adj[v] {
                if pred[w] != NONE {
                    continue;
                }
                pred[w] = v;
                depth[w] = depth[v] + 1;
                if w == join {
                    let mut path = vec![join];
                    let mut at = join;
                    while at != branch {
                        at = pred[at];
                        path.push(at);
                    }
                    path.reverse();
                    return Some(path);
                }
                queue.push(w);
            }
        }
        None
    }
}

/// Projects every layer a target touches with the targets pinned and returns what moved. `None`
/// if a minimum gap, and so possibly the layer order, would break.
fn stage(
    st: &LayeredState,
    o: &LayoutOptions,
    base: &[f64],
    held: &[Option<f64>],
    cross: &[f64],
    targets: &[(usize, f64)],
) -> Option<Vec<(usize, f64)>> {
    let mut pin: BTreeMap<usize, f64> = BTreeMap::new();
    for &(v, y) in targets {
        if pin.insert(v, y).is_some_and(|old| (old - y).abs() > EPS) {
            return None;
        }
    }
    // Not collected, since `BTreeSet::from_iter` pulls in a stable sort that costs kilobytes of
    // wasm.
    let mut ranks: BTreeSet<i32> = BTreeSet::new();
    for &(v, _) in targets {
        ranks.insert(st.nodes[v].rank);
    }
    let mut staged: Vec<(usize, f64)> = Vec::new();
    for rank in ranks {
        let layer = st.layers.get(usize::try_from(rank).ok()?)?;
        if layer.len() < 2 {
            if let Some(&v) = layer.first() {
                if let Some(&y) = pin.get(&v) {
                    if (y - cross[v]).abs() > EPS {
                        staged.push((v, y));
                    }
                }
            }
            continue;
        }
        let line_of = |v: usize| pin.get(&v).copied().or(held[v]);
        let d: Vec<f64> = layer
            .iter()
            .map(|&v| line_of(v).unwrap_or(base[v]))
            .collect();
        let gap: Vec<f64> = (0..layer.len() - 1)
            .map(|i| st.sep(layer[i], layer[i + 1], o))
            .collect();
        let fixed: Vec<Option<f64>> = layer.iter().map(|&v| line_of(v)).collect();
        let placed = project_gaps_pinned(&d, &gap, &fixed);
        // Gaps are strictly positive, so holding them also means the layer kept its order.
        if !gaps_hold(&placed, &gap) {
            return None;
        }
        for (i, &v) in layer.iter().enumerate() {
            if (placed[i] - cross[v]).abs() > EPS {
                staged.push((v, placed[i]));
            }
        }
    }
    Some(staged)
}

/// Weighted cross-axis offset summed over the arcs a staged move touches.
fn slope(st: &LayeredState, blocks: &Blocks, staged: &[(usize, f64)], cross: &[f64]) -> f64 {
    let mut total = 0.0;
    let mut counted: BTreeSet<u32> = BTreeSet::new();
    for &(v, _) in staged {
        for &a in &blocks.arcs_at[v] {
            if !counted.insert(a) {
                continue;
            }
            let arc = st.arcs[a as usize];
            let w = st.weight.get(arc.edge).copied().unwrap_or(1.0);
            total += w * (cross[arc.from] - cross[arc.to]).abs();
        }
    }
    total
}

struct Judge<'a> {
    st: &'a LayeredState,
    blocks: &'a Blocks,
    o: &'a LayoutOptions,
    style: BranchStyle,
}

/// Applies `staged` if it satisfies more blocks and loses none the metric counted. Otherwise
/// `cross` is left unchanged.
fn commit(
    judge: &Judge,
    cross: &mut [f64],
    seen: &mut [u32],
    attempt: u32,
    staged: &[(usize, f64)],
) -> bool {
    let Judge {
        st,
        blocks,
        o,
        style,
    } = *judge;
    let mut affected: Vec<u32> = Vec::new();
    for &(v, _) in staged {
        if v >= st.n_real {
            continue;
        }
        for &k in &blocks.at[v] {
            if seen[k as usize] != attempt {
                seen[k as usize] = attempt;
                affected.push(k);
            }
        }
    }
    if affected.is_empty() {
        return false;
    }
    // `(satisfied, aligned)`, where the first must grow and the second may not shrink.
    let score = |cross: &[f64]| {
        affected.iter().fold((0usize, 0usize), |(a, b), &k| {
            let pair = &blocks.pairs[k as usize];
            (
                a + usize::from(satisfied(st, cross, pair, style)),
                b + usize::from(aligned(st, cross, pair)),
            )
        })
    };
    let before = score(cross);
    let slope_before = slope(st, blocks, staged, cross);
    let old: Vec<f64> = staged.iter().map(|&(v, _)| cross[v]).collect();
    for &(v, y) in staged {
        cross[v] = y;
    }
    let after = score(cross);
    // The slope limit stops a block being bought with diagonals elsewhere. A plain XOR costs
    // nothing here, since the skip takes on the offset the main branch gives up.
    let slack = SLOPE_SLACK_GAPS * o.spacing.node_gap * (after.0.saturating_sub(before.0)) as f64;
    if after.0 > before.0
        && after.1 >= before.1
        && slope(st, blocks, staged, cross) <= slope_before + slack + SLACK
    {
        return true;
    }
    for (&(v, _), &y) in staged.iter().zip(old.iter()) {
        cross[v] = y;
    }
    false
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use crate::geometry::Size;
    use crate::graph::{Edge, Graph, Node, NodeId};
    use crate::layout::Layout;
    use crate::options::{BranchStyle, Direction, LayoutOptions};

    fn node(w: f64, h: f64) -> Node {
        Node {
            size: Size::new(w, h),
            ..Node::default()
        }
    }

    fn edge(a: u32, b: u32) -> Edge {
        Edge {
            source: NodeId(a),
            target: NodeId(b),
            ..Edge::default()
        }
    }

    fn options(style: BranchStyle) -> LayoutOptions {
        LayoutOptions {
            direction: Direction::LeftRight,
            coords: crate::options::CoordOptions {
                branch_style: style,
                ..LayoutOptions::layered_default().coords
            },
            ..LayoutOptions::layered_default()
        }
    }

    fn run(g: &Graph, style: BranchStyle) -> Layout {
        crate::layout(g, &options(style)).expect("layout")
    }

    /// `p0 -> {t1, silent} -> p2`, the XOR common in discovered Petri nets.
    fn xor_block() -> Graph {
        Graph {
            nodes: vec![
                node(30.0, 30.0),
                node(120.0, 40.0),
                node(14.0, 40.0),
                node(30.0, 30.0),
            ],
            edges: vec![edge(0, 1), edge(0, 2), edge(1, 3), edge(2, 3)],
        }
    }

    /// The labelled transition runs straight between its places and the silent one is a skip.
    #[test]
    fn straight_main_puts_the_real_transition_on_the_line() {
        let g = xor_block();
        let l = run(&g, BranchStyle::StraightMain);
        let y = |i: usize| l.nodes[i].center.y;
        assert!(
            (y(0) - y(1)).abs() < 0.5 && (y(0) - y(3)).abs() < 0.5,
            "fork {}, main {}, join {} are not collinear",
            y(0),
            y(1),
            y(3)
        );
        assert!(
            (y(2) - y(0)).abs() > 20.0,
            "the silent transition is still on the line at {}",
            y(2)
        );
        let m = crate::metrics::measure(&g, &options(BranchStyle::StraightMain), &l);
        assert_eq!((m.split_pairs, m.split_aligned), (1, 1));
    }

    #[test]
    fn symmetric_mirrors_the_two_branches() {
        let g = xor_block();
        let l = run(&g, BranchStyle::Symmetric);
        let y = |i: usize| l.nodes[i].center.y;
        assert!((y(0) - y(3)).abs() < 0.5, "fork-join line is not straight");
        assert!(
            ((y(1) - y(0)) + (y(2) - y(0))).abs() < 0.5,
            "branches at {} and {} are not mirrored about {}",
            y(1),
            y(2),
            y(0)
        );
        let m = crate::metrics::measure(&g, &options(BranchStyle::Symmetric), &l);
        assert_eq!((m.split_pairs, m.split_aligned), (1, 1));
    }

    /// A nested block gives no overlaps and no reordered layer.
    #[test]
    fn a_nested_block_stays_legal() {
        // 0 -> {1, 2}, 1 -> {3, 4} -> 5, 5 and 2 -> 6.
        let g = Graph {
            nodes: (0..7).map(|_| node(90.0, 40.0)).collect(),
            edges: vec![
                edge(0, 1),
                edge(0, 2),
                edge(1, 3),
                edge(1, 4),
                edge(3, 5),
                edge(4, 5),
                edge(5, 6),
                edge(2, 6),
            ],
        };
        let free = run(&g, BranchStyle::Free);
        let aligned = run(&g, BranchStyle::StraightMain);
        let o = options(BranchStyle::StraightMain);
        let m = crate::metrics::measure(&g, &o, &aligned);
        assert_eq!(m.node_overlaps, 0);
        assert!(m.split_aligned >= crate::metrics::measure(&g, &o, &free).split_aligned);
        assert_orders_match(&free, &aligned);
    }

    fn assert_orders_match(free: &Layout, aligned: &Layout) {
        let key = |l: &Layout| -> Vec<(Option<i32>, u32)> {
            l.nodes.iter().map(|p| (p.rank, p.order)).collect()
        };
        assert_eq!(key(free), key(aligned), "the pass reordered a layer");
    }

    #[test]
    fn the_pass_is_deterministic_and_free_changes_nothing() {
        let g = xor_block();
        let a = run(&g, BranchStyle::StraightMain);
        let b = run(&g, BranchStyle::StraightMain);
        assert_eq!(a, b);
        let free_a = run(&g, BranchStyle::Free);
        let free_b = crate::layout(
            &g,
            &LayoutOptions {
                direction: Direction::LeftRight,
                ..LayoutOptions::layered_default()
            },
        )
        .expect("layout");
        assert_eq!(free_a, free_b);
    }

    #[test]
    fn a_graph_without_blocks_is_untouched() {
        let g = Graph {
            nodes: (0..5).map(|_| node(80.0, 40.0)).collect(),
            edges: vec![edge(0, 1), edge(1, 2), edge(2, 3), edge(1, 4)],
        };
        assert_eq!(
            run(&g, BranchStyle::Free),
            run(&g, BranchStyle::StraightMain)
        );
    }

    /// A dense DAG of overlapping blocks keeps the order crossing minimisation chose.
    #[test]
    fn no_layer_is_ever_reordered() {
        // Six ranks of three, every node feeding two of the next rank.
        let (rows, wide) = (6u32, 3u32);
        let mut edges = Vec::new();
        for r in 0..rows - 1 {
            for i in 0..wide {
                let a = r * wide + i;
                edges.push(edge(a, (r + 1) * wide + i));
                edges.push(edge(a, (r + 1) * wide + (i + 1) % wide));
            }
        }
        let g = Graph {
            nodes: (0..rows * wide).map(|_| node(80.0, 36.0)).collect(),
            edges,
        };
        let free = run(&g, BranchStyle::Free);
        for style in [BranchStyle::StraightMain, BranchStyle::Symmetric] {
            let l = run(&g, style);
            assert_orders_match(&free, &l);
            // Check the coordinates too, not just `order`.
            for rank in 0..rows as i32 {
                let mut ys: Vec<(u32, f64)> = (0..l.nodes.len())
                    .filter(|&i| l.nodes[i].rank == Some(rank))
                    .map(|i| (l.nodes[i].order, l.nodes[i].center.y))
                    .collect();
                ys.sort_unstable_by_key(|&(o, _)| o);
                assert!(
                    ys.windows(2).all(|w| w[0].1 < w[1].1),
                    "{style:?} rank {rank} came out as {ys:?}"
                );
            }
        }
    }

    /// The pass finds the blocks the metric counts, even with a reversed edge in the graph.
    #[test]
    fn the_pass_and_the_metric_find_the_same_blocks() {
        use crate::geometry::shape::NodeShape;
        use crate::layered::pipeline::{Phase, Pipeline};
        use crate::layered::prepare::{LayeredInput, LayeredInputNode};
        use crate::layered::state::LayeredState;
        use crate::splits::{self, SplitPair};
        use std::cell::RefCell;

        thread_local! {
            static FOUND: RefCell<Vec<SplitPair>> = const { RefCell::new(Vec::new()) };
        }
        fn capture(
            st: &mut LayeredState,
            _o: &LayoutOptions,
        ) -> Result<(), crate::error::LayoutError> {
            let pairs = super::Blocks::build(st).pairs;
            FOUND.with(|f| *f.borrow_mut() = pairs);
            Ok(())
        }

        // 0 -> {1, 2} -> 3 -> {4, 5} -> 6, plus 6 -> 3 so the cycle breaker reverses an edge.
        let edges = vec![
            (0, 1),
            (0, 2),
            (1, 3),
            (2, 3),
            (3, 4),
            (3, 5),
            (4, 6),
            (5, 6),
            (6, 3),
        ];
        let n = 7;
        let input = LayeredInput {
            nodes: (0..n)
                .map(|_| LayeredInputNode {
                    width: 90.0,
                    height: 40.0,
                    shape: NodeShape::Box,
                    constraint: None,
                    category: None,
                    clear_after: 0.0,
                    clear_before: 0.0,
                    port_sides: None,
                    lane_key: None,
                    along_before: 0.0,
                    along_after: 0.0,
                })
                .collect(),
            edges: edges.clone(),
            weights: vec![],
            thickness: vec![],
            edge_label_sizes: vec![],
            seed: vec![],
            pinned: vec![],
            self_loops: vec![],
            ranked_backwards: Vec::new(),
            options: options(BranchStyle::StraightMain),
        };
        let mut st = LayeredState::new(&input);
        let mut pipeline = Pipeline::layered();
        pipeline.phases.push(Phase::Custom(capture));
        pipeline.run(&mut st, &input.options).expect("pipeline");
        assert!(st.arcs.iter().any(|a| a.reversed), "no edge was reversed");

        let expected = splits::find_pairs(&splits::successors(n, edges));
        assert!(!expected.is_empty());
        assert_eq!(FOUND.with(|f| f.borrow().clone()), expected);
    }
}

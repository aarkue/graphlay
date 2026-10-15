//! Fork/join blocks, and the test for whether a drawing shows one as a readable block.
//!
//! [`crate::metrics::LayoutMetrics::split_alignment`] and the layered `AlignSplits` pass share
//! this one definition, so the metric measures exactly what the pass optimises. Positions are
//! plain `(along, cross)` pairs, so output space and flow space both work.

use crate::geometry::fp;
use crate::geometry::EPS;

/// Forks with more outgoing edges than this are skipped. A wide fan does not read as a block,
/// and the pairwise search over its successors is quadratic.
pub(crate) const MAX_OUT_DEGREE: usize = 6;

/// How far from a branch the join is looked for, in edges.
pub(crate) const MAX_DEPTH: usize = 12;

/// How far off the line a node may sit and still count as on it, in px.
pub(crate) const ALIGN_TOL: f64 = 2.0;

/// One fork/join block. `fork` splits into `b1` and `b2`, which meet again at `join`.
///
/// The branches are immediate successors and `join` is the nearest node reachable from both.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct SplitPair {
    /// The node the block splits at.
    pub fork: usize,
    /// One immediate successor of `fork`.
    pub b1: usize,
    /// The other one.
    pub b2: usize,
    /// The nearest node reachable from both branches.
    pub join: usize,
}

/// Successor lists over `n` nodes, without self-loops, parallel edges or out-of-range ends.
///
/// Lists keep edge arrival order, which makes [`find_pairs`] deterministic without sorting.
pub(crate) fn successors(
    n: usize,
    edges: impl IntoIterator<Item = (usize, usize)>,
) -> Vec<Vec<usize>> {
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (a, b) in edges {
        if a < n && b < n && a != b && !adj[a].contains(&b) {
            adj[a].push(b);
        }
    }
    adj
}

/// Every fork/join block in `adj`, in a deterministic order: forks ascending, then the branch
/// pairs in adjacency order.
///
/// See [`nearest_common`] for how the join is picked.
pub(crate) fn find_pairs(adj: &[Vec<usize>]) -> Vec<SplitPair> {
    let n = adj.len();
    let mut out = Vec::new();
    if n == 0 {
        return out;
    }
    // `dist[k][i]` is the hop count from branch k to node i, `usize::MAX` if not reached.
    // Reused across forks. `touched` lists the entries to reset.
    let mut dist: Vec<Vec<usize>> = vec![vec![usize::MAX; n]; MAX_OUT_DEGREE];
    let mut touched: Vec<Vec<usize>> = vec![Vec::new(); MAX_OUT_DEGREE];
    let mut queue: Vec<usize> = Vec::new();

    for fork in 0..n {
        let branches = &adj[fork];
        if branches.len() < 2 || branches.len() > MAX_OUT_DEGREE {
            continue;
        }
        for (k, &b) in branches.iter().enumerate() {
            bounded_bfs(adj, b, &mut dist[k], &mut touched[k], &mut queue);
        }
        for i in 0..branches.len() {
            for j in i + 1..branches.len() {
                if let Some(join) = nearest_common(&dist[i], &dist[j], fork) {
                    out.push(SplitPair {
                        fork,
                        b1: branches[i],
                        b2: branches[j],
                        join,
                    });
                }
            }
        }
        for k in 0..branches.len() {
            for &i in &touched[k] {
                dist[k][i] = usize::MAX;
            }
            touched[k].clear();
        }
    }
    out
}

/// Hop counts from `start`, out to [`MAX_DEPTH`] edges. `dist` must be all `usize::MAX` on entry.
/// Every written index goes onto `touched`, so the reset costs only what was visited.
fn bounded_bfs(
    adj: &[Vec<usize>],
    start: usize,
    dist: &mut [usize],
    touched: &mut Vec<usize>,
    queue: &mut Vec<usize>,
) {
    queue.clear();
    queue.push(start);
    dist[start] = 0;
    touched.push(start);
    let mut head = 0;
    while head < queue.len() {
        let v = queue[head];
        head += 1;
        let d = dist[v];
        if d >= MAX_DEPTH {
            continue;
        }
        for &w in &adj[v] {
            if dist[w] == usize::MAX {
                dist[w] = d + 1;
                touched.push(w);
                queue.push(w);
            }
        }
    }
}

/// The node reachable from both branches with the smallest total hop count, ties to the lower
/// index. `fork` itself is never the join, since an edge back to the fork does not close a split.
fn nearest_common(a: &[usize], b: &[usize], fork: usize) -> Option<usize> {
    let mut best: Option<(usize, usize)> = None;
    for i in 0..a.len() {
        if i == fork || a[i] == usize::MAX || b[i] == usize::MAX {
            continue;
        }
        let total = a[i] + b[i];
        if best.is_none_or(|(t, _)| total < t) {
            best = Some((total, i));
        }
    }
    best.map(|(_, i)| i)
}

/// Whether `pair` is drawn as one readable block.
///
/// `pos` gives a node's centre as `(along, cross)` relative to the layout direction.
/// Two shapes count:
///
/// 1. A straight main line ([`is_straight_main`]). `fork`, one branch and `join` share a cross
///    position within [`ALIGN_TOL`], so the other branch reads as a skip.
/// 2. A symmetric block ([`is_mirrored`]). The branches mirror each other about the
///    `fork`-`join` line.
pub(crate) fn is_aligned(pos: impl Fn(usize) -> (f64, f64), pair: &SplitPair) -> bool {
    is_straight_main(&pos, pair) || is_mirrored(&pos, pair)
}

/// Shape 1 of [`is_aligned`]: `fork`, one branch and `join` on one cross line.
pub(crate) fn is_straight_main(pos: impl Fn(usize) -> (f64, f64), pair: &SplitPair) -> bool {
    let SplitPair { fork, b1, b2, join } = *pair;
    let cross = |i: usize| pos(i).1;
    (cross(fork) - cross(join)).abs() <= ALIGN_TOL
        && ((cross(fork) - cross(b1)).abs() <= ALIGN_TOL
            || (cross(fork) - cross(b2)).abs() <= ALIGN_TOL)
}

/// Shape 2 of [`is_aligned`]: the branches at the same distance along the `fork`-`join` line and
/// opposite distances across it.
pub(crate) fn is_mirrored(pos: impl Fn(usize) -> (f64, f64), pair: &SplitPair) -> bool {
    let SplitPair { fork, b1, b2, join } = *pair;
    let (fx, fy) = pos(fork);
    let (jx, jy) = pos(join);
    let (dx, dy) = (jx - fx, jy - fy);
    let len = fp::hypot(dx, dy);
    if len < EPS {
        return false; // fork and join coincide, so there is no line to mirror about
    }
    let (ux, uy) = (dx / len, dy / len);
    let project = |i: usize| {
        let p = pos(i);
        let (vx, vy) = (p.0 - fx, p.1 - fy);
        (vx * ux + vy * uy, vx * -uy + vy * ux)
    };
    let (along1, across1) = project(b1);
    let (along2, across2) = project(b2);
    (along1 - along2).abs() <= ALIGN_TOL && (across1 + across2).abs() <= ALIGN_TOL
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// A diamond is one block, with branches in edge order.
    #[test]
    fn a_diamond_is_one_block() {
        let adj = successors(4, [(0, 1), (0, 2), (1, 3), (2, 3)]);
        assert_eq!(
            find_pairs(&adj),
            vec![SplitPair {
                fork: 0,
                b1: 1,
                b2: 2,
                join: 3
            }]
        );
    }

    /// Parallel edges are one branch, and a self-loop is not a branch at all.
    #[test]
    fn parallel_edges_and_self_loops_are_not_branches() {
        let adj = successors(3, [(0, 1), (0, 1), (0, 0), (1, 2)]);
        assert_eq!(adj[0], vec![1]);
        assert!(find_pairs(&adj).is_empty());
    }

    /// Branches that never meet again are not a block, so a tree reports no pairs.
    #[test]
    fn branches_that_never_meet_are_not_a_block() {
        let adj = successors(5, [(0, 1), (0, 2), (1, 3), (2, 4)]);
        assert!(find_pairs(&adj).is_empty());
    }

    /// The out-degree cap and the depth cap both hold.
    #[test]
    fn the_search_is_capped() {
        let mut edges: Vec<(usize, usize)> = Vec::new();
        for b in 1..=MAX_OUT_DEGREE + 1 {
            edges.push((0, b));
            edges.push((b, MAX_OUT_DEGREE + 2));
        }
        let n = MAX_OUT_DEGREE + 3;
        assert!(find_pairs(&successors(n, edges.iter().copied())).is_empty());
        // One fewer branch and the same graph is a block per pair.
        edges.retain(|&(a, b)| !(a == 0 && b == MAX_OUT_DEGREE + 1));
        let pairs = find_pairs(&successors(n, edges.iter().copied()));
        assert_eq!(pairs.len(), MAX_OUT_DEGREE * (MAX_OUT_DEGREE - 1) / 2);

        // Two chains of MAX_DEPTH edges meeting one edge further on put the join one hop out of
        // reach.
        let (a0, b0, join) = (1, MAX_DEPTH + 2, 2 * MAX_DEPTH + 3);
        let mut long = vec![(0, a0), (0, b0)];
        for k in 0..MAX_DEPTH {
            long.push((a0 + k, a0 + k + 1));
            long.push((b0 + k, b0 + k + 1));
        }
        long.push((a0 + MAX_DEPTH, join));
        long.push((b0 + MAX_DEPTH, join));
        assert!(find_pairs(&successors(join + 1, long.iter().copied())).is_empty());
        // One hop closer and it is a block again.
        long.retain(|&(a, _)| a != a0 + MAX_DEPTH - 1 && a != b0 + MAX_DEPTH - 1);
        long.push((a0 + MAX_DEPTH - 1, join));
        long.push((b0 + MAX_DEPTH - 1, join));
        assert_eq!(find_pairs(&successors(join + 1, long)).len(), 1);
    }

    /// Straight main line and mirrored block, in both spaces.
    #[test]
    fn alignment_recognises_both_shapes() {
        let pair = SplitPair {
            fork: 0,
            b1: 1,
            b2: 2,
            join: 3,
        };
        // fork, b1 and join on one cross line, b2 off to the side.
        let straight = [(0.0, 100.0), (100.0, 100.0), (100.0, 40.0), (200.0, 100.0)];
        assert!(is_aligned(|i| straight[i], &pair));
        // Mirrored branches, fork and join off the branch line entirely.
        let mirror = [(0.0, 100.0), (100.0, 40.0), (100.0, 160.0), (200.0, 100.0)];
        assert!(is_aligned(|i| mirror[i], &pair));
        // Neither: b1 and b2 both below the fork-join line by different amounts.
        let neither = [(0.0, 100.0), (100.0, 40.0), (100.0, 20.0), (200.0, 100.0)];
        assert!(!is_aligned(|i| neither[i], &pair));
        // Swapping coordinates is a reflection, which must not change the answer.
        let swapped: Vec<(f64, f64)> = mirror.iter().map(|&(a, c)| (c, a)).collect();
        assert!(is_aligned(|i| swapped[i], &pair));
    }
}

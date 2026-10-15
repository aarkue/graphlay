//! Route and path shape metrics: bends, lengths, per-edge turn angles and the straightness of
//! the heaviest source-to-sink path.
//!
//! On path-following tasks, continuity (keeping a multi-edge path straight) and crossings cost
//! most after path length (Ware, Purchase, Colpoys and McGill).

use super::{
    distance, EdgeGeom, LayoutMetrics, NodeBox, Problem, BEND_MIN_DEG, MAIN_PATH_TOL, STEP_MAX_PX,
};
use crate::geometry::fp;
use crate::geometry::{stats, EPS};

/// `(total bends, edges with at least one bend)`.
pub(super) fn count_bends(edges: &[EdgeGeom]) -> (usize, usize) {
    let min_cos = fp::cos(BEND_MIN_DEG.to_radians());
    let mut total = 0;
    let mut with_bends = 0;
    for e in edges {
        let mut here = 0;
        for (cos, _) in turns(e) {
            if cos < min_cos {
                here += 1;
            }
        }
        total += here;
        if here > 0 {
            with_bends += 1;
        }
    }
    (total, with_bends)
}

/// Counts [`LayoutMetrics::excess_bends`], the bends beyond the minimum each orthogonal route's
/// two borders allow.
pub(super) fn excess_bends(edges: &[EdgeGeom], boxes: &[NodeBox]) -> usize {
    let min_cos = fp::cos(BEND_MIN_DEG.to_radians());
    let mut total = 0;
    for e in edges {
        if e.from == e.to {
            continue;
        }
        let (Some(a), Some(b)) = (boxes.get(e.from), boxes.get(e.to)) else {
            continue;
        };
        let Some((leave, arrive)) = end_directions(e) else {
            continue;
        };
        let bends = turns(e).iter().filter(|(cos, _)| *cos < min_cos).count();
        total += bends.saturating_sub(min_orthogonal_bends(a, leave, b, arrive));
    }
    total
}

/// A unit axis direction: `(axis, sign)`, axis `0` for `x` and `1` for `y`.
type AxisDir = (usize, f64);

/// The directions a route leaves its source and enters its target, or `None` for a route that is
/// not orthogonal.
fn end_directions(e: &EdgeGeom) -> Option<(AxisDir, AxisDir)> {
    let mut dirs: Vec<AxisDir> = Vec::new();
    for s in e.pts.windows(2) {
        let (dx, dy) = (s[1][0] - s[0][0], s[1][1] - s[0][1]);
        if fp::hypot(dx, dy) < 0.5 {
            continue;
        }
        if dy.abs() < 0.5 {
            dirs.push((0, dx.signum()));
        } else if dx.abs() < 0.5 {
            dirs.push((1, dy.signum()));
        } else {
            return None;
        }
    }
    Some((*dirs.first()?, *dirs.last()?))
}

/// The fewest bends of an orthogonal route leaving `a` along `leave` and entering `b` along
/// `arrive`, with ports free along their borders and no obstacles.
fn min_orthogonal_bends(a: &NodeBox, leave: AxisDir, b: &NodeBox, arrive: AxisDir) -> usize {
    const TOL: f64 = 0.5;
    let lo = |n: &NodeBox, axis: usize| if axis == 0 { n.left() } else { n.top() };
    let hi = |n: &NodeBox, axis: usize| if axis == 0 { n.right() } else { n.bottom() };
    let face =
        |n: &NodeBox, (axis, sign): AxisDir| if sign > 0.0 { hi(n, axis) } else { lo(n, axis) };
    let (la, ls) = leave;
    let (aa, asg) = arrive;
    let p_line = face(a, leave);
    let q_line = face(b, (aa, -asg));
    if la == aa {
        if (asg - ls).abs() > f64::EPSILON {
            // A U.
            return 2;
        }
        let ahead = (q_line - p_line) * ls > TOL;
        let c = 1 - la;
        let share = lo(a, c).max(lo(b, c)) <= hi(a, c).min(hi(b, c)) + TOL;
        return match (ahead, share) {
            (true, true) => 0,
            (true, false) => 2,
            (false, _) => 4,
        };
    }
    // Perpendicular borders allow an L if the target border is ahead of some source port along
    // `leave`, and some target port is beyond the source border along `arrive`.
    let ahead_leave = if ls > 0.0 {
        hi(b, la) - p_line > TOL
    } else {
        p_line - lo(b, la) > TOL
    };
    let ahead_arrive = if asg > 0.0 {
        q_line - lo(a, aa) > TOL
    } else {
        hi(a, aa) - q_line > TOL
    };
    if ahead_leave && ahead_arrive {
        1
    } else {
        3
    }
}

/// Every joint of a polyline as `(cosine of the turn, turn in degrees)`, skipping degenerate
/// segments.
fn turns(e: &EdgeGeom) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    let mut prev: Option<[f64; 2]> = None;
    for s in e.pts.windows(2) {
        let d = [s[1][0] - s[0][0], s[1][1] - s[0][1]];
        let len = fp::hypot(d[0], d[1]);
        if len < EPS {
            continue;
        }
        let dir = [d[0] / len, d[1] / len];
        if let Some(p) = prev {
            let cos = (p[0] * dir[0] + p[1] * dir[1]).clamp(-1.0, 1.0);
            out.push((cos, cos.acos().to_degrees()));
        }
        prev = Some(dir);
    }
    out
}

/// Fills the four length metrics.
pub(super) fn fill_length_metrics(edges: &[EdgeGeom], m: &mut LayoutMetrics) {
    let mut total = 0.0;
    let mut max = 0.0f64;
    let mut detour_poly = 0.0;
    let mut detour_straight = 0.0;
    for e in edges {
        let len: f64 = e.pts.windows(2).map(|s| distance(s[0], s[1])).sum();
        total += len;
        max = max.max(len);
        let straight = distance(e.pts[0], e.pts[e.pts.len() - 1]);
        if straight > EPS {
            detour_poly += len;
            detour_straight += straight;
        }
    }
    m.total_edge_length = total;
    m.mean_edge_length = if edges.is_empty() {
        0.0
    } else {
        total / edges.len() as f64
    };
    m.max_edge_length = max;
    m.detour_ratio = if detour_straight > EPS {
        detour_poly / detour_straight
    } else {
        1.0
    };
}

/// Fills [`LayoutMetrics::path_turn_mean`] and [`LayoutMetrics::path_turn_max`].
///
/// [`LayoutMetrics::bends`] counts every joint alike, so four 6 degree jinks score worse than one
/// 170 degree reversal. The weighted mean total turn and the sharpest turn tell those apart.
pub(super) fn fill_path_continuity(edges: &[EdgeGeom], m: &mut LayoutMetrics) {
    let mut weighted = 0.0;
    let mut weight_total = 0.0;
    let mut max_turn = 0.0f64;
    for e in edges {
        let mut edge_total = 0.0;
        for (_, degrees) in turns(e) {
            edge_total += degrees;
            max_turn = max_turn.max(degrees);
        }
        // `measure` does not validate, so clamp a negative weight to keep the mean finite.
        let w = e.weight.max(0.0);
        weighted += w * edge_total;
        weight_total += w;
    }
    m.path_turn_mean = if weight_total > 0.0 {
        weighted / weight_total
    } else {
        0.0
    };
    m.path_turn_max = max_turn;
}

/// Fills [`LayoutMetrics::short_steps`], [`LayoutMetrics::route_reversals`] and
/// [`LayoutMetrics::staircase_steps`].
///
/// A step is three consecutive segments with the outer two parallel and the middle one
/// perpendicular and shorter than [`STEP_MAX_PX`]. A reversal is a sign change in one
/// coordinate's non-zero increments. Self-loops always reverse, so they are skipped.
pub(super) fn fill_step_metrics(edges: &[EdgeGeom], m: &mut LayoutMetrics) {
    let parallel_cos = fp::cos(10f64.to_radians());
    let perpendicular_cos = fp::cos(80f64.to_radians());
    let mut steps = 0;
    let mut reversals = 0;
    let mut stairs = 0;
    for e in edges {
        if e.from == e.to {
            continue;
        }
        stairs += staircase_steps(&e.pts);
        let segs: Vec<([f64; 2], f64)> = e
            .pts
            .windows(2)
            .filter_map(|s| {
                let d = [s[1][0] - s[0][0], s[1][1] - s[0][1]];
                let len = fp::hypot(d[0], d[1]);
                (len > 0.5).then(|| ([d[0] / len, d[1] / len], len))
            })
            .collect();
        for w in segs.windows(3) {
            let (a, b, c) = (w[0], w[1], w[2]);
            let dot = |p: [f64; 2], q: [f64; 2]| p[0] * q[0] + p[1] * q[1];
            if b.1 < STEP_MAX_PX
                && dot(a.0, c.0).abs() > parallel_cos
                && dot(a.0, b.0).abs() < perpendicular_cos
            {
                steps += 1;
            }
        }
        for axis in 0..2 {
            let mut last_sign = 0.0f64;
            for s in e.pts.windows(2) {
                let d = s[1][axis] - s[0][axis];
                if d.abs() < 0.5 {
                    continue;
                }
                if last_sign * d < 0.0 {
                    reversals += 1;
                }
                last_sign = d.signum();
            }
        }
    }
    m.short_steps = steps;
    m.route_reversals = reversals;
    m.staircase_steps = stairs;
}

/// Consecutive same-direction jogs of one polyline. A jog is an axis-aligned segment across the
/// main axis between two runs along it. The main axis is the one its segments cover more of.
fn staircase_steps(pts: &[[f64; 2]]) -> usize {
    // (horizontal, signed extent) per axis-aligned segment, None for a diagonal.
    let mut segs: Vec<Option<(bool, f64)>> = Vec::new();
    let (mut ext_x, mut ext_y) = (0.0, 0.0);
    for s in pts.windows(2) {
        let (dx, dy) = (s[1][0] - s[0][0], s[1][1] - s[0][1]);
        if dx.abs() < 1.0 && dy.abs() >= 1.0 {
            ext_y += dy.abs();
            segs.push(Some((false, dy)));
        } else if dy.abs() < 1.0 && dx.abs() >= 1.0 {
            ext_x += dx.abs();
            segs.push(Some((true, dx)));
        } else if dx.abs() >= 1.0 || dy.abs() >= 1.0 {
            segs.push(None);
        }
    }
    let main_is_x = ext_x >= ext_y;
    let mut jog_signs: Vec<bool> = Vec::new();
    for i in 1..segs.len().saturating_sub(1) {
        let (Some(p), Some(c), Some(n)) = (segs[i - 1], segs[i], segs[i + 1]) else {
            continue;
        };
        if c.0 != main_is_x && p.0 == main_is_x && n.0 == main_is_x {
            jog_signs.push(c.1 > 0.0);
        }
    }
    jog_signs.windows(2).filter(|w| w[0] == w[1]).count()
}

/// The heaviest source-to-sink path and how straight it is drawn (after Mennens et al.).
/// [`LayoutMetrics::split_alignment`] is per block, so this checks the global path.
///
/// The path is the widest one (heaviest lightest arc, then heaviest total) over the acyclic
/// orientation the layering used, with [`crate::Layout::reversed`] edges turned round. Sources are
/// nodes with no in-edges plus every [`crate::RankConstraint::First`] node.
pub(super) fn fill_main_path(problem: &Problem, boxes: &[NodeBox], m: &mut LayoutMetrics) {
    // An empty drawing has nothing off the line, which keeps its score term at zero.
    m.main_path_aligned_fraction = 1.0;
    let n = boxes.len().min(problem.node_count);
    if n == 0 {
        return;
    }
    let oriented = orient(problem, n);
    let path = heaviest_path(&oriented, problem, n);
    m.main_path_len = path.len();
    if path.len() < 2 {
        return;
    }

    let pos = |i: usize| {
        let b = &boxes[i];
        if problem.left_right {
            (b.cx, b.cy)
        } else {
            (b.cy, b.cx)
        }
    };
    let mut cross: Vec<f64> = path.iter().map(|&i| pos(i).1).collect();
    let along: Vec<f64> = path.iter().map(|&i| pos(i).0).collect();
    let median = stats::median(&mut cross);
    let aligned = path
        .iter()
        .filter(|&&i| (pos(i).1 - median).abs() <= MAIN_PATH_TOL)
        .count();
    m.main_path_aligned_fraction = aligned as f64 / path.len() as f64;

    let deviation: f64 =
        path.iter().map(|&i| (pos(i).1 - median).abs()).sum::<f64>() / path.len() as f64;
    let span = along.iter().copied().fold(f64::NEG_INFINITY, f64::max)
        - along.iter().copied().fold(f64::INFINITY, f64::min);
    m.main_path_deviation = if span > EPS { deviation / span } else { 0.0 };
}

/// One oriented arc of the acyclic view: `(from, to, weight)`.
type Arc = (usize, usize, f64);

/// The graph as the layering saw it, with self-loops dropped and every edge in
/// [`Problem::reversed`] turned round.
fn orient(problem: &Problem, n: usize) -> Vec<Arc> {
    problem
        .edges
        .iter()
        .enumerate()
        .filter_map(|(i, &(a, b))| {
            if a == b || a >= n || b >= n {
                return None;
            }
            let w = problem.weights.get(i).copied().unwrap_or(1.0);
            Some(if problem.reversed.get(i).copied().unwrap_or(false) {
                (b, a, w)
            } else {
                (a, b, w)
            })
        })
        .collect()
}

/// Nodes of the maximum-weight source-to-sink path, in path order.
///
/// Dynamic programming over a topological order. A layout from another engine can keep a cycle.
/// Its nodes are appended in index order and backward relaxations are dropped, so the walk
/// terminates.
fn heaviest_path(arcs: &[Arc], problem: &Problem, n: usize) -> Vec<usize> {
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indegree = vec![0usize; n];
    for (k, &(a, b, _)) in arcs.iter().enumerate() {
        out[a].push(k);
        indegree[b] += 1;
    }
    let is_source =
        |i: usize| indegree[i] == 0 || problem.rank_first.get(i).copied().unwrap_or(false);

    // Kahn, lowest index first, for determinism.
    let mut remaining = indegree.clone();
    let mut order: Vec<usize> = Vec::with_capacity(n);
    let mut ready: Vec<usize> = (0..n).filter(|&i| remaining[i] == 0).collect();
    while let Some(v) = pop_lowest(&mut ready) {
        order.push(v);
        for &k in &out[v] {
            let to = arcs[k].1;
            remaining[to] -= 1;
            if remaining[to] == 0 {
                ready.push(to);
            }
        }
    }
    let mut rank_in_order = vec![usize::MAX; n];
    for (r, &v) in order.iter().enumerate() {
        rank_in_order[v] = r;
    }
    for (v, rank) in rank_in_order.iter_mut().enumerate() {
        if *rank == usize::MAX {
            *rank = order.len();
            order.push(v);
        }
    }

    // `(bottleneck, total)`. A plain heaviest-sum path would prefer a detour of two light arcs
    // over the heavy arc that skips it.
    let mut pred: Vec<Option<(usize, f64)>> = vec![None; n];
    let unreached = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    let mut best: Vec<(f64, f64)> = (0..n)
        .map(|i| {
            if is_source(i) {
                (f64::INFINITY, 0.0)
            } else {
                unreached
            }
        })
        .collect();
    for &v in &order {
        if best[v] == unreached {
            continue;
        }
        for &k in &out[v] {
            let (_, to, w) = arcs[k];
            if rank_in_order[to] <= rank_in_order[v] {
                continue; // surviving cycle
            }
            let candidate = (best[v].0.min(w), best[v].1 + w);
            let wins = candidate.0 > best[to].0 + EPS
                || ((candidate.0 - best[to].0).abs() <= EPS && candidate.1 > best[to].1 + EPS);
            if wins {
                best[to] = candidate;
                pred[to] = Some((v, w));
            }
        }
    }

    // Prefer a true sink (no out-edges in the input). A node whose only out-edge loops back is
    // also an oriented sink, but ending there stops at the rework. Fall back to oriented sinks,
    // then any reached node.
    let mut leaves = vec![false; n];
    for &(a, b) in &problem.edges {
        if a != b && a < n {
            leaves[a] = true;
        }
    }
    let sink = pick_end(&best, |i| !leaves[i])
        .or_else(|| pick_end(&best, |i| out[i].is_empty()))
        .or_else(|| pick_end(&best, |_| true));
    let Some(mut v) = sink else { return Vec::new() };
    let mut path = vec![v];
    let mut seen = vec![false; n];
    seen[v] = true;
    while let Some((u, _)) = pred[v] {
        if seen[u] {
            break;
        }
        seen[u] = true;
        path.push(u);
        v = u;
    }
    path.reverse();
    path
}

/// Remove and return the lowest value in `ready`.
fn pop_lowest(ready: &mut Vec<usize>) -> Option<usize> {
    let at = ready
        .iter()
        .enumerate()
        .min_by_key(|(_, &v)| v)
        .map(|(i, _)| i)?;
    Some(ready.swap_remove(at))
}

/// The reached node with the largest `best` among those `allowed`, ties to the lower index.
fn pick_end(best: &[(f64, f64)], allowed: impl Fn(usize) -> bool) -> Option<usize> {
    let mut chosen: Option<((f64, f64), usize)> = None;
    for (i, &b) in best.iter().enumerate() {
        // A source's own `(inf, 0)` is not a path.
        if b.0 == f64::NEG_INFINITY || b.0 == f64::INFINITY || !allowed(i) {
            continue;
        }
        let beats = |w: (f64, f64)| b.0 > w.0 || (b.0 == w.0 && b.1 > w.1);
        if chosen.is_none_or(|(w, _)| beats(w)) {
            chosen = Some((b, i));
        }
    }
    chosen.map(|(_, i)| i)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::super::measure;
    use super::super::tests::compute;
    use super::super::tests::{boxes, layout, spec};
    use crate::geometry::{Point, Size};
    use crate::layout::{EdgeRoute, Layout, Placement};
    use crate::{Direction, Edge, EdgeId, Graph, LayoutOptions, Node, NodeId};

    /// The fixtures are drawn top to bottom, and the default direction is left to right.
    fn tb() -> LayoutOptions {
        LayoutOptions {
            direction: Direction::TopBottom,
            ..LayoutOptions::default()
        }
    }

    /// A chain of `n` nodes with the given centres, drawn with straight border-to-border edges.
    fn chain(centers: &[[f64; 2]], weights: &[f64]) -> (Graph, Layout) {
        let size = Size::new(60.0, 30.0);
        let nodes: Vec<Node> = centers
            .iter()
            .map(|_| Node {
                size,
                ..Node::default()
            })
            .collect();
        let edges: Vec<Edge> = (0..centers.len().saturating_sub(1))
            .map(|i| Edge {
                weight: weights.get(i).copied().unwrap_or(1.0),
                ..Edge::new(NodeId(i as u32), NodeId(i as u32 + 1))
            })
            .collect();
        let placements: Vec<Placement> = centers
            .iter()
            .map(|c| Placement {
                center: Point::new(c[0], c[1]),
                size,
                ..Placement::default()
            })
            .collect();
        let routes: Vec<EdgeRoute> = (0..edges.len())
            .map(|i| EdgeRoute {
                points: vec![
                    Point::new(centers[i][0], centers[i][1] + 15.0),
                    Point::new(centers[i + 1][0], centers[i + 1][1] - 15.0),
                ],
                ..EdgeRoute::default()
            })
            .collect();
        (
            Graph { nodes, edges },
            Layout {
                nodes: placements,
                edges: routes,
                ..Layout::default()
            },
        )
    }

    /// Right-angle joins are bends, a 2 degree kink is not.
    #[test]
    fn bends_need_five_degrees() {
        let s = spec(boxes(4), vec![(0, 1), (2, 3)], "TB");
        let l = layout(
            vec![[0.0, 0.0], [0.0, 400.0], [1000.0, 0.0], [1000.0, 400.0]],
            vec![
                // Two right angles.
                vec![[0.0, 25.0], [0.0, 100.0], [100.0, 100.0], [100.0, 375.0]],
                // About 2 degrees off straight, so not a bend.
                vec![[1000.0, 25.0], [1000.0, 200.0], [1006.0, 375.0]],
            ],
        );
        let m = compute(&s, &l);
        assert_eq!(m.bends, 2);
        assert_eq!(m.edges_with_bends, 1);
    }

    /// Lengths and the detour ratio come straight from the polylines.
    #[test]
    fn lengths_and_detour_ratio() {
        let s = spec(boxes(4), vec![(0, 1), (2, 3)], "TB");
        let l = layout(
            vec![[0.0, 0.0], [0.0, 100.0], [500.0, 0.0], [500.0, 100.0]],
            vec![
                vec![[0.0, 0.0], [0.0, 100.0]],                  // straight, 100
                vec![[500.0, 0.0], [530.0, 40.0], [500.0, 0.0]], // out and back, 100 for 0 straight
            ],
        );
        let m = compute(&s, &l);
        assert!((m.total_edge_length - 200.0).abs() < 1e-6);
        assert!((m.mean_edge_length - 100.0).abs() < 1e-6);
        assert!((m.max_edge_length - 100.0).abs() < 1e-6);
        // The second edge's endpoints coincide, so it is left out of the ratio.
        assert!((m.detour_ratio - 1.0).abs() < 1e-6);
    }

    #[test]
    fn detour_ratio_counts_the_detour() {
        let s = spec(boxes(2), vec![(0, 1)], "TB");
        let l = layout(
            vec![[0.0, 0.0], [0.0, 100.0]],
            vec![vec![[0.0, 0.0], [30.0, 40.0], [0.0, 80.0]]], // 100 drawn for 80 straight
        );
        assert!((compute(&s, &l).detour_ratio - 1.25).abs() < 1e-6);
    }

    /// Total turn measures how hard the turns are, which `bends` cannot.
    #[test]
    fn path_turn_measures_how_hard_the_turns_are() {
        let s = spec(boxes(2), vec![(0, 1)], "TB");
        let square = layout(
            vec![[0.0, 0.0], [0.0, 400.0]],
            vec![vec![[0.0, 25.0], [100.0, 25.0], [100.0, 375.0]]],
        );
        let m = compute(&s, &square);
        assert_eq!(m.bends, 1);
        assert!((m.path_turn_mean - 90.0).abs() < 1e-9, "{m:?}");
        assert!((m.path_turn_max - 90.0).abs() < 1e-9, "{m:?}");
        let straight = layout(
            vec![[0.0, 0.0], [0.0, 400.0]],
            vec![vec![[0.0, 25.0], [0.0, 375.0]]],
        );
        let m = compute(&s, &straight);
        assert_eq!(m.path_turn_mean, 0.0);
        assert_eq!(m.path_turn_max, 0.0);
    }

    /// Right, up a little, right, down a little, right has two short steps and one reversal. A run
    /// that steps down by more than 12px is a detour, not a step, but still reverses.
    #[test]
    fn short_steps_and_reversals_see_the_jitter() {
        let s = spec(boxes(2), vec![(0, 1)], "LR");
        let jittery = layout(
            vec![[0.0, 0.0], [400.0, 0.0]],
            vec![vec![
                [25.0, 0.0],
                [100.0, 0.0],
                [100.0, -6.0],
                [200.0, -6.0],
                [200.0, 2.0],
                [375.0, 2.0],
            ]],
        );
        let m = compute(&s, &jittery);
        assert_eq!(m.short_steps, 2, "{m:?}");
        assert_eq!(m.route_reversals, 1, "{m:?}");
        let detour = layout(
            vec![[0.0, 0.0], [400.0, 0.0]],
            vec![vec![
                [25.0, 0.0],
                [100.0, 0.0],
                [100.0, -40.0],
                [200.0, -40.0],
                [200.0, 0.0],
                [375.0, 0.0],
            ]],
        );
        let m = compute(&s, &detour);
        assert_eq!(m.short_steps, 0);
        assert_eq!(m.route_reversals, 1);
        let straight = layout(
            vec![[0.0, 0.0], [400.0, 0.0]],
            vec![vec![[25.0, 0.0], [375.0, 0.0]]],
        );
        let m = compute(&s, &straight);
        assert_eq!((m.short_steps, m.route_reversals), (0, 0));
    }

    /// Right, up, right, up, right is a staircase of one step. Right, up, right, down is not.
    #[test]
    fn staircase_steps_count_same_direction_jogs() {
        let s = spec(boxes(2), vec![(0, 1)], "LR");
        let stairs = layout(
            vec![[0.0, 0.0], [400.0, 0.0]],
            vec![vec![
                [25.0, 0.0],
                [100.0, 0.0],
                [100.0, -30.0],
                [200.0, -30.0],
                [200.0, -60.0],
                [375.0, -60.0],
            ]],
        );
        assert_eq!(compute(&s, &stairs).staircase_steps, 1);
        let detour = layout(
            vec![[0.0, 0.0], [400.0, 0.0]],
            vec![vec![
                [25.0, 0.0],
                [100.0, 0.0],
                [100.0, -30.0],
                [200.0, -30.0],
                [200.0, 0.0],
                [375.0, 0.0],
            ]],
        );
        assert_eq!(compute(&s, &detour).staircase_steps, 0);
        let one_jog = layout(
            vec![[0.0, 0.0], [400.0, 0.0]],
            vec![vec![
                [25.0, 0.0],
                [200.0, 0.0],
                [200.0, -60.0],
                [375.0, -60.0],
            ]],
        );
        assert_eq!(compute(&s, &one_jog).staircase_steps, 0);
    }

    /// A self-loop reverses by construction and is left out.
    #[test]
    fn self_loops_do_not_count_as_steps_or_reversals() {
        let s = spec(boxes(1), vec![(0, 0)], "LR");
        let l = layout(
            vec![[0.0, 0.0]],
            vec![vec![
                [25.0, 0.0],
                [40.0, 0.0],
                [40.0, -30.0],
                [0.0, -30.0],
                [0.0, -25.0],
            ]],
        );
        let m = compute(&s, &l);
        assert_eq!(
            (m.short_steps, m.route_reversals, m.staircase_steps),
            (0, 0, 0)
        );
    }

    /// A straight chain is a straight main path, with every node on the median line.
    #[test]
    fn a_straight_chain_is_a_straight_main_path() {
        let (g, l) = chain(
            &[[0.0, 0.0], [0.0, 100.0], [0.0, 200.0], [0.0, 300.0]],
            &[1.0, 1.0, 1.0],
        );
        let m = measure(&g, &tb(), &l);
        assert_eq!(m.main_path_len, 4);
        assert_eq!(m.main_path_aligned_fraction, 1.0);
        assert_eq!(m.main_path_deviation, 0.0);
    }

    /// The deviation is the mean offset over the path's extent along the layout axis.
    #[test]
    fn a_kinked_chain_loses_the_node_it_kinked() {
        let (g, l) = chain(
            &[[0.0, 0.0], [60.0, 100.0], [0.0, 200.0], [0.0, 300.0]],
            &[1.0, 1.0, 1.0],
        );
        let m = measure(&g, &tb(), &l);
        assert_eq!(m.main_path_len, 4);
        assert_eq!(m.main_path_aligned_fraction, 0.75);
        // Mean |cross - 0| is 60/4 = 15 over an extent of 300.
        assert!((m.main_path_deviation - 15.0 / 300.0).abs() < 1e-9, "{m:?}");
    }

    /// The two-hop heavy branch beats the three-hop light one.
    #[test]
    fn the_main_path_follows_the_weight() {
        let size = Size::new(60.0, 30.0);
        let node = Node {
            size,
            ..Node::default()
        };
        // 0 -> 1 -> 4 has weight 20 per edge, 0 -> 2 -> 3 -> 4 has weight 1 per edge.
        let graph = Graph {
            nodes: vec![node.clone(), node.clone(), node.clone(), node.clone(), node],
            edges: vec![
                Edge {
                    weight: 20.0,
                    ..Edge::new(NodeId(0), NodeId(1))
                },
                Edge {
                    weight: 20.0,
                    ..Edge::new(NodeId(1), NodeId(4))
                },
                Edge::new(NodeId(0), NodeId(2)),
                Edge::new(NodeId(2), NodeId(3)),
                Edge::new(NodeId(3), NodeId(4)),
            ],
        };
        // The heavy branch is drawn straight and the light one wanders.
        let centers = [
            [0.0, 0.0],
            [0.0, 100.0],
            [200.0, 100.0],
            [200.0, 200.0],
            [0.0, 300.0],
        ];
        let placements: Vec<Placement> = centers
            .iter()
            .map(|c| Placement {
                center: Point::new(c[0], c[1]),
                size,
                ..Placement::default()
            })
            .collect();
        let route = |a: usize, b: usize| EdgeRoute {
            points: vec![
                Point::new(centers[a][0], centers[a][1]),
                Point::new(centers[b][0], centers[b][1]),
            ],
            ..EdgeRoute::default()
        };
        let l = Layout {
            nodes: placements,
            edges: vec![
                route(0, 1),
                route(1, 4),
                route(0, 2),
                route(2, 3),
                route(3, 4),
            ],
            ..Layout::default()
        };
        let m = measure(&graph, &tb(), &l);
        assert_eq!(m.main_path_len, 3);
        assert_eq!(m.main_path_aligned_fraction, 1.0);
    }

    /// An edge the cycle breaker reversed is followed the way the engine ranked it.
    #[test]
    fn the_main_path_follows_the_layerings_orientation() {
        let at = [[0.0, 0.0], [0.0, 100.0], [0.0, 200.0], [0.0, 300.0]];
        let (mut g, mut l) = chain(&at, &[1.0, 1.0, 1.0]);
        // The middle edge points backwards but the layering reversed it, so 0 -> 1 -> 2 -> 3 is
        // still one path.
        g.edges[1] = Edge::new(NodeId(2), NodeId(1));
        l.reversed = vec![EdgeId(1)];
        let m = measure(&g, &tb(), &l);
        assert_eq!(m.main_path_len, 4);

        // Without `reversed` no path is longer than one edge.
        let (mut g2, l2) = chain(&at, &[1.0, 1.0, 1.0]);
        g2.edges[1] = Edge::new(NodeId(2), NodeId(1));
        assert_eq!(measure(&g2, &tb(), &l2).main_path_len, 2);
    }

    /// The path does not end at a heavy node whose only out-edge loops back.
    #[test]
    fn the_main_path_ends_at_the_models_end() {
        // 0 -> 1, then 1 -> 2 (heavy) and 1 -> 3. The reversed 2 -> 1 makes 2 an oriented
        // sink. 3 is the true sink.
        let (mut g, mut l) = chain(&[[0.0, 0.0], [0.0, 100.0]], &[1.0]);
        let size = Size::new(60.0, 30.0);
        for c in [[200.0, 200.0], [0.0, 200.0]] {
            g.nodes.push(Node {
                size,
                ..Node::default()
            });
            l.nodes.push(Placement {
                center: Point::new(c[0], c[1]),
                size,
                ..Placement::default()
            });
        }
        let straight = |a: [f64; 2], b: [f64; 2]| EdgeRoute {
            points: vec![Point::new(a[0], a[1] + 15.0), Point::new(b[0], b[1] - 15.0)],
            ..EdgeRoute::default()
        };
        g.edges.push(Edge {
            weight: 5.0,
            ..Edge::new(NodeId(1), NodeId(2))
        });
        l.edges.push(straight([0.0, 100.0], [200.0, 200.0]));
        g.edges.push(Edge::new(NodeId(2), NodeId(1)));
        l.edges.push(straight([200.0, 200.0], [0.0, 100.0]));
        g.edges.push(Edge::new(NodeId(1), NodeId(3)));
        l.edges.push(straight([0.0, 100.0], [0.0, 200.0]));
        l.reversed = vec![EdgeId(2)];
        let m = measure(&g, &tb(), &l);
        // 0 -> 1 -> 3 runs straight down, while 0 -> 1 -> 2 would bend off to the side.
        assert_eq!(m.main_path_len, 3);
        assert_eq!(m.main_path_aligned_fraction, 1.0, "{m:?}");
    }

    /// An empty drawing reports an aligned fraction of 1, so it is not charged for a missing path.
    #[test]
    fn an_empty_graph_has_no_main_path() {
        let m = measure(&Graph::new(), &tb(), &Layout::default());
        assert_eq!(m.main_path_len, 0);
        assert_eq!(m.main_path_aligned_fraction, 1.0);
        assert_eq!(m.main_path_deviation, 0.0);
    }

    /// The minimum each border pair allows: straight across a shared height, a Z without one, an
    /// L between perpendicular borders, a U back into the side it left.
    #[test]
    fn min_orthogonal_bends_reads_the_two_borders() {
        use super::super::NodeBox;
        use super::min_orthogonal_bends;
        let at = |cx: f64, cy: f64| NodeBox {
            cx,
            cy,
            hw: 20.0,
            hh: 10.0,
            ellipse: false,
        };
        let right = (0usize, 1.0);
        let down = (1usize, 1.0);
        let left = (0usize, -1.0);
        // Right border to left border with overlapping heights is straight.
        assert_eq!(
            min_orthogonal_bends(&at(0.0, 0.0), right, &at(100.0, 5.0), right),
            0
        );
        // One row lower it is a Z.
        assert_eq!(
            min_orthogonal_bends(&at(0.0, 0.0), right, &at(100.0, 80.0), right),
            2
        );
        // Target behind the source.
        assert_eq!(
            min_orthogonal_bends(&at(100.0, 0.0), right, &at(0.0, 80.0), right),
            4
        );
        // Out of the right border into the top of a node below and to the right is an L.
        assert_eq!(
            min_orthogonal_bends(&at(0.0, 0.0), right, &at(100.0, 80.0), down),
            1
        );
        // Into the top of a node above, so no L.
        assert_eq!(
            min_orthogonal_bends(&at(0.0, 80.0), right, &at(100.0, 0.0), down),
            3
        );
        // Out of the right border and back in through the target's right border is a U.
        assert_eq!(
            min_orthogonal_bends(&at(0.0, 0.0), right, &at(0.0, 80.0), left),
            2
        );
    }
}

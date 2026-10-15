//! Crossings, near-parallel runs and flow direction.
//!
//! [`fill_segment_pair_metrics`] is one O(S^2) pass over all segment pairs of different edges. It
//! counts crossings and overlaps, the weighted crossing sums, the crossing angles and the
//! near-parallel run length. [`fill_flow_consistency`] implements Burattin et al.'s `M-E2`.

use super::{
    collinear_overlap_len, cross, distance, proper_intersection, EdgeGeom, LayoutMetrics, NodeBox,
    Problem, BACKWARD_TOL, OVERLAP_MIN_LEN, PARALLEL_SEPARATION_K, PARALLEL_TOL_DEG,
    SHALLOW_CROSSING_DEG, SHARED_ENDPOINT_TOL,
};
use crate::geometry::fp;
use crate::geometry::{DEGENERATE, EPS};
use crate::sort::SortDyn;

/// Edges whose target does not lie after its source along the layout axis.
pub(super) fn count_backward_edges(problem: &Problem, boxes: &[NodeBox]) -> usize {
    let axis = |b: &NodeBox| if problem.left_right { b.cx } else { b.cy };
    problem
        .edges
        .iter()
        .filter(|(a, b)| a != b)
        .filter(|&&(a, b)| match (boxes.get(a), boxes.get(b)) {
            (Some(sa), Some(sb)) => axis(sb) <= axis(sa) + BACKWARD_TOL,
            _ => false,
        })
        .count()
}

/// Counts [`LayoutMetrics::backward_nonloop_edges`], the backward edges whose target lies more
/// breadth-first hops from the sources than their source does.
pub(super) fn count_backward_nonloop_edges(problem: &Problem, boxes: &[NodeBox]) -> usize {
    let n = problem.node_count;
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut has_in = vec![false; n];
    for &(a, b) in problem
        .edges
        .iter()
        .filter(|(a, b)| a != b && *a < n && *b < n)
    {
        out[a].push(b);
        has_in[b] = true;
    }
    let mut depth = vec![usize::MAX; n];
    let mut queue = std::collections::VecDeque::new();
    for v in (0..n).filter(|&v| !has_in[v] && !out[v].is_empty()) {
        depth[v] = 0;
        queue.push_back(v);
    }
    while let Some(v) = queue.pop_front() {
        for &u in &out[v] {
            if depth[u] == usize::MAX {
                depth[u] = depth[v] + 1;
                queue.push_back(u);
            }
        }
    }
    let axis = |b: &NodeBox| if problem.left_right { b.cx } else { b.cy };
    problem
        .edges
        .iter()
        .filter(|&&(a, b)| a != b && a < n && b < n)
        .filter(|&&(a, b)| depth[a] != usize::MAX && depth[b] != usize::MAX && depth[b] > depth[a])
        .filter(|&&(a, b)| match (boxes.get(a), boxes.get(b)) {
            (Some(sa), Some(sb)) => axis(sb) <= axis(sa) + BACKWARD_TOL,
            _ => false,
        })
        .count()
}

/// Fills [`LayoutMetrics::crossings`], [`LayoutMetrics::overlaps`], the weighted crossing sums,
/// the crossing angles and [`LayoutMetrics::parallel_run_length`].
pub(super) fn fill_segment_pair_metrics(edges: &[EdgeGeom], m: &mut LayoutMetrics) {
    let mut angle_sum = 0.0;
    let mut angle_min: Option<f64> = None;
    for (i, e1) in edges.iter().enumerate() {
        for e2 in edges.iter().skip(i + 1) {
            let anchors = shared_anchors(e1, e2);
            let trunk = converged_segments(e1, e2);
            // Two strokes closer than this read as one thick line.
            let separation = PARALLEL_SEPARATION_K * e1.thickness.max(e2.thickness);
            for (k1, s1) in e1.pts.windows(2).enumerate() {
                for (k2, s2) in e2.pts.windows(2).enumerate() {
                    // Ends converged into one port share their end segments by design.
                    if trunk.contains(&(k1, k2)) {
                        continue;
                    }
                    if let Some(p) = proper_intersection(s1[0], s1[1], s2[0], s2[1]) {
                        if !anchors
                            .iter()
                            .any(|a| distance(*a, p) <= SHARED_ENDPOINT_TOL)
                        {
                            m.crossings += 1;
                            m.weighted_crossings += e1.weight * e2.weight;
                            m.thickness_weighted_crossings += e1.thickness * e2.thickness;
                            let angle = acute_angle_deg(s1[0], s1[1], s2[0], s2[1]);
                            angle_sum += angle;
                            angle_min = Some(angle_min.map_or(angle, |a: f64| a.min(angle)));
                            if angle < SHALLOW_CROSSING_DEG {
                                m.crossings_below_30deg += 1;
                            }
                        }
                    } else if collinear_overlap_len(s1[0], s1[1], s2[0], s2[1]) > OVERLAP_MIN_LEN {
                        m.overlaps += 1;
                    }
                    m.parallel_run_length +=
                        parallel_run_len(s1[0], s1[1], s2[0], s2[1], separation);
                }
            }
        }
    }
    m.crossing_angle_min = angle_min;
    m.crossing_angle_mean = (m.crossings > 0).then(|| angle_sum / m.crossings as f64);
}

/// `(segment of e1, segment of e2)` end-segment pairs whose ends meet at the same point of one
/// node. Empty when the edges converge at both ends, since that is one line drawn twice.
fn converged_segments(e1: &EdgeGeom, e2: &EdgeGeom) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut ends_of_e1 = [false; 2];
    for (k, (n1, p1)) in e1.ends().into_iter().enumerate() {
        for (j, (n2, p2)) in e2.ends().into_iter().enumerate() {
            if n1 != n2 || distance(p1, p2) > SHARED_ENDPOINT_TOL {
                continue;
            }
            ends_of_e1[k] = true;
            // A merged trunk can be several segments long, so walk inward from the shared end
            // while the vertices coincide.
            let (mut i1, d1) = if k == 0 {
                (0, 1isize)
            } else {
                (e1.pts.len() - 1, -1)
            };
            let (mut i2, d2) = if j == 0 {
                (0, 1isize)
            } else {
                (e2.pts.len() - 1, -1)
            };
            let step = |i: usize, d: isize| i.checked_add_signed(d);
            while let (Some(n1), Some(n2)) = (step(i1, d1), step(i2, d2)) {
                let (Some(q1), Some(q2)) = (e1.pts.get(n1), e2.pts.get(n2)) else {
                    break;
                };
                out.push((i1.min(n1), i2.min(n2)));
                if distance(*q1, *q2) > SHARED_ENDPOINT_TOL {
                    break;
                }
                i1 = n1;
                i2 = n2;
            }
        }
    }
    if ends_of_e1 == [true, true] {
        out.clear();
    }
    out
}

/// Attachment points at shared nodes, where an intersection is not a crossing.
fn shared_anchors(e1: &EdgeGeom, e2: &EdgeGeom) -> Vec<[f64; 2]> {
    let mut out = Vec::new();
    for (n1, p1) in e1.ends() {
        for (n2, p2) in e2.ends() {
            if n1 == n2 {
                out.push(p1);
                out.push(p2);
            }
        }
    }
    out
}

/// The acute angle in degrees between two segments. A zero-length segment gives `90`, which
/// costs nothing.
fn acute_angle_deg(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> f64 {
    let (Some(u), Some(v)) = (unit(a, b), unit(c, d)) else {
        return 90.0;
    };
    let dot = (u[0] * v[0] + u[1] * v[1]).abs().clamp(0.0, 1.0);
    dot.acos().to_degrees()
}

fn unit(a: [f64; 2], b: [f64; 2]) -> Option<[f64; 2]> {
    let d = [b[0] - a[0], b[1] - a[1]];
    let len = fp::hypot(d[0], d[1]);
    (len >= EPS).then(|| [d[0] / len, d[1] / len])
}

/// Length over which `c->d` runs within [`PARALLEL_TOL_DEG`] of parallel to `a->b` and closer
/// than `separation` px. The offset is linear along `a->b`, so the interval is solved exactly.
fn parallel_run_len(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2], separation: f64) -> f64 {
    if separation <= 0.0 {
        return 0.0;
    }
    let (Some(u), Some(v)) = (unit(a, b), unit(c, d)) else {
        return 0.0;
    };
    let dot = (u[0] * v[0] + u[1] * v[1]).abs().clamp(0.0, 1.0);
    if dot.acos().to_degrees() >= PARALLEL_TOL_DEG {
        return 0.0;
    }
    let len = distance(a, b);
    let along = |p: [f64; 2]| u[0] * (p[0] - a[0]) + u[1] * (p[1] - a[1]);
    let off = |p: [f64; 2]| cross(u, [p[0] - a[0], p[1] - a[1]]);
    let (tc, td) = (along(c), along(d));
    let (oc, od) = (off(c), off(d));
    let mut lo = tc.min(td).max(0.0);
    let mut hi = tc.max(td).min(len);
    if hi <= lo {
        return 0.0;
    }
    if (td - tc).abs() < EPS {
        return if oc.abs() < separation { hi - lo } else { 0.0 };
    }
    let slope = (od - oc) / (td - tc);
    if slope.abs() < DEGENERATE {
        return if oc.abs() < separation { hi - lo } else { 0.0 };
    }
    let t1 = tc + (-separation - oc) / slope;
    let t2 = tc + (separation - oc) / slope;
    lo = lo.max(t1.min(t2));
    hi = hi.min(t1.max(t2));
    (hi - lo).max(0.0)
}

/// North, east, south and west, in tie-break order. North is `-y` because `y` grows down.
const COMPASS: [[f64; 2]; 4] = [[0.0, -1.0], [1.0, 0.0], [0.0, 1.0], [-1.0, 0.0]];
/// The six unordered pairs of [`COMPASS`] directions, in a fixed order for deterministic ties.
const COMPASS_PAIRS: [(usize, usize); 6] = [(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)];

/// Burattin et al.'s `M-E2` and its weighted variant.
///
/// Each segment is labelled with the two compass directions nearest its heading, which keeps
/// diagonal segments from splitting the vote. The result is the share of drawn length in the
/// majority pair. The weighted variant multiplies each length by its edge's weight.
pub(super) fn fill_flow_consistency(edges: &[EdgeGeom], m: &mut LayoutMetrics) {
    let mut by_pair = [0.0f64; 6];
    let mut by_pair_weighted = [0.0f64; 6];
    let mut total = 0.0;
    let mut total_weighted = 0.0;
    for e in edges {
        for s in e.pts.windows(2) {
            let len = distance(s[0], s[1]);
            let Some(dir) = unit(s[0], s[1]) else {
                continue;
            };
            let slot = pair_slot(nearest_two(dir));
            by_pair[slot] += len;
            by_pair_weighted[slot] += len * e.weight;
            total += len;
            total_weighted += len * e.weight;
        }
    }
    m.flow_consistency = majority_share(&by_pair, total);
    m.flow_consistency_weighted = majority_share(&by_pair_weighted, total_weighted);
}

fn nearest_two(dir: [f64; 2]) -> (usize, usize) {
    let mut order: Vec<usize> = (0..4).collect();
    // The compass index breaks ties, e.g., for a heading at exactly 45 degrees.
    order.sort_unstable_by_dyn(|&a, &b| {
        let da = COMPASS[a][0] * dir[0] + COMPASS[a][1] * dir[1];
        let db = COMPASS[b][0] * dir[0] + COMPASS[b][1] * dir[1];
        db.total_cmp(&da).then(a.cmp(&b))
    });
    let (a, b) = (order[0], order[1]);
    (a.min(b), a.max(b))
}

fn pair_slot(pair: (usize, usize)) -> usize {
    COMPASS_PAIRS
        .iter()
        .position(|&p| p == pair)
        .unwrap_or_default()
}

/// Share of `total` in the largest bucket, `0.0` when nothing was drawn.
fn majority_share(buckets: &[f64; 6], total: f64) -> f64 {
    if total <= 0.0 {
        return 0.0;
    }
    let best = buckets.iter().copied().fold(0.0f64, f64::max);
    (best / total).clamp(0.0, 1.0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::super::tests::compute;
    use super::super::tests::{boxes, layout, spec};
    use super::super::{measure, LayoutMetrics};
    use crate::geometry::fp;
    use crate::geometry::{Point, Size};
    use crate::layout::{EdgeRoute, Layout, Placement};
    use crate::{Direction, Edge, Graph, LayoutOptions, Node, NodeId};

    fn two_node_layout(a: [f64; 2], b: [f64; 2], pts: Vec<[f64; 2]>) -> (Graph, Layout) {
        let node = Node {
            size: Size::new(100.0, 50.0),
            ..Node::default()
        };
        let graph = Graph {
            nodes: vec![node.clone(), node],
            edges: vec![Edge::new(NodeId(0), NodeId(1))],
        };
        let placement = |p: [f64; 2]| Placement {
            center: Point::new(p[0], p[1]),
            size: Size::new(100.0, 50.0),
            ..Placement::default()
        };
        let layout = Layout {
            nodes: vec![placement(a), placement(b)],
            edges: vec![EdgeRoute {
                points: pts.into_iter().map(|p| Point::new(p[0], p[1])).collect(),
                ..EdgeRoute::default()
            }],
            ..Layout::default()
        };
        (graph, layout)
    }

    #[test]
    fn x_shape_is_one_crossing() {
        let s = spec(boxes(4), vec![(0, 1), (2, 3)], "TB");
        let l = layout(
            vec![[0.0, 0.0], [400.0, 400.0], [400.0, 0.0], [0.0, 400.0]],
            vec![
                vec![[100.0, 100.0], [300.0, 300.0]],
                vec![[300.0, 100.0], [100.0, 300.0]],
            ],
        );
        let m = compute(&s, &l);
        assert_eq!(m.crossings, 1);
        assert_eq!(m.overlaps, 0);
    }

    /// Two merged edges share a trunk of several segments out of their source. The trunk is
    /// neither a crossing nor an overlap.
    #[test]
    fn a_merged_trunk_is_not_an_overlap() {
        let s = spec(boxes(3), vec![(0, 1), (0, 2)], "TB");
        let trunk = [[0.0, 25.0], [0.0, 60.0], [30.0, 60.0], [30.0, 100.0]];
        let mut left = trunk.to_vec();
        left.extend([[-100.0, 100.0], [-100.0, 175.0]]);
        let mut right = trunk.to_vec();
        right.extend([[100.0, 100.0], [100.0, 175.0]]);
        let l = layout(
            vec![[0.0, 0.0], [-100.0, 200.0], [100.0, 200.0]],
            vec![left, right],
        );
        let m = compute(&s, &l);
        assert_eq!((m.crossings, m.overlaps), (0, 0), "{m:?}");
    }

    #[test]
    fn shared_endpoint_is_not_a_crossing() {
        let s = spec(boxes(3), vec![(0, 1), (0, 2)], "TB");
        let l = layout(
            vec![[0.0, 0.0], [-200.0, 200.0], [200.0, 200.0]],
            vec![
                vec![[0.0, 25.0], [-200.0, 175.0]],
                vec![[0.0, 25.0], [200.0, 175.0]],
            ],
        );
        let m = compute(&s, &l);
        assert_eq!(m.crossings, 0);
    }

    #[test]
    fn same_source_edges_can_still_cross() {
        let s = spec(boxes(3), vec![(0, 1), (0, 2)], "TB");
        let l = layout(
            vec![[0.0, 0.0], [200.0, 400.0], [-200.0, 400.0]],
            vec![
                vec![[-20.0, 25.0], [200.0, 375.0]],
                vec![[20.0, 25.0], [-200.0, 375.0]],
            ],
        );
        assert_eq!(compute(&s, &l).crossings, 1);
    }

    #[test]
    fn collinear_duplicate_is_an_overlap() {
        let s = spec(boxes(4), vec![(0, 1), (2, 3)], "TB");
        let l = layout(
            vec![[0.0, 0.0], [0.0, 400.0], [500.0, 0.0], [500.0, 400.0]],
            vec![
                vec![[0.0, 25.0], [0.0, 375.0]],
                vec![[0.0, 100.0], [0.0, 300.0]],
            ],
        );
        let m = compute(&s, &l);
        assert_eq!(m.crossings, 0);
        assert_eq!(m.overlaps, 1);
    }

    #[test]
    fn backward_edges_follow_direction() {
        let tb = spec(boxes(3), vec![(0, 1), (1, 0), (2, 2)], "TB");
        let l = layout(
            vec![[0.0, 0.0], [0.0, 200.0], [300.0, 300.0]],
            vec![
                vec![[0.0, 25.0], [0.0, 175.0]],
                vec![[0.0, 175.0], [0.0, 25.0]],
                vec![],
            ],
        );
        let m = compute(&tb, &l);
        assert_eq!(m.backward_edges, 1);
        assert_eq!(m.self_loops, 1);
        assert_eq!(m.unrouted_edges, 1);

        // Read left-to-right, neither edge advances on x.
        let lr = spec(boxes(3), vec![(0, 1), (1, 0), (2, 2)], "LR");
        assert_eq!(compute(&lr, &l).backward_edges, 2);
    }

    /// A loop's backward closing edge is fine, but a loop drawn with its body before its entry is
    /// counted.
    #[test]
    fn a_loop_drawn_inside_out_is_counted() {
        // 0 -> 1 -> 2 -> 1, with node 1 the loop's entry and 2 its body.
        let s = spec(boxes(3), vec![(0, 1), (1, 2), (2, 1)], "TB");
        let straight = |ys: [f64; 3]| {
            layout(
                vec![[0.0, ys[0]], [0.0, ys[1]], [0.0, ys[2]]],
                vec![
                    vec![[0.0, ys[0]], [0.0, ys[1]]],
                    vec![[0.0, ys[1]], [0.0, ys[2]]],
                    vec![[0.0, ys[2]], [0.0, ys[1]]],
                ],
            )
        };
        let natural = compute(&s, &straight([0.0, 200.0, 400.0]));
        assert_eq!(
            (natural.backward_edges, natural.backward_nonloop_edges),
            (1, 0)
        );
        let inside_out = compute(&s, &straight([0.0, 400.0, 200.0]));
        assert_eq!(
            (inside_out.backward_edges, inside_out.backward_nonloop_edges),
            (1, 1)
        );
    }

    /// A crossing costs the product of the two edges' weights, and of their thicknesses.
    #[test]
    fn crossings_are_weighted_by_both_edges() {
        let node = Node {
            size: Size::new(40.0, 20.0),
            ..Node::default()
        };
        let graph = Graph {
            nodes: vec![node.clone(), node.clone(), node.clone(), node],
            edges: vec![
                Edge {
                    weight: 3.0,
                    thickness: 4.0,
                    ..Edge::new(NodeId(0), NodeId(1))
                },
                Edge {
                    weight: 5.0,
                    thickness: 2.0,
                    ..Edge::new(NodeId(2), NodeId(3))
                },
            ],
        };
        let place = |x: f64, y: f64| Placement {
            center: Point::new(x, y),
            size: Size::new(40.0, 20.0),
            ..Placement::default()
        };
        let route = |a: [f64; 2], b: [f64; 2]| EdgeRoute {
            points: vec![Point::new(a[0], a[1]), Point::new(b[0], b[1])],
            ..EdgeRoute::default()
        };
        let l = Layout {
            nodes: vec![
                place(0.0, 0.0),
                place(400.0, 400.0),
                place(400.0, 0.0),
                place(0.0, 400.0),
            ],
            edges: vec![
                route([100.0, 100.0], [300.0, 300.0]),
                route([300.0, 100.0], [100.0, 300.0]),
            ],
            ..Layout::default()
        };
        let m = measure(&graph, &LayoutOptions::default(), &l);
        assert_eq!(m.crossings, 1);
        assert_eq!(m.weighted_crossings, 15.0);
        assert_eq!(m.thickness_weighted_crossings, 8.0);
        assert_eq!(m.crossing_angle_min, Some(90.0));
        assert_eq!(m.crossing_angle_mean, Some(90.0));
        assert_eq!(m.crossings_below_30deg, 0);
    }

    #[test]
    fn shallow_crossings_are_reported_by_angle() {
        let s = spec(boxes(4), vec![(0, 1), (2, 3)], "TB");
        let l = layout(
            vec![[0.0, 0.0], [400.0, 0.0], [0.0, 100.0], [400.0, 100.0]],
            vec![
                vec![[0.0, 0.0], [400.0, 20.0]],
                vec![[0.0, 20.0], [400.0, 0.0]],
            ],
        );
        let m = compute(&s, &l);
        assert_eq!(m.crossings, 1);
        assert_eq!(m.crossings_below_30deg, 1);
        let angle = m.crossing_angle_min.unwrap();
        assert!((0.0..30.0).contains(&angle), "{angle}");
    }

    #[test]
    fn parallel_runs_accumulate_only_while_close() {
        let s = spec(boxes(4), vec![(0, 1), (2, 3)], "TB");
        let close = layout(
            vec![[0.0, 0.0], [0.0, 400.0], [3.0, 0.0], [3.0, 400.0]],
            vec![
                vec![[0.0, 100.0], [0.0, 300.0]],
                vec![[2.0, 100.0], [2.0, 300.0]],
            ],
        );
        let m = compute(&s, &close);
        // Default thickness 2px gives a 3px separation limit.
        assert!((m.parallel_run_length - 200.0).abs() < 1e-6, "{m:?}");
        let apart = layout(
            vec![[0.0, 0.0], [0.0, 400.0], [80.0, 0.0], [80.0, 400.0]],
            vec![
                vec![[0.0, 100.0], [0.0, 300.0]],
                vec![[80.0, 100.0], [80.0, 300.0]],
            ],
        );
        assert_eq!(compute(&s, &apart).parallel_run_length, 0.0);
    }

    #[test]
    fn flow_consistency_is_the_majority_share_of_length() {
        let (g, l) = two_node_layout([0.0, 0.0], [0.0, 400.0], vec![[0.0, 25.0], [0.0, 375.0]]);
        let straight = measure(&g, &LayoutOptions::default(), &l);
        assert_eq!(straight.flow_consistency, 1.0);
        assert_eq!(straight.flow_consistency_weighted, 1.0);

        // Three quarters south-east and one quarter north-east.
        let (g2, l2) = two_node_layout(
            [0.0, 0.0],
            [0.0, 400.0],
            vec![[0.0, 0.0], [300.0, 300.0], [400.0, 200.0]],
        );
        let bent = measure(&g2, &LayoutOptions::default(), &l2);
        let share =
            fp::hypot(300.0f64, 300.0) / (fp::hypot(300.0f64, 300.0) + fp::hypot(100.0f64, 100.0));
        assert!((bent.flow_consistency - share).abs() < 1e-9, "{bent:?}");
    }

    /// The majority direction comes from the drawing, not from the layout direction.
    #[test]
    fn flow_consistency_finds_its_own_axis() {
        let (g, l) = two_node_layout([0.0, 0.0], [400.0, 0.0], vec![[50.0, 0.0], [350.0, 0.0]]);
        let opts = LayoutOptions {
            direction: Direction::LeftRight,
            ..LayoutOptions::default()
        };
        assert_eq!(measure(&g, &opts, &l).flow_consistency, 1.0);
    }

    #[test]
    fn flow_consistency_of_an_empty_drawing_is_zero() {
        let m = LayoutMetrics::default();
        assert_eq!(m.flow_consistency, 0.0);
        let empty = measure(&Graph::new(), &LayoutOptions::default(), &Layout::default());
        assert_eq!(empty.flow_consistency, 0.0);
        assert_eq!(empty.crossing_angle_min, None);
    }
}

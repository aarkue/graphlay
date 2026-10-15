//! Node placement metrics: node and edge clearances, bounding box and density, fork/join blocks,
//! group lanes and whitespace balance.

use super::{
    box_gap, box_penetration, segment_box_distance, segment_clip_box, segment_hits_node, EdgeGeom,
    LayoutMetrics, NodeBox, Problem, FIT_VIEWPORT, LABEL_FONT_PX, NODE_OVERLAP_TOL,
};
use crate::geometry::EPS;
use crate::sort::SortDyn;
use crate::splits;

/// `(node_edge_hits, min_edge_node_clearance)` over all (edge, node) pairs where the node is not
/// an end of the edge and not a compound enclosing one.
pub(super) fn node_edge_interaction(
    edges: &[EdgeGeom],
    boxes: &[NodeBox],
    parents: &[Option<usize>],
) -> (usize, Option<f64>) {
    let mut hits = 0;
    let mut clearance: Option<f64> = None;
    for e in edges {
        for (ni, nb) in boxes.iter().enumerate() {
            if super::encloses(parents, ni, e.from) || super::encloses(parents, ni, e.to) {
                continue;
            }
            let mut hit = false;
            let mut best = f64::INFINITY;
            for s in e.pts.windows(2) {
                if segment_hits_node(s[0], s[1], nb) {
                    hit = true;
                    break;
                }
                best = best.min(segment_box_distance(s[0], s[1], nb));
            }
            if hit {
                hits += 1;
            }
            let d = if hit { 0.0 } else { best };
            if d.is_finite() {
                clearance = Some(clearance.map_or(d, |c: f64| c.min(d)));
            }
        }
    }
    (hits, clearance)
}

/// `(overlapping node pairs, min signed gap)` over pairs where neither node is a compound holding
/// the other.
pub(super) fn node_pair_metrics(
    boxes: &[NodeBox],
    parents: &[Option<usize>],
) -> (usize, Option<f64>) {
    let mut overlaps = 0;
    let mut min_gap: Option<f64> = None;
    for (i, a) in boxes.iter().enumerate() {
        for (j, b) in boxes.iter().enumerate().skip(i + 1) {
            if super::encloses(parents, i, j) || super::encloses(parents, j, i) {
                continue;
            }
            let (ox, oy) = box_penetration(a, b);
            if ox > NODE_OVERLAP_TOL && oy > NODE_OVERLAP_TOL {
                overlaps += 1;
            }
            let gap = box_gap(a, b);
            min_gap = Some(min_gap.map_or(gap, |g: f64| g.min(gap)));
        }
    }
    (overlaps, min_gap)
}

/// Fills `fit_font_px`, `density` and `aspect_penalty`. Needs [`fill_bounding_box`] first.
pub(super) fn fill_perceived_metrics(boxes: &[NodeBox], m: &mut LayoutMetrics) {
    if m.width <= 0.0 || m.height <= 0.0 {
        return;
    }
    let scale = (FIT_VIEWPORT.0 / m.width)
        .min(FIT_VIEWPORT.1 / m.height)
        .min(1.0);
    m.fit_font_px = LABEL_FONT_PX * scale;
    let ink: f64 = boxes.iter().map(|b| 4.0 * b.hw * b.hh).sum();
    // Overlapping nodes could push the ratio past 1.
    m.density = (ink / m.area).clamp(0.0, 1.0);
    let aspect = m.width / m.height;
    m.aspect_penalty = aspect.max(1.0 / aspect);
}

/// `(fork/join pairs, aligned pairs)`, as [`crate::splits`] defines them.
pub(super) fn split_alignment(problem: &Problem, boxes: &[NodeBox]) -> (usize, usize) {
    let n = boxes.len().min(problem.node_count);
    let adj = splits::successors(n, problem.edges.iter().copied());
    let pairs = splits::find_pairs(&adj);
    // The predicate takes `(along, cross)`.
    let pos = |i: usize| {
        let b = &boxes[i];
        if problem.left_right {
            (b.cx, b.cy)
        } else {
            (b.cy, b.cx)
        }
    };
    let aligned = pairs.iter().filter(|p| splits::is_aligned(pos, p)).count();
    (pairs.len(), aligned)
}

/// Fills width/height/area/aspect over node boxes and route points.
pub(super) fn fill_bounding_box(boxes: &[NodeBox], edges: &[EdgeGeom], m: &mut LayoutMetrics) {
    let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
    let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for b in boxes {
        min_x = min_x.min(b.left());
        max_x = max_x.max(b.right());
        min_y = min_y.min(b.top());
        max_y = max_y.max(b.bottom());
    }
    for e in edges {
        for p in &e.pts {
            min_x = min_x.min(p[0]);
            max_x = max_x.max(p[0]);
            min_y = min_y.min(p[1]);
            max_y = max_y.max(p[1]);
        }
    }
    if !min_x.is_finite() || !min_y.is_finite() {
        return;
    }
    m.width = max_x - min_x;
    m.height = max_y - min_y;
    m.area = m.width * m.height;
    m.aspect_ratio = if m.height > 0.0 {
        m.width / m.height
    } else {
        0.0
    };
}

/// Foreign edge segments crossing the bounding box of a fork/join block's four nodes.
///
/// [`LayoutMetrics::split_alignment`] only looks at four node centres and misses an edge drawn
/// through the block. An edge is foreign when neither endpoint is a member, and each crossing
/// segment counts once.
pub(super) fn block_intrusion(problem: &Problem, boxes: &[NodeBox], edges: &[EdgeGeom]) -> usize {
    let n = boxes.len().min(problem.node_count);
    let adj = splits::successors(n, problem.edges.iter().copied());
    let pairs = splits::find_pairs(&adj);
    let mut count = 0;
    for p in &pairs {
        let members = [p.fork, p.b1, p.b2, p.join];
        let Some((left, right, top, bottom)) = member_bounds(&members, boxes) else {
            continue;
        };
        for e in edges {
            if members.contains(&e.from) || members.contains(&e.to) {
                continue;
            }
            for s in e.pts.windows(2) {
                if segment_clip_box(s[0], s[1], left, right, top, bottom).is_some() {
                    count += 1;
                }
            }
        }
    }
    count
}

/// `(left, right, top, bottom)` of the union of the member nodes' boxes.
fn member_bounds(members: &[usize], boxes: &[NodeBox]) -> Option<(f64, f64, f64, f64)> {
    let mut out: Option<(f64, f64, f64, f64)> = None;
    for &i in members {
        let b = boxes.get(i)?;
        out = Some(match out {
            None => (b.left(), b.right(), b.top(), b.bottom()),
            Some((l, r, t, bo)) => (
                l.min(b.left()),
                r.max(b.right()),
                t.min(b.top()),
                bo.max(b.bottom()),
            ),
        });
    }
    out
}

/// How well each [`crate::Node::group`] keeps its own lane.
///
/// [`LayoutMetrics::lane_purity`] is the mean share of a group's cross-axis span that no other
/// group covers. [`LayoutMetrics::lane_intruders`] counts nodes of other groups inside a group's
/// interquartile band, which a single outlier cannot skew. With fewer than two groups they stay
/// `1.0` and `0`.
pub(super) fn fill_lane_purity(problem: &Problem, boxes: &[NodeBox], m: &mut LayoutMetrics) {
    m.lane_purity = 1.0;
    let n = boxes.len().min(problem.node_count);
    let cross = |i: usize| {
        let b = &boxes[i];
        if problem.left_right {
            b.cy
        } else {
            b.cx
        }
    };
    // Sorted so the result does not depend on node order.
    let mut ids: Vec<u32> = (0..n)
        .filter_map(|i| problem.groups.get(i).copied().flatten())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.len() < 2 {
        return;
    }
    let members: Vec<Vec<f64>> = ids
        .iter()
        .map(|&g| {
            let mut v: Vec<f64> = (0..n)
                .filter(|&i| problem.groups.get(i).copied().flatten() == Some(g))
                .map(cross)
                .collect();
            v.sort_unstable_by_dyn(f64::total_cmp);
            v
        })
        .collect();
    let spans: Vec<(f64, f64)> = members
        .iter()
        .map(|v| (v[0], v[v.len() - 1]))
        .collect::<Vec<_>>();

    let mut purity_sum = 0.0;
    for (k, &(lo, hi)) in spans.iter().enumerate() {
        let others: Vec<(f64, f64)> = spans
            .iter()
            .enumerate()
            .filter(|&(j, _)| j != k)
            .map(|(_, &s)| s)
            .collect();
        purity_sum += if hi - lo > EPS {
            1.0 - covered_length(lo, hi, &others) / (hi - lo)
        } else if others.iter().any(|&(a, b)| lo >= a && lo <= b) {
            0.0
        } else {
            1.0
        };
    }
    m.lane_purity = (purity_sum / spans.len() as f64).clamp(0.0, 1.0);

    for (k, values) in members.iter().enumerate() {
        let (q1, q3) = interquartile(values);
        m.lane_intruders += (0..n)
            // Only another group's node intrudes. An ungrouped node is often shared between
            // groups and may stand in any lane, and `lane_purity` ignores it too.
            .filter(|&i| matches!(problem.groups.get(i).copied().flatten(), Some(g) if g != ids[k]))
            .filter(|&i| {
                let c = cross(i);
                c >= q1 && c <= q3
            })
            .count();
    }
}

/// Length of `[lo, hi]` covered by the union of `others`.
fn covered_length(lo: f64, hi: f64, others: &[(f64, f64)]) -> f64 {
    let mut clipped: Vec<(f64, f64)> = others
        .iter()
        .map(|&(a, b)| (a.max(lo), b.min(hi)))
        .filter(|&(a, b)| b > a)
        .collect();
    clipped.sort_unstable_by_dyn(crate::geometry::stats::pair_order);
    let mut covered = 0.0;
    let mut reach = f64::NEG_INFINITY;
    for (a, b) in clipped {
        let start = a.max(reach);
        if b > start {
            covered += b - start;
            reach = b;
        }
    }
    covered
}

/// `(first quartile, third quartile)` of a sorted, non-empty slice, linearly interpolated.
fn interquartile(sorted: &[f64]) -> (f64, f64) {
    (quantile(sorted, 0.25), quantile(sorted, 0.75))
}

/// The `q`-quantile of a sorted, non-empty slice.
fn quantile(sorted: &[f64], q: f64) -> f64 {
    let last = sorted.len() - 1;
    let pos = q * last as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    if lo == hi {
        sorted[lo]
    } else {
        sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64)
    }
}

/// Mooney et al.'s Node Uniformity as a coefficient of variation.
///
/// The bounding box is cut into roughly square cells, about one per node, and each cell gets the
/// node-box area inside it. The result is the standard deviation over the mean, so `0.0` is
/// perfectly even and one dense corner scores high.
pub(super) fn fill_whitespace_balance(boxes: &[NodeBox], m: &mut LayoutMetrics) {
    if boxes.len() < 2 || m.width <= 0.0 || m.height <= 0.0 {
        return;
    }
    let n = boxes.len() as f64;
    let cols = ((n * m.width / m.height).sqrt().round() as usize).clamp(1, boxes.len());
    let rows = ((n / cols as f64).round() as usize).max(1);
    if cols * rows < 2 {
        return;
    }
    let (left, top) = (
        boxes.iter().map(|b| b.left()).fold(f64::INFINITY, f64::min),
        boxes.iter().map(|b| b.top()).fold(f64::INFINITY, f64::min),
    );
    let (cw, ch) = (m.width / cols as f64, m.height / rows as f64);
    let mut cells = vec![0.0f64; cols * rows];
    for b in boxes {
        // Edges are not charged, since a region crossed only by long edges should read as empty.
        for r in 0..rows {
            let (ct, cb) = (top + r as f64 * ch, top + (r + 1) as f64 * ch);
            let oy = b.bottom().min(cb) - b.top().max(ct);
            if oy <= 0.0 {
                continue;
            }
            for c in 0..cols {
                let (cl, cr) = (left + c as f64 * cw, left + (c + 1) as f64 * cw);
                let ox = b.right().min(cr) - b.left().max(cl);
                if ox > 0.0 {
                    cells[r * cols + c] += ox * oy;
                }
            }
        }
    }
    let mean = cells.iter().sum::<f64>() / cells.len() as f64;
    if mean <= 0.0 {
        return;
    }
    let variance = cells.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / cells.len() as f64;
    m.whitespace_balance = variance.sqrt() / mean;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::super::measure;
    use super::super::tests::compute;
    use super::super::tests::{boxes, layout, node, spec};
    use crate::geometry::{Point, Size};
    use crate::layout::{EdgeRoute, Layout, Placement};
    use crate::{Direction, Edge, Graph, GroupId, LayoutOptions, Node, NodeId};

    /// The fixtures are drawn top to bottom, but `LayoutOptions` defaults to left to right.
    fn tb() -> LayoutOptions {
        LayoutOptions {
            direction: Direction::TopBottom,
            ..LayoutOptions::default()
        }
    }

    const SIZE: Size = Size {
        width: 60.0,
        height: 30.0,
    };

    /// Grouped nodes at the given `(x, y)` centres, without edges.
    fn grouped(points: &[([f64; 2], u32)]) -> (Graph, Layout) {
        let nodes: Vec<Node> = points
            .iter()
            .map(|&(_, g)| Node {
                size: SIZE,
                group: Some(GroupId(g)),
                ..Node::default()
            })
            .collect();
        let placements: Vec<Placement> = points
            .iter()
            .map(|&(p, _)| Placement {
                center: Point::new(p[0], p[1]),
                size: SIZE,
                ..Placement::default()
            })
            .collect();
        (
            Graph {
                nodes,
                edges: Vec::new(),
            },
            Layout {
                nodes: placements,
                edges: Vec::new(),
                ..Layout::default()
            },
        )
    }

    #[test]
    fn edge_through_box_is_one_hit() {
        let s = spec(boxes(3), vec![(0, 1)], "TB");
        let through = layout(
            vec![[0.0, 0.0], [0.0, 400.0], [0.0, 200.0]],
            vec![vec![[0.0, 25.0], [0.0, 375.0]]],
        );
        let m = compute(&s, &through);
        assert_eq!(m.node_edge_hits, 1);
        assert_eq!(m.min_edge_node_clearance, Some(0.0));

        let clear = layout(
            vec![[0.0, 0.0], [0.0, 400.0], [200.0, 200.0]],
            vec![vec![[0.0, 25.0], [0.0, 375.0]]],
        );
        let m = compute(&s, &clear);
        assert_eq!(m.node_edge_hits, 0);
        // Node 2 spans x in [150, 250] and the edge runs at x = 0.
        assert!((m.min_edge_node_clearance.unwrap() - 150.0).abs() < 1e-6);
    }

    /// An edge on a node's border is not a hit (1px tolerance).
    #[test]
    fn edge_along_border_is_not_a_hit() {
        let s = spec(boxes(3), vec![(0, 1)], "TB");
        let l = layout(
            vec![[0.0, 0.0], [0.0, 400.0], [50.0, 200.0]],
            vec![vec![[0.0, 25.0], [0.0, 375.0]]], // node 2 spans x in [0, 100]
        );
        assert_eq!(compute(&s, &l).node_edge_hits, 0);
    }

    /// A segment through the corner of an ellipse's box misses the ellipse.
    #[test]
    fn ellipse_corner_is_not_a_hit() {
        let mut nodes = boxes(2);
        nodes.push(node(100.0, 100.0, true));
        let s = spec(nodes, vec![(0, 1)], "TB");
        // The segment cuts the top-left corner of node 2's box around (200, 200).
        let corner = layout(
            vec![[0.0, 0.0], [0.0, 400.0], [200.0, 200.0]],
            vec![vec![[110.0, 155.0], [155.0, 110.0]]],
        );
        assert_eq!(compute(&s, &corner).node_edge_hits, 0);
        let centre = layout(
            vec![[0.0, 0.0], [0.0, 400.0], [200.0, 200.0]],
            vec![vec![[120.0, 200.0], [280.0, 200.0]]],
        );
        assert_eq!(compute(&s, &centre).node_edge_hits, 1);
    }

    #[test]
    fn node_overlaps_and_gap() {
        let s = spec(boxes(3), vec![], "TB");
        let apart = layout(vec![[0.0, 0.0], [200.0, 0.0], [400.0, 0.0]], vec![]);
        let m = compute(&s, &apart);
        assert_eq!(m.node_overlaps, 0);
        assert!((m.min_node_gap.unwrap() - 100.0).abs() < 1e-6);

        let stacked = layout(vec![[0.0, 0.0], [10.0, 0.0], [400.0, 0.0]], vec![]);
        let m = compute(&s, &stacked);
        assert_eq!(m.node_overlaps, 1);
        // The gap is minus the smaller penetration depth (90px on x, 50px on y).
        assert!((m.min_node_gap.unwrap() + 50.0).abs() < 1e-6);
    }

    #[test]
    fn bounding_box_covers_nodes_and_routes() {
        let s = spec(boxes(2), vec![(0, 1)], "TB");
        let l = layout(
            vec![[0.0, 0.0], [0.0, 200.0]],
            vec![vec![[0.0, 25.0], [300.0, 100.0], [0.0, 175.0]]],
        );
        let m = compute(&s, &l);
        assert!((m.width - 350.0).abs() < 1e-6); // -50 .. 300
        assert!((m.height - 250.0).abs() < 1e-6); // -25 .. 225
        assert!((m.area - 350.0 * 250.0).abs() < 1e-6);
        assert!((m.aspect_ratio - 1.4).abs() < 1e-6);
    }

    #[test]
    fn fit_font_px_is_the_label_after_scaling_to_fit() {
        // 100 x 450 fits unscaled.
        let s = spec(boxes(2), vec![(0, 1)], "TB");
        let small = compute(
            &s,
            &layout(
                vec![[0.0, 0.0], [0.0, 400.0]],
                vec![vec![[0.0, 25.0], [0.0, 375.0]]],
            ),
        );
        assert!(
            (small.fit_font_px - 12.5).abs() < 1e-9,
            "a small drawing is not blown up"
        );

        // 6400 x 50 scales by 1600/6400 = 0.25.
        let wide = compute(
            &s,
            &layout(
                vec![[0.0, 0.0], [6300.0, 0.0]],
                vec![vec![[50.0, 0.0], [6250.0, 0.0]]],
            ),
        );
        assert!((wide.width - 6400.0).abs() < 1e-6);
        assert!(
            (wide.fit_font_px - 12.5 * 0.25).abs() < 1e-9,
            "{}",
            wide.fit_font_px
        );
        // It scores perfectly elsewhere, but its labels shrink to about 3px.
        assert_eq!(wide.crossings, 0);
        assert_eq!(wide.bends, 0);
        assert!(wide.fit_font_px < 7.0);
    }

    #[test]
    fn density_and_aspect_penalty_describe_the_shape() {
        // 10 000 px2 of node in 45 000 px2 of drawing.
        let s = spec(boxes(2), vec![(0, 1)], "TB");
        let tall = compute(
            &s,
            &layout(
                vec![[0.0, 0.0], [0.0, 400.0]],
                vec![vec![[0.0, 25.0], [0.0, 375.0]]],
            ),
        );
        assert!(
            (tall.density - 10_000.0 / 45_000.0).abs() < 1e-9,
            "{}",
            tall.density
        );
        assert!((tall.aspect_penalty - 450.0 / 100.0).abs() < 1e-9);

        let wide = compute(
            &s,
            &layout(
                vec![[0.0, 0.0], [400.0, 0.0]],
                vec![vec![[50.0, 0.0], [350.0, 0.0]]],
            ),
        );
        assert!((wide.aspect_penalty - 500.0 / 50.0).abs() < 1e-9);
        assert!(wide.aspect_ratio > 1.0 && tall.aspect_ratio < 1.0);
        assert!(wide.density > 0.0 && wide.density <= 1.0);
    }

    /// A straight main line and a mirrored pair are aligned, a lopsided pair is not.
    #[test]
    fn split_alignment_finds_and_judges_fork_join_blocks() {
        // 0 forks to 1 and 2, which merge at 3.
        let s = spec(boxes(4), vec![(0, 1), (0, 2), (1, 3), (2, 3)], "TB");
        let route = |a: [f64; 2], b: [f64; 2]| vec![a, b];
        let routes = |c: &Vec<[f64; 2]>| {
            vec![
                route(c[0], c[1]),
                route(c[0], c[2]),
                route(c[1], c[3]),
                route(c[2], c[3]),
            ]
        };

        let sym = vec![[0.0, 0.0], [-200.0, 200.0], [200.0, 200.0], [0.0, 400.0]];
        let m = compute(&s, &layout(sym.clone(), routes(&sym)));
        assert_eq!((m.split_pairs, m.split_aligned), (1, 1));
        assert!((m.split_alignment - 1.0).abs() < 1e-9);

        // Branch 1 on the 0 -> 3 line, branch 2 out to the side.
        let straight = vec![[0.0, 0.0], [0.0, 200.0], [260.0, 200.0], [0.0, 400.0]];
        let m = compute(&s, &layout(straight.clone(), routes(&straight)));
        assert_eq!((m.split_pairs, m.split_aligned), (1, 1));

        // Both branches off to one side by different amounts.
        let skew = vec![[0.0, 0.0], [120.0, 200.0], [400.0, 200.0], [0.0, 400.0]];
        let m = compute(&s, &layout(skew.clone(), routes(&skew)));
        assert_eq!((m.split_pairs, m.split_aligned), (1, 0));
        assert!((m.split_alignment - 0.0).abs() < 1e-9);
    }

    /// No fork/join block reads as 1.0, so "nothing to align" is not a defect.
    #[test]
    fn split_alignment_is_one_when_there_is_nothing_to_align() {
        let s = spec(boxes(3), vec![(0, 1), (0, 2)], "TB");
        let c = vec![[0.0, 0.0], [-200.0, 200.0], [200.0, 200.0]];
        let m = compute(
            &s,
            &layout(c.clone(), vec![vec![c[0], c[1]], vec![c[0], c[2]]]),
        );
        assert_eq!((m.split_pairs, m.split_aligned), (0, 0));
        assert!((m.split_alignment - 1.0).abs() < 1e-9);

        let chain = spec(boxes(3), vec![(0, 1), (1, 2)], "TB");
        let m = compute(
            &chain,
            &layout(c.clone(), vec![vec![c[0], c[1]], vec![c[1], c[2]]]),
        );
        assert_eq!(m.split_pairs, 0);
    }

    /// Every pair of a three-way fork counts, and a fan-out over the cap is skipped.
    #[test]
    fn split_search_is_capped_by_out_degree() {
        let three = spec(
            boxes(5),
            vec![(0, 1), (0, 2), (0, 3), (1, 4), (2, 4), (3, 4)],
            "TB",
        );
        let c = vec![
            [0.0, 0.0],
            [-200.0, 200.0],
            [0.0, 200.0],
            [200.0, 200.0],
            [0.0, 400.0],
        ];
        let r: Vec<Vec<[f64; 2]>> = vec![
            vec![c[0], c[1]],
            vec![c[0], c[2]],
            vec![c[0], c[3]],
            vec![c[1], c[4]],
            vec![c[2], c[4]],
            vec![c[3], c[4]],
        ];
        let m = compute(&three, &layout(c, r));
        assert_eq!(m.split_pairs, 3);
        // (1,3) mirror each other, (1,2) and (2,3) are a straight main line with a skip.
        assert_eq!(m.split_aligned, 3);

        // 0 -> {1..7} -> 8 is over the cap.
        let n = 9;
        let mut edges: Vec<(usize, usize)> = (1..8).map(|i| (0, i)).collect();
        edges.extend((1..8).map(|i| (i, 8)));
        let big = spec(boxes(n), edges.clone(), "TB");
        let centers: Vec<[f64; 2]> = (0..n).map(|i| [i as f64 * 150.0, 0.0]).collect();
        let routes: Vec<Vec<[f64; 2]>> = edges
            .iter()
            .map(|&(a, b)| vec![centers[a], centers[b]])
            .collect();
        assert_eq!(compute(&big, &layout(centers, routes)).split_pairs, 0);
    }

    #[test]
    fn a_foreign_edge_through_a_block_is_an_intrusion() {
        let n = Node {
            size: SIZE,
            ..Node::default()
        };
        // A 0 -> {1, 2} -> 3 block and an unrelated edge 4 -> 5.
        let graph = Graph {
            nodes: vec![n.clone(), n.clone(), n.clone(), n.clone(), n.clone(), n],
            edges: vec![
                Edge::new(NodeId(0), NodeId(1)),
                Edge::new(NodeId(0), NodeId(2)),
                Edge::new(NodeId(1), NodeId(3)),
                Edge::new(NodeId(2), NodeId(3)),
                Edge::new(NodeId(4), NodeId(5)),
            ],
        };
        let place = |x: f64, y: f64| Placement {
            center: Point::new(x, y),
            size: SIZE,
            ..Placement::default()
        };
        let route = |a: [f64; 2], b: [f64; 2]| EdgeRoute {
            points: vec![Point::new(a[0], a[1]), Point::new(b[0], b[1])],
            ..EdgeRoute::default()
        };
        let nodes = vec![
            place(0.0, 0.0),
            place(-100.0, 150.0),
            place(100.0, 150.0),
            place(0.0, 300.0),
            place(-400.0, 150.0),
            place(400.0, 150.0),
        ];
        let through = Layout {
            nodes: nodes.clone(),
            edges: vec![
                route([0.0, 15.0], [-100.0, 135.0]),
                route([0.0, 15.0], [100.0, 135.0]),
                route([-100.0, 165.0], [0.0, 285.0]),
                route([100.0, 165.0], [0.0, 285.0]),
                route([-370.0, 150.0], [370.0, 150.0]),
            ],
            ..Layout::default()
        };
        let m = measure(&graph, &tb(), &through);
        assert_eq!(m.split_pairs, 1);
        assert_eq!(m.block_intrusion, 1, "{m:?}");

        let mut clear = through.clone();
        clear.edges[4] = route([-370.0, 500.0], [370.0, 500.0]);
        assert_eq!(measure(&graph, &tb(), &clear).block_intrusion, 0);
    }

    #[test]
    fn lane_purity_separates_clean_bands_from_interleaved_ones() {
        let clean = grouped(&[
            ([0.0, 0.0], 0),
            ([0.0, 100.0], 0),
            ([200.0, 0.0], 1),
            ([200.0, 100.0], 1),
            ([400.0, 0.0], 2),
            ([400.0, 100.0], 2),
        ]);
        let m = measure(&clean.0, &tb(), &clean.1);
        assert_eq!(m.lane_purity, 1.0);
        assert_eq!(m.lane_intruders, 0);

        // Here every group spans the whole cross axis.
        let mixed = grouped(&[
            ([0.0, 0.0], 0),
            ([200.0, 100.0], 0),
            ([400.0, 200.0], 0),
            ([0.0, 300.0], 1),
            ([200.0, 400.0], 1),
            ([400.0, 500.0], 1),
            ([0.0, 600.0], 2),
            ([200.0, 700.0], 2),
            ([400.0, 800.0], 2),
        ]);
        let m = measure(&mixed.0, &tb(), &mixed.1);
        assert_eq!(m.lane_purity, 0.0);
        // Each group's band is x in [100, 300], holding the two foreign nodes at x = 200.
        assert_eq!(m.lane_intruders, 6, "{m:?}");
    }

    #[test]
    fn lane_purity_is_one_without_two_groups() {
        let one = grouped(&[([0.0, 0.0], 0), ([200.0, 0.0], 0)]);
        assert_eq!(measure(&one.0, &tb(), &one.1).lane_purity, 1.0);
        let s = spec(boxes(3), vec![], "TB");
        let l = layout(vec![[0.0, 0.0], [200.0, 0.0], [400.0, 0.0]], vec![]);
        assert_eq!(compute(&s, &l).lane_purity, 1.0);
    }

    #[test]
    fn whitespace_balance_notices_a_pile_in_one_corner() {
        let spread: Vec<([f64; 2], u32)> = (0..4)
            .flat_map(|r| (0..4).map(move |c| ([c as f64 * 200.0, r as f64 * 200.0], 0)))
            .collect();
        let even = grouped(&spread);
        let even_cv = measure(&even.0, &tb(), &even.1).whitespace_balance;

        let mut piled: Vec<([f64; 2], u32)> = (0..15)
            .map(|i| ([(i % 4) as f64 * 10.0, (i / 4) as f64 * 10.0], 0))
            .collect();
        piled.push(([600.0, 600.0], 0)); // keeps the bounding box the same shape
        let heap = grouped(&piled);
        let heap_cv = measure(&heap.0, &tb(), &heap.1).whitespace_balance;
        assert!(heap_cv > even_cv, "{heap_cv} vs {even_cv}");
    }
}

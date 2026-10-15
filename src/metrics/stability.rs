//! How much a drawing moved when the graph changed.
//!
//! [`compare`] takes two layouts, so it is not part of [`super::measure`] or the score. Studies
//! disagree on whether stability helps readers (Mennens et al., Hage and Rehse), so use it as a
//! regression signal for a jumpy engine.

use crate::geometry::fp;
use serde::{Deserialize, Serialize};

use super::distance;
use crate::geometry::EPS;
use crate::graph::NodeId;
use crate::layout::Layout;
use crate::sort::SortDyn;

/// Cap on pairwise samples for the two quadratic measures, which keeps their cost flat. The
/// sampled answer stays within a fraction of a percent of the exhaustive one.
const MAX_PAIRS: usize = 2000;
/// Fixed seed, so repeated runs over the same layouts give byte-identical numbers.
const SAMPLE_SEED: u64 = 0x9E37_79B9_7F4A_7C15;
/// Interior points sampled per edge when comparing edge shapes.
const SHAPE_SAMPLES: usize = 8;

/// How far a drawing moved between two layouts of (nearly) the same graph.
///
/// Every distance field is `0.0` for identical drawings and grows with disruption. Positions are
/// compared after moving the common nodes' centroid to the origin, so a pure translation scores
/// zero.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StabilityMetrics {
    /// Entries of `common` that named a placed node in both layouts.
    #[serde(default)]
    pub common_nodes: usize,
    /// Node pairs sampled for `relative_euclidean` and `orthogonal_ordering`, at most 2000.
    #[serde(default)]
    pub sampled_pairs: usize,
    /// Largest distance in px any common node moved, centroid-aligned.
    #[serde(default)]
    pub hausdorff: f64,
    /// Mean over sampled node pairs of `|d_a - d_b| / max(d_a, d_b)`, the relative change in
    /// their distance. In `[0, 1]`, near `1.0` for a drawing redrawn from scratch.
    #[serde(default)]
    pub relative_euclidean: f64,
    /// Fraction of sampled node pairs whose left/right or above/below order flipped. This is the
    /// change a reader notices as a jump.
    #[serde(default)]
    pub orthogonal_ordering: f64,
    /// Mean shape difference over edges present in both drawings. Each polyline is resampled at
    /// 8 arc-length fractions in its own start-to-end frame, so position, length and rotation do
    /// not count.
    #[serde(default)]
    pub edge_shape: f64,
    /// Edges matched between the drawings by their endpoint nodes. `edge_shape` averages these.
    #[serde(default)]
    pub matched_edges: usize,
}

/// Compare two layouts of the same graph before and after a change (e.g., a filter).
///
/// `common` pairs each node of `a` with its node in `b`. Entries naming a node missing from
/// either layout are skipped, so passing the whole node list is fine.
pub fn compare(a: &Layout, b: &Layout, common: &[(NodeId, NodeId)]) -> StabilityMetrics {
    let pairs: Vec<(usize, usize)> = common
        .iter()
        .filter_map(|&(x, y)| {
            let (i, j) = (x.index(), y.index());
            (i < a.nodes.len() && j < b.nodes.len()).then_some((i, j))
        })
        .collect();
    let mut m = StabilityMetrics {
        common_nodes: pairs.len(),
        ..StabilityMetrics::default()
    };
    m.edge_shape = edge_shape(a, b, &pairs, &mut m.matched_edges);
    if pairs.is_empty() {
        return m;
    }

    let pa: Vec<[f64; 2]> = pairs
        .iter()
        .map(|&(i, _)| [a.nodes[i].center.x, a.nodes[i].center.y])
        .collect();
    let pb: Vec<[f64; 2]> = pairs
        .iter()
        .map(|&(_, j)| [b.nodes[j].center.x, b.nodes[j].center.y])
        .collect();
    let ca = centroid(&pa);
    let cb = centroid(&pb);
    m.hausdorff = pa
        .iter()
        .zip(pb.iter())
        .map(|(x, y)| distance([x[0] - ca[0], x[1] - ca[1]], [y[0] - cb[0], y[1] - cb[1]]))
        .fold(0.0f64, f64::max);

    let n = pairs.len();
    let total_pairs = n * n.saturating_sub(1) / 2;
    let samples = sample_pairs(n, total_pairs);
    m.sampled_pairs = samples.len();
    if samples.is_empty() {
        return m;
    }
    let mut relative = 0.0;
    let mut flipped = 0usize;
    for &(i, j) in &samples {
        let da = distance(pa[i], pa[j]);
        let db = distance(pb[i], pb[j]);
        let scale = da.max(db);
        if scale > EPS {
            relative += (da - db).abs() / scale;
        }
        if sign(pa[i][0] - pa[j][0]) != sign(pb[i][0] - pb[j][0])
            || sign(pa[i][1] - pa[j][1]) != sign(pb[i][1] - pb[j][1])
        {
            flipped += 1;
        }
    }
    m.relative_euclidean = relative / samples.len() as f64;
    m.orthogonal_ordering = flipped as f64 / samples.len() as f64;
    m
}

fn centroid(points: &[[f64; 2]]) -> [f64; 2] {
    let n = points.len().max(1) as f64;
    [
        points.iter().map(|p| p[0]).sum::<f64>() / n,
        points.iter().map(|p| p[1]).sum::<f64>() / n,
    ]
}

/// `-1`, `0` or `1`, with a 0.5px dead band so sub-pixel jitter does not count as a swap.
fn sign(d: f64) -> i8 {
    if d > 0.5 {
        1
    } else if d < -0.5 {
        -1
    } else {
        0
    }
}

/// Up to [`MAX_PAIRS`] index pairs out of the `n * (n - 1) / 2` there are.
///
/// Every pair when there are few enough, otherwise a seeded, deduplicated sample that may fall
/// slightly under the cap.
fn sample_pairs(n: usize, total: usize) -> Vec<(usize, usize)> {
    if n < 2 {
        return Vec::new();
    }
    if total <= MAX_PAIRS {
        let mut out = Vec::with_capacity(total);
        for i in 0..n {
            for j in i + 1..n {
                out.push((i, j));
            }
        }
        return out;
    }
    let mut state = SAMPLE_SEED;
    let mut chosen: Vec<usize> = Vec::with_capacity(MAX_PAIRS);
    for _ in 0..MAX_PAIRS {
        chosen.push((next(&mut state) % total as u64) as usize);
    }
    chosen.sort_unstable();
    chosen.dedup();
    chosen.into_iter().map(|k| unrank(k, n)).collect()
}

/// xorshift64*.
fn next(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    *state = x;
    x.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

/// The `k`-th pair `(i, j)` with `i < j < n`, counting row by row. Walking the rows avoids the
/// floating-point rounding of a closed form.
fn unrank(k: usize, n: usize) -> (usize, usize) {
    let mut rest = k;
    for i in 0..n {
        let row = n - i - 1;
        if rest < row {
            return (i, i + 1 + rest);
        }
        rest -= row;
    }
    (0, 1)
}

/// Mean shape difference over the edges both drawings have, matched by their endpoint nodes.
///
/// A [`Layout`] does not record edge endpoints, so each route end is attributed to the node whose
/// outline is nearest. Parallel edges are matched in route order.
fn edge_shape(a: &Layout, b: &Layout, pairs: &[(usize, usize)], matched: &mut usize) -> f64 {
    let mut map = vec![usize::MAX; a.nodes.len()];
    for &(i, j) in pairs {
        map[i] = j;
    }
    let mut keys_a = keyed_shapes(a, |i| map.get(i).copied().unwrap_or(usize::MAX));
    let mut keys_b = keyed_shapes(b, |i| i);
    keys_a.sort_unstable_by_dyn(by_key_then_route);
    keys_b.sort_unstable_by_dyn(by_key_then_route);

    let mut total = 0.0;
    let mut count = 0usize;
    let (mut ia, mut ib) = (0, 0);
    while ia < keys_a.len() && ib < keys_b.len() {
        match keys_a[ia].0.cmp(&keys_b[ib].0) {
            std::cmp::Ordering::Less => ia += 1,
            std::cmp::Ordering::Greater => ib += 1,
            std::cmp::Ordering::Equal => {
                total += shape_difference(&keys_a[ia].2, &keys_b[ib].2);
                count += 1;
                ia += 1;
                ib += 1;
            }
        }
    }
    *matched = count;
    if count == 0 {
        0.0
    } else {
        total / count as f64
    }
}

/// Endpoint pair first, route index second, so parallel edges pair up in emission order.
fn by_key_then_route(x: &KeyedShape, y: &KeyedShape) -> std::cmp::Ordering {
    x.0.cmp(&y.0).then(x.1.cmp(&y.1))
}

/// `((source, target), route index, normalised shape)`.
type KeyedShape = ((usize, usize), usize, Vec<[f64; 2]>);

/// A [`KeyedShape`] for every routed edge, with node indices put through `remap`. Edges whose ends
/// cannot be attributed to a node are dropped.
fn keyed_shapes(layout: &Layout, remap: impl Fn(usize) -> usize) -> Vec<KeyedShape> {
    layout
        .edges
        .iter()
        .enumerate()
        .filter(|(_, r)| r.points.len() >= 2)
        .filter_map(|(i, r)| {
            let pts: Vec<[f64; 2]> = r.points.iter().map(|p| [p.x, p.y]).collect();
            let from = remap(attach(layout, pts[0])?);
            let to = remap(attach(layout, pts[pts.len() - 1])?);
            (from != usize::MAX && to != usize::MAX).then(|| ((from, to), i, normalise(&pts)))
        })
        .collect()
}

/// The node whose outline is nearest `p`.
fn attach(layout: &Layout, p: [f64; 2]) -> Option<usize> {
    let mut best: Option<(f64, usize)> = None;
    for (i, n) in layout.nodes.iter().enumerate() {
        let dx = (p[0] - n.center.x).abs() - n.size.width / 2.0;
        let dy = (p[1] - n.center.y).abs() - n.size.height / 2.0;
        let d = if dx > 0.0 || dy > 0.0 {
            fp::hypot(dx.max(0.0), dy.max(0.0))
        } else {
            (-dx).min(-dy)
        };
        if best.is_none_or(|(w, _)| d < w) {
            best = Some((d, i));
        }
    }
    best.map(|(_, i)| i)
}

/// A polyline resampled at [`SHAPE_SAMPLES`] arc-length fractions, in the frame where it starts
/// at the origin and ends at `(1, 0)`.
fn normalise(pts: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let start = pts[0];
    let end = pts[pts.len() - 1];
    let span = distance(start, end);
    let (ux, uy) = if span > EPS {
        ((end[0] - start[0]) / span, (end[1] - start[1]) / span)
    } else {
        (1.0, 0.0)
    };
    let scale = if span > EPS { 1.0 / span } else { 1.0 };
    (0..SHAPE_SAMPLES)
        .map(|k| {
            let t = (k + 1) as f64 / (SHAPE_SAMPLES + 1) as f64;
            let p = at_fraction(pts, t);
            let (vx, vy) = (p[0] - start[0], p[1] - start[1]);
            [(vx * ux + vy * uy) * scale, (vx * -uy + vy * ux) * scale]
        })
        .collect()
}

/// The point `t` of the way along a polyline by arc length.
fn at_fraction(pts: &[[f64; 2]], t: f64) -> [f64; 2] {
    let seg: Vec<f64> = pts.windows(2).map(|w| distance(w[0], w[1])).collect();
    let total: f64 = seg.iter().sum();
    if total <= EPS {
        return pts[0];
    }
    let want = total * t;
    let mut acc = 0.0;
    for (i, &s) in seg.iter().enumerate() {
        if acc + s >= want {
            let f = if s <= EPS { 0.0 } else { (want - acc) / s };
            return [
                pts[i][0] + (pts[i + 1][0] - pts[i][0]) * f,
                pts[i][1] + (pts[i + 1][1] - pts[i][1]) * f,
            ];
        }
        acc += s;
    }
    pts[pts.len() - 1]
}

/// Mean distance between two normalised shapes' corresponding sample points.
fn shape_difference(a: &[[f64; 2]], b: &[[f64; 2]]) -> f64 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| distance(*x, *y))
        .sum::<f64>()
        / a.len() as f64
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::geometry::{Point, Size};
    use crate::layout::{EdgeRoute, Placement};

    const SIZE: Size = Size {
        width: 60.0,
        height: 30.0,
    };

    fn build(centers: &[[f64; 2]], routes: &[Vec<[f64; 2]>]) -> Layout {
        Layout {
            nodes: centers
                .iter()
                .map(|c| Placement {
                    center: Point::new(c[0], c[1]),
                    size: SIZE,
                    ..Placement::default()
                })
                .collect(),
            edges: routes
                .iter()
                .map(|pts| EdgeRoute {
                    points: pts.iter().map(|p| Point::new(p[0], p[1])).collect(),
                    ..EdgeRoute::default()
                })
                .collect(),
            ..Layout::default()
        }
    }

    fn all(n: usize) -> Vec<(NodeId, NodeId)> {
        (0..n as u32).map(|i| (NodeId(i), NodeId(i))).collect()
    }

    /// A drawing compared with itself has moved nothing at all.
    #[test]
    fn an_identical_layout_is_perfectly_stable() {
        let l = build(
            &[[0.0, 0.0], [0.0, 200.0], [200.0, 200.0]],
            &[vec![[0.0, 15.0], [0.0, 185.0]]],
        );
        let m = compare(&l, &l, &all(3));
        assert_eq!(m.common_nodes, 3);
        assert_eq!(m.hausdorff, 0.0);
        assert_eq!(m.relative_euclidean, 0.0);
        assert_eq!(m.orthogonal_ordering, 0.0);
        assert_eq!(m.edge_shape, 0.0);
        assert_eq!(m.matched_edges, 1);
    }

    /// Centroid alignment absorbs a pure translation.
    #[test]
    fn a_pure_translation_is_stable() {
        let a = build(&[[0.0, 0.0], [0.0, 200.0], [200.0, 200.0]], &[]);
        let b = build(&[[400.0, 700.0], [400.0, 900.0], [600.0, 900.0]], &[]);
        let m = compare(&a, &b, &all(3));
        assert!(m.hausdorff < 1e-9, "{m:?}");
        assert_eq!(m.relative_euclidean, 0.0);
        assert_eq!(m.orthogonal_ordering, 0.0);
    }

    /// Swapping two nodes registers in every measure.
    #[test]
    fn a_swap_is_seen_by_every_measure() {
        let a = build(&[[0.0, 0.0], [200.0, 0.0], [500.0, 300.0]], &[]);
        let b = build(&[[200.0, 0.0], [0.0, 0.0], [500.0, 300.0]], &[]);
        let m = compare(&a, &b, &all(3));
        assert_eq!(m.hausdorff, 200.0);
        assert_eq!(m.sampled_pairs, 3);
        // Only the swapped pair changed sides. Both still sit left of and above node 2.
        assert!((m.orthogonal_ordering - 1.0 / 3.0).abs() < 1e-9, "{m:?}");
        // Nodes 0 and 1 are still 200px apart, but their distances to node 2 changed.
        assert!(m.relative_euclidean > 0.0, "{m:?}");
    }

    /// A smaller bulge is a smaller shape change than flattening the edge.
    #[test]
    fn edge_shape_compares_shape_not_position() {
        let nodes = [[0.0, 0.0], [0.0, 400.0]];
        let bent = build(&nodes, &[vec![[0.0, 15.0], [100.0, 200.0], [0.0, 385.0]]]);
        let scaled = build(&nodes, &[vec![[0.0, 15.0], [50.0, 200.0], [0.0, 385.0]]]);
        let straight = build(&nodes, &[vec![[0.0, 15.0], [0.0, 385.0]]]);
        let same = compare(&bent, &bent, &all(2));
        assert_eq!(same.edge_shape, 0.0);
        let flattened = compare(&bent, &straight, &all(2));
        assert!(flattened.edge_shape > 0.0, "{flattened:?}");
        // Halving the bulge is a smaller change than removing it.
        let halved = compare(&bent, &scaled, &all(2));
        assert!(halved.edge_shape < flattened.edge_shape, "{halved:?}");
    }

    /// An edge only one drawing has is not matched, so it does not count as unchanged.
    #[test]
    fn an_edge_present_in_only_one_drawing_is_not_matched() {
        let nodes = [[0.0, 0.0], [0.0, 400.0], [300.0, 200.0]];
        let a = build(
            &nodes,
            &[
                vec![[0.0, 15.0], [0.0, 385.0]],
                vec![[30.0, 0.0], [270.0, 200.0]],
            ],
        );
        let b = build(&nodes, &[vec![[0.0, 15.0], [0.0, 385.0]]]);
        let m = compare(&a, &b, &all(3));
        assert_eq!(m.matched_edges, 1);
    }

    /// No common nodes yields zeros, not a panic.
    #[test]
    fn no_common_nodes_is_not_an_error() {
        let a = build(&[[0.0, 0.0]], &[]);
        let b = build(&[[0.0, 0.0]], &[]);
        let m = compare(&a, &b, &[]);
        assert_eq!(m.common_nodes, 0);
        assert_eq!(m.sampled_pairs, 0);
        assert_eq!(m.hausdorff, 0.0);
    }

    /// The pairwise sample is capped, and two runs sample the same pairs.
    #[test]
    fn the_pair_sample_is_capped_and_seeded() {
        let centers: Vec<[f64; 2]> = (0..300).map(|i| [i as f64, (i * 7 % 53) as f64]).collect();
        let a = build(&centers, &[]);
        let shifted: Vec<[f64; 2]> = centers.iter().map(|c| [c[0], c[1] + 3.0]).collect();
        let b = build(&shifted, &[]);
        let m1 = compare(&a, &b, &all(300));
        let m2 = compare(&a, &b, &all(300));
        assert_eq!(m1, m2);
        assert!(m1.sampled_pairs <= MAX_PAIRS, "{m1:?}");
        // 300 nodes have 44850 pairs, so this exercises the sampled path.
        assert!(m1.sampled_pairs > MAX_PAIRS / 2, "{m1:?}");
    }

    /// Every pair is enumerated exactly once when there are few enough of them.
    #[test]
    fn small_graphs_are_compared_exhaustively() {
        let n = 20;
        let pairs = sample_pairs(n, n * (n - 1) / 2);
        assert_eq!(pairs.len(), n * (n - 1) / 2);
        let mut sorted = pairs.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), pairs.len());
    }

    /// `unrank` walks the upper triangle in order.
    #[test]
    fn unrank_enumerates_the_upper_triangle() {
        let n = 5;
        let expected: Vec<(usize, usize)> = (0..n)
            .flat_map(|i| (i + 1..n).map(move |j| (i, j)))
            .collect();
        for (k, want) in expected.iter().enumerate() {
            assert_eq!(unrank(k, n), *want, "k={k}");
        }
    }
}

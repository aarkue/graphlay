//! Merged edges. Forks leave a node on one shared trunk and branch off a common riser, and joins
//! arrive the same way.
//!
//! Runs over finished orthogonal routes in flow space. Forks and joins follow the input's arrow
//! direction, so an edge the cycle breaker reversed still joins the edges arriving at its head. A
//! merge is kept only if it adds no crossing, bend or node hit.

use crate::geometry::hit::segments_cross;
use crate::geometry::Boxes;

/// Ends closer than this (px) are the same point.
const EPS: f64 = 0.5;

/// A route's end at a node border.
#[derive(Clone, Copy, Debug)]
struct End {
    route: usize,
    /// The route's first point (`true`) or its last.
    first: bool,
}

/// Merges forks and joins in `routes` (flow space). `ends[i]` is route `i`'s `(first node, last
/// node, whether the first point is the edge's source)`, real nodes only (`usize::MAX` for none).
/// Returns the junctions where a branch leaves a shared run.
pub(crate) fn merge_routes(
    routes: &mut [Vec<(f64, f64)>],
    ends: &[(usize, usize, bool)],
    b: &Boxes,
    stub: f64,
    forks: bool,
    joins: bool,
) -> Vec<(f64, f64)> {
    let mut junctions = Vec::new();
    let roles = [(forks, true), (joins, false)];
    for node in 0..b.n_real {
        for forward in [true, false] {
            for (_, source) in roles.iter().filter(|r| r.0) {
                let face = b.cx[node] + if forward { 1.0 } else { -1.0 } * b.layer_w[node] / 2.0;
                let members: Vec<End> = ends
                    .iter()
                    .enumerate()
                    .flat_map(|(i, &(a, z, first_is_source))| {
                        [(a, true), (z, false)]
                            .into_iter()
                            .filter(move |&(n, first)| {
                                n == node && (first == first_is_source) == *source
                            })
                            .map(move |(_, first)| End { route: i, first })
                    })
                    .filter(|e| leaves_face(&routes[e.route], e.first, face, forward))
                    .collect();
                if members.len() < 2 {
                    continue;
                }
                if let Some(js) = merge_group(routes, &members, b, node, face, forward, stub) {
                    junctions.extend(js);
                }
            }
        }
    }
    junctions
}

/// The route oriented to start at the end `first`.
fn walk(route: &[(f64, f64)], first: bool) -> Vec<(f64, f64)> {
    if first {
        route.to_vec()
    } else {
        route.iter().rev().copied().collect()
    }
}

/// Whether the route's end sits on the face at `x = face` and its first segment runs straight away
/// from the node.
fn leaves_face(route: &[(f64, f64)], first: bool, face: f64, forward: bool) -> bool {
    if route.len() < 2 {
        return false;
    }
    let w = walk(route, first);
    let dir = if forward { 1.0 } else { -1.0 };
    (w[0].0 - face).abs() < EPS && (w[1].1 - w[0].1).abs() < EPS && (w[1].0 - w[0].0) * dir > EPS
}

/// Merges one fork or join group unless that costs a crossing, a bend or a node hit. Returns the
/// junctions it made.
fn merge_group(
    routes: &mut [Vec<(f64, f64)>],
    members: &[End],
    b: &Boxes,
    node: usize,
    face: f64,
    forward: bool,
    stub: f64,
) -> Option<Vec<(f64, f64)>> {
    let dir = if forward { 1.0 } else { -1.0 };
    // Local frame where the walk runs towards +x from the face.
    let to_local = |p: (f64, f64)| ((p.0 - face) * dir, p.1);
    let from_local = |p: (f64, f64)| (p.0 * dir + face, p.1);
    let walks: Vec<Vec<(f64, f64)>> = members
        .iter()
        .map(|e| {
            walk(&routes[e.route], e.first)
                .into_iter()
                .map(to_local)
                .collect()
        })
        .collect();
    let riser = |w: &[(f64, f64)]| -> f64 {
        let mut x = w[1].0;
        for p in &w[1..] {
            if (p.1 - w[0].1).abs() >= EPS {
                break;
            }
            x = p.0;
        }
        x
    };
    // A straight member would gain two bends on a trunk at another height, so it sets the trunk
    // height and straight members at other heights stay out.
    let straight = |w: &[(f64, f64)]| w.iter().all(|p| (p.1 - w[0].1).abs() < EPS);
    let trunk_y = walks
        .iter()
        .filter(|w| straight(w))
        .map(|w| w[0].1)
        .min_by(|a, c| (a - b.cy[node]).abs().total_cmp(&(c - b.cy[node]).abs()))
        .unwrap_or(b.cy[node]);
    let keep: Vec<usize> = (0..walks.len())
        .filter(|&k| !straight(&walks[k]) || (walks[k][0].1 - trunk_y).abs() < EPS)
        .collect();
    if keep.len() < 2 {
        return None;
    }
    let members: Vec<End> = keep.iter().map(|&k| members[k]).collect();
    let walks: Vec<Vec<(f64, f64)>> = keep.into_iter().map(|k| walks[k].clone()).collect();
    let bus = walks.iter().map(|w| riser(w)).fold(f64::INFINITY, f64::min);
    if !bus.is_finite() || bus < stub.min(4.0) {
        return None;
    }
    let mut merged: Vec<Vec<(f64, f64)>> = Vec::with_capacity(walks.len());
    let mut branch_y: Vec<f64> = Vec::with_capacity(walks.len());
    for w in &walks {
        let r = (1..w.len())
            .take_while(|&k| (w[k].1 - w[0].1).abs() < EPS)
            .last()?;
        let (y, rest) = if r + 1 < w.len() && (w[r + 1].0 - w[r].0).abs() < EPS {
            // A walk that doubles back after its riser would run over its own branch.
            if r + 2 < w.len() && w[r + 2].0 < w[r + 1].0 - EPS {
                return None;
            }
            (w[r + 1].1, &w[r + 1..])
        } else {
            (w[0].1, &w[r..])
        };
        let mut m = vec![(0.0, trunk_y), (bus, trunk_y), (bus, y)];
        m.extend_from_slice(rest);
        merged.push(simplify(m));
        branch_y.push(y);
    }

    let idx: Vec<usize> = members.iter().map(|e| e.route).collect();
    let before = cost(routes, &idx, b);
    let old: Vec<Vec<(f64, f64)>> = idx.iter().map(|&i| routes[i].clone()).collect();
    for (e, m) in members.iter().zip(&merged) {
        let back: Vec<(f64, f64)> = m.iter().map(|&p| from_local(p)).collect();
        routes[e.route] = if e.first {
            back
        } else {
            back.into_iter().rev().collect()
        };
    }
    let after = cost(routes, &idx, b);
    if after.0 > before.0 || after.1 > before.1 || after.2 > before.2 {
        for (&i, r) in idx.iter().zip(old) {
            routes[i] = r;
        }
        return None;
    }

    // Junctions sit where the trunk splits and where a branch leaves a bus that continues past it.
    let mut js = Vec::new();
    let up = branch_y.iter().filter(|&&y| y < trunk_y - EPS).count();
    let down = branch_y.iter().filter(|&&y| y > trunk_y + EPS).count();
    let level = branch_y.len() - up - down;
    if [up, down, level].iter().filter(|&&c| c > 0).count() > 1 {
        js.push(from_local((bus, trunk_y)));
    }
    let mut ys: Vec<f64> = branch_y.clone();
    ys.sort_by(f64::total_cmp);
    ys.dedup_by(|a, b| (*a - *b).abs() < EPS);
    let (lo, hi) = (ys.first().copied(), ys.last().copied());
    for &y in &ys {
        let far_end = Some(y) == lo && y < trunk_y - EPS || Some(y) == hi && y > trunk_y + EPS;
        if (y - trunk_y).abs() >= EPS && !far_end {
            js.push(from_local((bus, y)));
        }
    }
    Some(js)
}

/// Drops repeated points and the middle of three collinear ones.
fn simplify(pts: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(pts.len());
    for p in pts {
        if let Some(&q) = out.last() {
            if (p.0 - q.0).abs() < EPS && (p.1 - q.1).abs() < EPS {
                continue;
            }
        }
        if out.len() >= 2 {
            let (a, q) = (out[out.len() - 2], out[out.len() - 1]);
            let same_x = (a.0 - q.0).abs() < EPS && (q.0 - p.0).abs() < EPS;
            let same_y = (a.1 - q.1).abs() < EPS && (q.1 - p.1).abs() < EPS;
            if same_x || same_y {
                out.pop();
            }
        }
        out.push(p);
    }
    out
}

/// `(crossings, node hits, bends)` of the routes in `idx`. Members are not compared with each other
/// because their shared runs overlap by design.
fn cost(routes: &[Vec<(f64, f64)>], idx: &[usize], b: &Boxes) -> (usize, usize, usize) {
    let mut crossings = 0;
    let mut hits = 0;
    let mut bends = 0;
    for &i in idx {
        let r = &routes[i];
        bends += turns(r);
        for (o, other) in routes.iter().enumerate() {
            if o == i || idx.contains(&o) {
                continue;
            }
            for s in r.windows(2) {
                crossings += other
                    .windows(2)
                    .filter(|t| segments_cross(s[0], s[1], t[0], t[1]))
                    .count();
            }
        }
        // `hits_one` shrinks boxes by EPS, so an end segment on its own border is no hit.
        for s in r.windows(2) {
            hits += (0..b.n_real).filter(|&v| hits_one(s, b, v)).count();
        }
    }
    (crossings, hits, bends)
}

/// How many times the route changes direction.
fn turns(r: &[(f64, f64)]) -> usize {
    simplify(r.to_vec()).len().saturating_sub(2)
}

/// Whether segment `s` cuts through node `v`'s box.
fn hits_one(s: &[(f64, f64)], b: &Boxes, v: usize) -> bool {
    let (hw, hh) = (b.layer_w[v] / 2.0 - EPS, b.order_w[v] / 2.0 - EPS);
    let (x0, x1) = (s[0].0.min(s[1].0), s[0].0.max(s[1].0));
    let (y0, y1) = (s[0].1.min(s[1].1), s[0].1.max(s[1].1));
    x1 > b.cx[v] - hw && x0 < b.cx[v] + hw && y1 > b.cy[v] - hh && y0 < b.cy[v] + hh
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Three boxes: `0` at the origin, `1` and `2` one layer on, above and below.
    fn boxes() -> Boxes {
        Boxes {
            cx: vec![0.0, 200.0, 200.0],
            cy: vec![0.0, -80.0, 80.0],
            layer_w: vec![80.0, 80.0, 80.0],
            order_w: vec![40.0, 40.0, 40.0],
            ellipse: vec![false; 3],
            n_real: 3,
        }
    }

    #[test]
    fn a_fork_leaves_on_one_trunk_and_branches_off_one_riser() {
        let b = boxes();
        let mut routes = vec![
            vec![
                (40.0, -10.0),
                (100.0, -10.0),
                (100.0, -80.0),
                (160.0, -80.0),
            ],
            vec![(40.0, 10.0), (110.0, 10.0), (110.0, 80.0), (160.0, 80.0)],
        ];
        let ends = vec![(0, 1, true), (0, 2, true)];
        let js = merge_routes(&mut routes, &ends, &b, 15.0, true, false);
        assert_eq!(routes[0][0], (40.0, 0.0));
        assert_eq!(routes[1][0], (40.0, 0.0));
        assert_eq!(routes[0][1], (100.0, 0.0));
        assert_eq!(routes[1][1], (100.0, 0.0));
        assert_eq!(*routes[0].last().unwrap(), (160.0, -80.0));
        assert_eq!(*routes[1].last().unwrap(), (160.0, 80.0));
        assert_eq!(js, vec![(100.0, 0.0)]);
    }

    #[test]
    fn a_join_arrives_on_one_trunk() {
        let b = Boxes {
            cx: vec![0.0, 0.0, 200.0],
            cy: vec![-80.0, 80.0, 0.0],
            layer_w: vec![80.0; 3],
            order_w: vec![40.0; 3],
            ellipse: vec![false; 3],
            n_real: 3,
        };
        let mut routes = vec![
            vec![(40.0, -80.0), (90.0, -80.0), (90.0, -10.0), (160.0, -10.0)],
            vec![(40.0, 80.0), (100.0, 80.0), (100.0, 10.0), (160.0, 10.0)],
        ];
        let ends = vec![(0, 2, true), (1, 2, true)];
        let js = merge_routes(&mut routes, &ends, &b, 15.0, false, true);
        assert_eq!(*routes[0].last().unwrap(), (160.0, 0.0));
        assert_eq!(*routes[1].last().unwrap(), (160.0, 0.0));
        assert_eq!(routes[0][routes[0].len() - 2], (100.0, 0.0));
        assert_eq!(js, vec![(100.0, 0.0)]);
    }

    #[test]
    fn an_edge_arriving_never_shares_a_trunk_with_one_leaving() {
        let b = boxes();
        // The second edge's arrow points back into node 0, so this is one fork end and one join.
        let mut routes = vec![
            vec![
                (40.0, -10.0),
                (100.0, -10.0),
                (100.0, -80.0),
                (160.0, -80.0),
            ],
            vec![(40.0, 10.0), (110.0, 10.0), (110.0, 80.0), (160.0, 80.0)],
        ];
        let kept = routes.clone();
        let ends = vec![(0, 1, true), (0, 2, false)];
        let js = merge_routes(&mut routes, &ends, &b, 15.0, true, true);
        assert!(js.is_empty());
        assert_eq!(routes, kept);
    }

    #[test]
    fn a_merge_that_would_cross_another_edge_is_left_alone() {
        let b = boxes();
        let mut routes = vec![
            vec![
                (40.0, -10.0),
                (100.0, -10.0),
                (100.0, -80.0),
                (160.0, -80.0),
            ],
            vec![(40.0, 10.0), (100.0, 10.0), (100.0, 80.0), (160.0, 80.0)],
            // Crosses the gap where the trunk would go.
            vec![(70.0, -200.0), (70.0, 200.0)],
        ];
        let before = routes.clone();
        let ends = vec![(0, 1, true), (0, 2, true), (usize::MAX, usize::MAX, true)];
        let js = merge_routes(&mut routes, &ends, &b, 15.0, true, false);
        // Each member already crosses it once, so the merged trunk adds no crossing.
        assert_eq!(js.len(), 1);
        // A short riser between the members' original heights only crosses the merged trunk.
        let mut routes = before;
        routes[2] = vec![(60.0, -5.0), (60.0, 5.0)];
        let kept = routes.clone();
        let js = merge_routes(&mut routes, &ends, &b, 15.0, true, false);
        assert!(js.is_empty());
        assert_eq!(routes, kept);
    }
}

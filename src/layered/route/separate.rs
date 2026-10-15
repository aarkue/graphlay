//! Run separation. Two edges whose runs along the layer axis lie on one line read as merging and
//! splitting again, so this pass moves one of the runs off the line.

use crate::geometry::fp;
use std::collections::BTreeMap;

use super::geom::{near_collinear_len, RUN_NEAR};
use super::hints::Hints;
use super::PORT_SPAN;
use crate::geometry::hit::{seg_hits_box, segments_cross_proper};
use crate::geometry::Boxes;
use crate::options::RoutingTuning;
use crate::sort::SortDyn;

/// Shared length above which two runs overlap. Matches the metric's threshold.
const RUN_OVERLAP_MIN: f64 = 2.0;

/// Segment `i -> i + 1` of `arc`'s route, along the layer axis at `y`, spanning `x0..x1`.
#[derive(Clone, Copy)]
struct AxisRun {
    arc: usize,
    i: usize,
    y: f64,
    x0: f64,
    x1: f64,
}

/// Every layer-axis run in `routes`, sorted by cross-position so a sweep finds shared lines.
fn axis_runs(routes: &[Vec<(f64, f64)>]) -> Vec<AxisRun> {
    let mut runs = vec![];
    for (arc, r) in routes.iter().enumerate() {
        for i in 0..r.len().saturating_sub(1) {
            if (r[i].1 - r[i + 1].1).abs() < 0.5 && (r[i].0 - r[i + 1].0).abs() >= 0.5 {
                runs.push(AxisRun {
                    arc,
                    i,
                    y: r[i].1,
                    x0: r[i].0.min(r[i + 1].0),
                    x1: r[i].0.max(r[i + 1].0),
                });
            }
        }
    }
    runs.sort_unstable_by_dyn(|a, b| {
        a.y.total_cmp(&b.y)
            .then(a.arc.cmp(&b.arc))
            .then(a.i.cmp(&b.i))
    });
    runs
}

/// Cost of one route: foreign box hits, shared runs, then proper crossings.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
struct RunPrice {
    hits: usize,
    overlaps: usize,
    crossings: usize,
}

/// One candidate move of a run off a shared line.
struct RunMove {
    route: Vec<(f64, f64)>,
    bends: usize,
    moved: f64,
}

/// Price of `pts` as `arc`'s route against `routes`. Cutting a hinted end's own node counts as a
/// box hit.
fn run_price(
    pts: &[(f64, f64)],
    arc: usize,
    ends: (usize, usize),
    routes: &[Vec<(f64, f64)>],
    b: &Boxes,
    hints: &Hints,
) -> RunPrice {
    let mut price = RunPrice {
        hits: usize::from(hints.hinted(arc) && hints.cuts_own_boxes(arc, pts, b)),
        ..RunPrice::default()
    };
    for w in pts.windows(2) {
        if seg_hits_box(w[0], w[1], b, ends.0, ends.1) {
            price.hits += 1;
        }
        for (other, o) in routes.iter().enumerate() {
            if other == arc {
                continue;
            }
            for v in o.windows(2) {
                if segments_cross_proper(w[0], w[1], v[0], v[1]) {
                    price.crossings += 1;
                } else if near_collinear_len(w[0], w[1], v[0], v[1]) > RUN_OVERLAP_MIN {
                    price.overlaps += 1;
                }
            }
        }
    }
    price
}

/// Jog height for separating a run in place, the smallest step that reads as deliberate.
const SEPARATION_JOG: f64 = 12.0;

/// Pull apart layer-axis runs that two edges draw on one line, where they may hide a crossing.
/// A move must remove an overlap without a box hit and add at most the crossing it was hiding.
pub(super) fn separate_axis_runs(
    routes: &mut [Vec<(f64, f64)>],
    chains: &BTreeMap<usize, Vec<usize>>,
    b: &Boxes,
    port_gap: f64,
    r: &RoutingTuning,
    hints: &Hints,
) {
    let step = r.overlap_repair_step;
    let jog = SEPARATION_JOG.max(step);
    if step <= 0.0 || r.overlap_repair_rounds == 0 {
        return;
    }
    let ends_of = |arc: usize| -> Option<(usize, usize)> {
        chains
            .get(&arc)
            .filter(|c| c.len() >= 2)
            .map(|c| (c[0], c[c.len() - 1]))
    };
    // Same neighbour rule as the step flattener. No port slides to within `port_gap` of another.
    let mut anchors_of: BTreeMap<usize, Vec<(f64, f64)>> = BTreeMap::new();
    for (&arc, chain) in chains {
        if let (Some(&first), Some(&last), Some(route)) =
            (chain.first(), chain.last(), routes.get(arc))
        {
            if chain.len() >= 2 && route.len() >= 2 {
                anchors_of.entry(first).or_default().push(route[0]);
                anchors_of
                    .entry(last)
                    .or_default()
                    .push(route[route.len() - 1]);
            }
        }
    }
    let dist = |a: (f64, f64), c: (f64, f64)| fp::hypot(a.0 - c.0, a.1 - c.1);
    let slide_ok = |node: usize, old: (f64, f64), new: (f64, f64)| -> bool {
        node < b.n_real
            && (new.1 - b.cy[node]).abs() <= b.order_w[node] * PORT_SPAN
            && anchors_of.get(&node).is_none_or(|anchors| {
                anchors
                    .iter()
                    .filter(|&&a| a != old)
                    .all(|&a| dist(a, new) >= port_gap.min(dist(a, old)))
            })
    };

    for _ in 0..r.overlap_repair_rounds {
        let runs = axis_runs(routes);
        let mut modified: Vec<bool> = vec![false; routes.len()];
        let mut any = false;
        for p in 0..runs.len() {
            for q in p + 1..runs.len() {
                let (rp, rq) = (runs[p], runs[q]);
                if rq.y - rp.y >= RUN_NEAR {
                    break;
                }
                if rp.arc == rq.arc || modified[rp.arc] || modified[rq.arc] {
                    continue;
                }
                let (lo, hi) = (rp.x0.max(rq.x0), rp.x1.min(rq.x1));
                if hi - lo <= RUN_OVERLAP_MIN {
                    continue;
                }
                type Key = (usize, usize, usize, u32, usize);
                let mut best: Option<(Key, usize, RunMove)> = None;
                for run in [rp, rq] {
                    let Some(ends) = ends_of(run.arc) else {
                        continue;
                    };
                    let route = &routes[run.arc];
                    let before = run_price(route, run.arc, ends, routes, b, hints);
                    let mut trials = run_moves(route, run, ends, (lo, hi), (step, jog), &slide_ok);
                    if hints.hinted(run.arc) {
                        let (head, tail) =
                            (hints.pinned(run.arc, true), hints.pinned(run.arc, false));
                        trials.retain(|mv| {
                            (!head || mv.route.first() == route.first())
                                && (!tail || mv.route.last() == route.last())
                                && hints.keeps_ends_straight(run.arc, route, &mv.route, b)
                                && slant(&mv.route) <= slant(route) + 1e-6
                        });
                    }
                    for mv in trials {
                        let after = run_price(&mv.route, run.arc, ends, routes, b, hints);
                        if after.hits > before.hits
                            || after.overlaps >= before.overlaps
                            || after.crossings + after.overlaps > before.crossings + before.overlaps
                        {
                            continue;
                        }
                        let key: Key = (
                            after.crossings.saturating_sub(before.crossings),
                            after.overlaps,
                            mv.bends,
                            mv.moved.round() as u32,
                            usize::from(run.arc == rq.arc),
                        );
                        if best.as_ref().is_none_or(|(k, _, _)| key < *k) {
                            best = Some((key, run.arc, mv));
                        }
                    }
                }
                if let Some((_, arc, mv)) = best {
                    routes[arc] = mv.route;
                    modified[arc] = true;
                    any = true;
                }
            }
        }
        if !any {
            break;
        }
    }
}

/// Sum of each segment's smaller extent. A move next to a held hinted port must not add slant.
fn slant(route: &[(f64, f64)]) -> f64 {
    route
        .windows(2)
        .map(|w| (w[0].0 - w[1].0).abs().min((w[0].1 - w[1].1).abs()))
        .sum()
}

/// Whether the neighbouring segment keeps its direction and does not shrink into a step the
/// flattener would have removed. A riser shorter than the move would fold back on itself.
fn keeps_direction(from: f64, to: f64, moved_to: f64) -> bool {
    let (old_len, new_len) = ((to - from).abs(), (to - moved_to).abs());
    new_len >= old_len.clamp(0.5, SEPARATION_JOG) && ((to - from) > 0.0) == ((to - moved_to) > 0.0)
}

/// Candidate moves of `run` off the line it shares over `lo..hi`: a whole shift or port slide by
/// one or two `step`s, or a jog of its lane-side part by `jog`.
fn run_moves(
    route: &[(f64, f64)],
    run: AxisRun,
    ends: (usize, usize),
    (lo, hi): (f64, f64),
    (step, jog): (f64, f64),
    slide_ok: &impl Fn(usize, (f64, f64), (f64, f64)) -> bool,
) -> Vec<RunMove> {
    let n = route.len();
    let (i, y) = (run.i, run.y);
    let (li, ri) = if route[i].0 <= route[i + 1].0 {
        (i, i + 1)
    } else {
        (i + 1, i)
    };
    let is_anchor = |k: usize| k == 0 || k + 1 == n;
    let mut trials: Vec<RunMove> = vec![];
    for sign in [1.0, -1.0] {
        let anchors = usize::from(is_anchor(i)) + usize::from(is_anchor(i + 1));
        for steps in [1.0, 2.0] {
            let d = sign * steps * step;
            if anchors > 1 {
                break;
            }
            let mut t = route.to_vec();
            t[i].1 += d;
            t[i + 1].1 += d;
            let ok = (i == 0 || keeps_direction(y, route[i - 1].1, y + d))
                && (i + 2 >= n || keeps_direction(y, route[i + 2].1, y + d))
                && (anchors == 0 || {
                    let (k, node) = if i == 0 { (0, ends.0) } else { (n - 1, ends.1) };
                    slide_ok(node, route[k], t[k])
                });
            if ok {
                trials.push(RunMove {
                    route: t,
                    bends: 0,
                    moved: steps * step,
                });
            }
        }
        // What stays on the line must be long enough for an arrowhead at a port, and must not
        // become a step between the jog and a corner's riser.
        let d = sign * jog;
        for margin in [jog, 1.5 * jog] {
            let (xj, part_right) = if lo - margin >= route[li].0 + jog && !is_anchor(ri) {
                (lo - margin, true)
            } else if hi + margin <= route[ri].0 - jog && !is_anchor(li) {
                (hi + margin, false)
            } else {
                continue;
            };
            let corner = if part_right { ri } else { li };
            let next = if corner == i {
                i.checked_sub(1)
            } else {
                Some(i + 2)
            };
            if next.is_some_and(|k| !keeps_direction(y, route[k].1, y + d)) {
                continue;
            }
            let mut t = route.to_vec();
            t[corner].1 = y + d;
            let (near_pt, far_pt) = ((xj, y), (xj, y + d));
            if corner == i + 1 {
                t.splice(i + 1..i + 1, [near_pt, far_pt]);
            } else {
                t.splice(i + 1..i + 1, [far_pt, near_pt]);
            }
            trials.push(RunMove {
                route: t,
                bends: 2,
                moved: jog,
            });
        }
    }
    trials
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Two layers of 40x60 boxes 300 apart, nodes 0-1 at cross 0 and 100, nodes 2-4 at 0, 100
    /// and 200. Arcs past the first two are stubs that only hold a port on a border.
    fn two_layers(chains: &[(usize, usize)]) -> (Boxes, BTreeMap<usize, Vec<usize>>) {
        let b = Boxes {
            cx: vec![0.0, 0.0, 300.0, 300.0, 300.0],
            cy: vec![0.0, 100.0, 0.0, 100.0, 200.0],
            layer_w: vec![40.0; 5],
            order_w: vec![60.0; 5],
            ellipse: vec![false; 5],
            n_real: 5,
        };
        let chains = chains
            .iter()
            .enumerate()
            .map(|(arc, &(from, to))| (arc, vec![from, to]))
            .collect();
        (b, chains)
    }

    fn overlaps_between(a: &[(f64, f64)], b: &[(f64, f64)]) -> usize {
        a.windows(2)
            .flat_map(|w| b.windows(2).map(move |v| (w, v)))
            .filter(|(w, v)| near_collinear_len(w[0], w[1], v[0], v[1]) > RUN_OVERLAP_MIN)
            .count()
    }

    fn crossings_between(a: &[(f64, f64)], b: &[(f64, f64)]) -> usize {
        a.windows(2)
            .flat_map(|w| b.windows(2).map(move |v| (w, v)))
            .filter(|(w, v)| segments_cross_proper(w[0], w[1], v[0], v[1]))
            .count()
    }

    /// Arcs 0 and 1 swap columns, so they must cross, and their runs at 90 share a line. A port
    /// slide separates them without a new bend and leaves the crossing visible.
    #[test]
    fn a_hidden_crossing_on_a_shared_column_is_revealed_by_a_port_slide() {
        let (b, chains) = two_layers(&[(0, 3), (1, 2)]);
        let mut routes = vec![
            vec![(20.0, 10.0), (120.0, 10.0), (120.0, 90.0), (280.0, 90.0)],
            vec![(20.0, 90.0), (180.0, 90.0), (180.0, 10.0), (280.0, 10.0)],
        ];
        assert_eq!(overlaps_between(&routes[0], &routes[1]), 1);
        assert_eq!(crossings_between(&routes[0], &routes[1]), 0);
        separate_axis_runs(
            &mut routes,
            &chains,
            &b,
            18.0,
            &RoutingTuning::default(),
            &Hints::none(),
        );
        assert_eq!(overlaps_between(&routes[0], &routes[1]), 0, "{routes:?}");
        assert_eq!(crossings_between(&routes[0], &routes[1]), 1, "{routes:?}");
        assert_eq!(
            routes[0].len() + routes[1].len(),
            8,
            "no bend added: {routes:?}"
        );
        for (arc, r) in routes.iter().enumerate() {
            let (s, t) = (r[0], r[r.len() - 1]);
            let (from, to) = (chains[&arc][0], chains[&arc][1]);
            assert_eq!((s.0, t.0), (20.0, 280.0));
            assert!((s.1 - b.cy[from]).abs() <= 27.0 && (t.1 - b.cy[to]).abs() <= 27.0);
        }
    }

    /// With every border packed no port may slide, so one run's lane-side part jogs instead.
    #[test]
    fn on_a_packed_border_the_lane_side_part_jogs_instead() {
        // Neighbour stubs 18 either side of each port. Their far ends are not on any node.
        let mut chains = vec![(0, 3), (1, 2)];
        let mut routes = vec![
            vec![(20.0, 10.0), (120.0, 10.0), (120.0, 90.0), (280.0, 90.0)],
            vec![(20.0, 90.0), (180.0, 90.0), (180.0, 10.0), (280.0, 10.0)],
        ];
        for (node, port_y) in [(0usize, 10.0), (1, 90.0), (2, 10.0), (3, 90.0)] {
            for off in [-18.0, 18.0] {
                let y = port_y + off;
                if node < 2 {
                    chains.push((node, 2));
                    routes.push(vec![(20.0, y), (50.0, y)]);
                } else {
                    chains.push((0, node));
                    routes.push(vec![(250.0, y), (280.0, y)]);
                }
            }
        }
        let (b, chains) = two_layers(&chains);
        let before = routes.clone();
        separate_axis_runs(
            &mut routes,
            &chains,
            &b,
            18.0,
            &RoutingTuning::default(),
            &Hints::none(),
        );
        assert_eq!(overlaps_between(&routes[0], &routes[1]), 0, "{routes:?}");
        assert_eq!(crossings_between(&routes[0], &routes[1]), 1, "{routes:?}");
        assert_eq!(routes[0].len() + routes[1].len(), 10, "one jog: {routes:?}");
        for arc in 0..2 {
            let (r, was) = (&routes[arc], &before[arc]);
            assert_eq!((r[0], r[r.len() - 1]), (was[0], was[was.len() - 1]));
            let last = (r[r.len() - 1].0 - r[r.len() - 2].0).abs();
            let first = (r[1].0 - r[0].0).abs();
            assert!(first >= 12.0 && last >= 12.0, "port runs too short: {r:?}");
        }
        assert_eq!(routes[2..], before[2..], "the stubs were not touched");
    }

    /// The two arcs share a run at 90 but need not cross, so the separation adds no crossing.
    #[test]
    fn a_shared_column_that_hides_no_crossing_is_separated_for_free() {
        let (b, chains) = two_layers(&[(0, 3), (1, 4)]);
        let mut routes = vec![
            vec![(20.0, 10.0), (60.0, 10.0), (60.0, 90.0), (280.0, 90.0)],
            vec![(20.0, 90.0), (120.0, 90.0), (120.0, 190.0), (280.0, 190.0)],
        ];
        assert_eq!(overlaps_between(&routes[0], &routes[1]), 1);
        separate_axis_runs(
            &mut routes,
            &chains,
            &b,
            18.0,
            &RoutingTuning::default(),
            &Hints::none(),
        );
        assert_eq!(overlaps_between(&routes[0], &routes[1]), 0, "{routes:?}");
        assert_eq!(crossings_between(&routes[0], &routes[1]), 0, "{routes:?}");
        assert_eq!(
            routes[0].len() + routes[1].len(),
            8,
            "no bend added: {routes:?}"
        );
    }

    #[test]
    fn separate_axis_runs_leaves_a_clean_drawing_alone() {
        let (b, chains) = two_layers(&[(0, 2), (1, 3)]);
        let before = vec![
            vec![(20.0, 0.0), (280.0, 0.0)],
            vec![
                (20.0, 100.0),
                (150.0, 100.0),
                (150.0, 110.0),
                (280.0, 110.0),
            ],
        ];
        let mut routes = before.clone();
        separate_axis_runs(
            &mut routes,
            &chains,
            &b,
            18.0,
            &RoutingTuning::default(),
            &Hints::none(),
        );
        assert_eq!(routes, before);
    }
}

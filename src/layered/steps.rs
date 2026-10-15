//! Flattens the few-pixel steps an orthogonal route picks up.
//!
//! Waypoints straightened within their slots and ports spread along a border both leave small Zs
//! that read as jitter. [`flatten_short_steps`] moves one run onto the line of the next so the
//! route loses two bends. Steps above the threshold are real detours and stay.

use crate::geometry::polyline::simplify_collinear;
use crate::sort::SortDyn;

/// `Some(true)` for a vertical segment, `Some(false)` for a horizontal one, `None` otherwise.
fn axis(p: (f64, f64), q: (f64, f64)) -> Option<bool> {
    let (dx, dy) = ((q.0 - p.0).abs(), (q.1 - p.1).abs());
    if dx < 0.5 && dy >= 0.5 {
        Some(true)
    } else if dy < 0.5 && dx >= 0.5 {
        Some(false)
    } else {
        None
    }
}

/// Removes every step shorter than `max_step` from `route` where a move passes all vetoes.
///
/// `head_along_y` and `tail_along_y` give the axis of each endpoint's border, the only axis that
/// endpoint may slide on. `slide_ok(is_head, new_anchor)` vetoes a slide that would crowd the
/// node's other ports, `hits` vetoes a new segment through a box, and `accept(old, new)` vetoes a
/// whole move that would cross or overlap another edge. Diagonal segments are never part of a step.
pub fn flatten_short_steps(
    route: &mut Vec<(f64, f64)>,
    head_along_y: bool,
    tail_along_y: bool,
    max_step: f64,
    slide_ok: impl Fn(bool, (f64, f64)) -> bool,
    hits: impl Fn((f64, f64), (f64, f64)) -> bool,
    accept: impl Fn(&[(f64, f64)], &[(f64, f64)]) -> bool,
) {
    if max_step <= 0.0 {
        return;
    }
    // Each pass removes one step, and a route has at most one step per segment.
    for _ in 0..route.len().max(1) {
        let n = route.len();
        if n < 4 {
            return;
        }
        let mut changed = false;
        for i in 1..n - 2 {
            // Segment i is the step, and i-1 and i+1 are the two runs it joins.
            let Some(step_vertical) = axis(route[i], route[i + 1]) else {
                continue;
            };
            let len = if step_vertical {
                (route[i + 1].1 - route[i].1).abs()
            } else {
                (route[i + 1].0 - route[i].0).abs()
            };
            if len >= max_step {
                continue;
            }
            if axis(route[i - 1], route[i]) != Some(!step_vertical)
                || axis(route[i + 1], route[i + 2]) != Some(!step_vertical)
            {
                continue;
            }
            // Move run A (i-1 -> i) onto B's line or B (i+1 -> i+2) onto A's. Moving an end run
            // slides its port, which is only allowed along that port's border.
            let anchor_ok_head = i - 1 == 0 && (head_along_y == step_vertical);
            let anchor_ok_tail = i + 2 == n - 1 && (tail_along_y == step_vertical);
            let run_len = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() + (a.1 - b.1).abs();
            let mut cands: Vec<(bool, f64, bool)> = vec![]; // (touches anchor, run length, move A)
            if i - 1 > 0 || anchor_ok_head {
                cands.push((i - 1 == 0, run_len(route[i - 1], route[i]), true));
            }
            if i + 2 < n - 1 || anchor_ok_tail {
                cands.push((i + 2 == n - 1, run_len(route[i + 1], route[i + 2]), false));
            }
            // Prefer keeping ports, then the shorter run, then the head move as a total tiebreak.
            cands.sort_unstable_by_dyn(|a, b| {
                a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(b.2.cmp(&a.2))
            });
            for (_, _, move_a) in cands {
                let mut trial = route.clone();
                let (lo, hi, onto) = if move_a {
                    (i - 1, i, route[i + 1])
                } else {
                    (i + 1, i + 2, route[i])
                };
                for p in &mut trial[lo..=hi] {
                    if step_vertical {
                        p.1 = onto.1;
                    } else {
                        p.0 = onto.0;
                    }
                }
                if (move_a && i - 1 == 0 && !slide_ok(true, trial[0]))
                    || (!move_a && i + 2 == n - 1 && !slide_ok(false, trial[n - 1]))
                {
                    continue;
                }
                let trial = simplify_collinear(trial);
                // Only new segments are checked, so a hit the route already had blocks nothing.
                let existed =
                    |w: &[(f64, f64)]| route.windows(2).any(|o| o[0] == w[0] && o[1] == w[1]);
                let clear = !trial.windows(2).any(|w| !existed(w) && hits(w[0], w[1]));
                if clear && accept(route, &trial) {
                    *route = trial;
                    changed = true;
                    break;
                }
            }
            if changed {
                break;
            }
        }
        if !changed {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn never(_: (f64, f64), _: (f64, f64)) -> bool {
        false
    }

    fn any_slide(_: bool, _: (f64, f64)) -> bool {
        true
    }

    /// An interior step is absorbed by the riser before it.
    #[test]
    fn an_interior_step_is_flattened_into_the_riser() {
        let mut r = vec![
            (0.0, 100.0),
            (40.0, 100.0),
            (40.0, 50.0),
            (120.0, 50.0),
            (120.0, 44.0),
            (200.0, 44.0),
            (200.0, 10.0),
            (240.0, 10.0),
        ];
        flatten_short_steps(&mut r, true, true, 12.0, any_slide, never, |_, _| true);
        assert_eq!(
            r,
            vec![
                (0.0, 100.0),
                (40.0, 100.0),
                (40.0, 44.0),
                (200.0, 44.0),
                (200.0, 10.0),
                (240.0, 10.0)
            ]
        );
    }

    /// For a Z at a port, the long run moves onto the port's line and the port stays.
    #[test]
    fn a_port_jog_moves_the_run_not_the_port() {
        let mut r = vec![
            (0.0, 50.0),
            (30.0, 50.0),
            (30.0, 100.0),
            (300.0, 100.0),
            (300.0, 108.0),
            (340.0, 108.0),
        ];
        flatten_short_steps(&mut r, true, true, 12.0, any_slide, never, |_, _| true);
        assert_eq!(
            r,
            vec![(0.0, 50.0), (30.0, 50.0), (30.0, 108.0), (340.0, 108.0)]
        );
    }

    /// A lone Z between two ports on vertical borders slides the shorter end along its border.
    #[test]
    fn a_lone_z_slides_the_nearer_port_along_its_border() {
        let mut r = vec![(0.0, 50.0), (300.0, 50.0), (300.0, 58.0), (340.0, 58.0)];
        flatten_short_steps(&mut r, true, true, 12.0, any_slide, never, |_, _| true);
        assert_eq!(r, vec![(0.0, 50.0), (340.0, 50.0)]);
        let mut r = vec![(0.0, 50.0), (300.0, 50.0), (300.0, 58.0), (340.0, 58.0)];
        flatten_short_steps(&mut r, true, true, 12.0, |_, _| false, never, |_, _| true);
        assert_eq!(r.len(), 4);
        // Ports on horizontal borders may not slide in y.
        let mut r = vec![(0.0, 50.0), (300.0, 50.0), (300.0, 58.0), (340.0, 58.0)];
        flatten_short_steps(&mut r, false, false, 12.0, any_slide, never, |_, _| true);
        assert_eq!(r.len(), 4);
    }

    #[test]
    fn a_tall_step_is_left_alone() {
        let mut r = vec![(0.0, 50.0), (300.0, 50.0), (300.0, 90.0), (340.0, 90.0)];
        flatten_short_steps(&mut r, true, true, 12.0, any_slide, never, |_, _| true);
        assert_eq!(r.len(), 4);
    }

    /// A blocked move falls back to the other move, and the step stays when both are blocked.
    #[test]
    fn a_blocked_move_is_skipped() {
        let z = || vec![(0.0, 50.0), (300.0, 50.0), (300.0, 58.0), (340.0, 58.0)];
        // Veto the preferred move, which makes a 340px run at y=50.
        let blocks_50 =
            |p: (f64, f64), q: (f64, f64)| p.1 == 50.0 && q.1 == 50.0 && (q.0 - p.0).abs() > 320.0;
        let mut r = z();
        flatten_short_steps(&mut r, true, true, 12.0, any_slide, blocks_50, |_, _| true);
        assert_eq!(r, vec![(0.0, 58.0), (340.0, 58.0)]);
        let mut r = z();
        flatten_short_steps(
            &mut r,
            true,
            true,
            12.0,
            any_slide,
            |_, _| true,
            |_, _| true,
        );
        assert_eq!(r.len(), 4);
    }

    /// `accept` can refuse a move that clears every box.
    #[test]
    fn a_refused_move_leaves_the_route_as_it_was() {
        let mut r = vec![
            (0.0, 0.0),
            (50.0, 0.0),
            (50.0, 200.0),
            (100.0, 200.0),
            (100.0, 400.0),
            (150.0, 400.0),
        ];
        let before = r.clone();
        flatten_short_steps(&mut r, true, true, 1.0e6, any_slide, never, |_, _| false);
        assert_eq!(r, before);
        flatten_short_steps(&mut r, true, true, 1.0e6, any_slide, never, |_, _| true);
        assert!(
            r.len() < before.len(),
            "an accepted move merges a step: {r:?}"
        );
    }
}

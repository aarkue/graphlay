//! Isotonic (monotone) projection kernels.
//!
//! [`project_gaps`] and its variants use pool-adjacent-violators to find the positions closest to
//! the requested ones that keep minimum gaps between neighbours. Weights pick which nodes absorb a
//! squeeze (e.g., real nodes before a long edge's waypoints, where a moved pixel shows as a jog).

/// Positions `p` minimising `sum (p_i - d_i)^2` subject to `p_{i+1} - p_i >= gap_i`.
pub fn project_gaps(d: &[f64], gap: &[f64]) -> Vec<f64> {
    project(d, gap, &[])
}

/// [`project_gaps`] minimising `sum w_i (p_i - d_i)^2`. A missing, non-finite or non-positive
/// weight reads as `1.0`, so bad weights never produce NaN.
pub fn project_gaps_weighted(d: &[f64], gap: &[f64], w: &[f64]) -> Vec<f64> {
    project(d, gap, w)
}

fn weight_at(w: &[f64], i: usize) -> f64 {
    match w.get(i) {
        Some(&wi) if wi.is_finite() && wi > 0.0 => wi,
        _ => 1.0,
    }
}

/// PAVA on the gap-shifted sequence. An empty `w` means uniform weights.
fn project(d: &[f64], gap: &[f64], w: &[f64]) -> Vec<f64> {
    let m = d.len();
    if m == 0 {
        return vec![];
    }
    // Subtracting the cumulative gaps turns the problem into a plain monotone fit on y.
    let mut off = vec![0.0; m];
    for i in 1..m {
        off[i] = off[i - 1] + gap.get(i - 1).copied().unwrap_or(0.0);
    }
    let y: Vec<f64> = (0..m).map(|i| d[i] - off[i]).collect();
    // With weights a block's total weight differs from its node count, so both are kept.
    let mut val: Vec<f64> = vec![];
    let mut wsum: Vec<f64> = vec![];
    let mut cnt: Vec<usize> = vec![];
    for (i, &yi) in y.iter().enumerate() {
        val.push(yi);
        wsum.push(weight_at(w, i));
        cnt.push(1);
        while val.len() > 1 && val[val.len() - 2] > val[val.len() - 1] {
            let (Some(v1), Some(w1), Some(n1), Some(v0), Some(w0), Some(n0)) = (
                val.pop(),
                wsum.pop(),
                cnt.pop(),
                val.pop(),
                wsum.pop(),
                cnt.pop(),
            ) else {
                break;
            };
            val.push((v0 * w0 + v1 * w1) / (w0 + w1));
            wsum.push(w0 + w1);
            cnt.push(n0 + n1);
        }
    }
    let mut out = Vec::with_capacity(m);
    for (b, &v) in val.iter().enumerate() {
        for _ in 0..cnt[b] {
            let i = out.len();
            out.push(v + off[i]);
        }
    }
    out
}

/// [`project_gaps`] with nodes fixed where `fixed` is `Some`. Each free run is projected on its
/// own, then shifted to clear its fixed neighbours.
pub fn project_gaps_pinned(d: &[f64], gap: &[f64], fixed: &[Option<f64>]) -> Vec<f64> {
    let m = d.len();
    if m == 0 {
        return vec![];
    }
    if fixed.iter().all(Option::is_none) {
        return project_gaps(d, gap);
    }
    let mut out = vec![0.0; m];
    let mut i = 0;
    while i < m {
        if let Some(f) = fixed[i] {
            out[i] = f;
            i += 1;
            continue;
        }
        let mut j = i;
        while j < m && fixed[j].is_none() {
            j += 1;
        }
        let mut p = project_gaps(&d[i..j], &gap[i..j.saturating_sub(1)]);
        if i > 0 {
            let lo = out[i - 1] + gap[i - 1];
            if p[0] < lo {
                let s = lo - p[0];
                for x in p.iter_mut() {
                    *x += s;
                }
            }
        }
        if let Some(Some(f)) = fixed.get(j).copied() {
            let hi = f - gap[j - 1];
            let last = p.len() - 1;
            if p[last] > hi {
                let s = p[last] - hi;
                for x in p.iter_mut() {
                    *x -= s;
                }
            }
        }
        for (k, &v) in p.iter().enumerate() {
            out[i + k] = v;
        }
        i = j;
    }
    out
}

/// Monotone version of `ys` (non-decreasing if `increasing`), the mean of the forward and backward
/// envelopes, so a track never doubles back.
pub fn monotone_project(ys: &[f64], increasing: bool) -> Vec<f64> {
    let n = ys.len();
    if n == 0 {
        return vec![];
    }
    let s: Vec<f64> = if increasing {
        ys.to_vec()
    } else {
        ys.iter().map(|y| -y).collect()
    };
    let mut fwd = s.clone();
    for i in 1..n {
        if fwd[i] < fwd[i - 1] {
            fwd[i] = fwd[i - 1];
        }
    }
    let mut bwd = s.clone();
    for i in (0..n - 1).rev() {
        if bwd[i] > bwd[i + 1] {
            bwd[i] = bwd[i + 1];
        }
    }
    (0..n)
        .map(|i| {
            let v = (fwd[i] + bwd[i]) / 2.0;
            if increasing {
                v
            } else {
                -v
            }
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn project_gaps_spreads_a_pile_symmetrically() {
        assert_eq!(
            project_gaps(&[0.0, 0.0, 0.0], &[10.0, 10.0]),
            vec![-10.0, 0.0, 10.0]
        );
    }

    #[test]
    fn project_gaps_is_identity_when_feasible() {
        assert_eq!(
            project_gaps(&[0.0, 10.0, 20.0], &[5.0, 5.0]),
            vec![0.0, 10.0, 20.0]
        );
    }

    #[test]
    fn project_gaps_handles_the_empty_layer() {
        assert!(project_gaps(&[], &[]).is_empty());
    }

    #[test]
    fn project_gaps_pinned_holds_the_pin_and_shifts_the_run() {
        let out = project_gaps_pinned(&[100.0, 0.0], &[10.0], &[Some(0.0), None]);
        assert_eq!(out, vec![0.0, 10.0]);
    }

    #[test]
    fn project_gaps_pinned_without_pins_is_project_gaps() {
        let (d, gap) = ([0.0, 0.0, 0.0], [10.0, 10.0]);
        assert_eq!(
            project_gaps_pinned(&d, &gap, &[None, None, None]),
            project_gaps(&d, &gap)
        );
    }

    #[test]
    fn monotone_project_averages_the_two_envelopes() {
        assert_eq!(
            monotone_project(&[0.0, 5.0, 3.0, 8.0], true),
            vec![0.0, 4.0, 4.0, 8.0]
        );
    }

    #[test]
    fn monotone_project_mirrors_for_decreasing() {
        let up = monotone_project(&[0.0, 5.0, 3.0, 8.0], true);
        let down = monotone_project(&[0.0, -5.0, -3.0, -8.0], false);
        assert_eq!(down, up.iter().map(|v| -v).collect::<Vec<_>>());
    }

    #[test]
    fn project_gaps_weighted_moves_the_cheap_nodes() {
        let out = project_gaps_weighted(&[0.0, 0.0, 0.0], &[10.0, 10.0], &[1.0, 100.0, 1.0]);
        assert!((out[1] - 0.0).abs() < 0.2, "{out:?}");
        assert!((out[0] + 10.0).abs() < 0.2, "{out:?}");
        assert!((out[2] - 10.0).abs() < 0.2, "{out:?}");
        assert!(out[1] - out[0] >= 10.0 - 1e-9 && out[2] - out[1] >= 10.0 - 1e-9);
    }

    #[test]
    fn project_gaps_weighted_pins_softly() {
        let out = project_gaps_weighted(&[50.0, 50.0], &[10.0], &[1.0, 1000.0]);
        assert!((out[1] - 50.0).abs() < 0.1, "{out:?}");
        assert!((out[0] - 40.0).abs() < 0.1, "{out:?}");
    }

    #[test]
    fn project_gaps_weighted_with_equal_weights_is_project_gaps() {
        let (d, gap) = ([5.0, 0.0, 12.0, 3.0], [10.0, 10.0, 10.0]);
        assert_eq!(
            project_gaps_weighted(&d, &gap, &[2.0, 2.0, 2.0, 2.0]),
            project_gaps(&d, &gap)
        );
        assert_eq!(
            project_gaps_weighted(&d, &gap, &[f64::NAN, 0.0, -1.0, 1.0]),
            project_gaps(&d, &gap)
        );
    }

    #[test]
    fn monotone_project_is_identity_on_a_monotone_input() {
        assert_eq!(
            monotone_project(&[1.0, 2.0, 3.0], true),
            vec![1.0, 2.0, 3.0]
        );
        assert!(monotone_project(&[], true).is_empty());
    }
}

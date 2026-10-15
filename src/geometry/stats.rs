//! Order statistics and barycentres. Alignment uses the weighted median because it lands on a
//! sample, so an edge comes out straight.

use crate::sort::SortDyn;
use std::cmp::Ordering;

/// Total order on `(f64, f64)` by `total_cmp`, first component first.
/// A shared fn item keeps one `sort` instantiation, which saves wasm code size.
pub fn pair_order(a: &(f64, f64), b: &(f64, f64)) -> Ordering {
    a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1))
}

/// Weighted median of `(value, weight)` samples, which minimises `sum w*|m - value|`.
pub fn weighted_median(samples: &mut [(f64, f64)]) -> f64 {
    samples.sort_unstable_by_dyn(pair_order);
    let total: f64 = samples.iter().map(|s| s.1).sum();
    let mut acc = 0.0;
    for &(v, w) in samples.iter() {
        acc += w;
        if acc * 2.0 >= total {
            return v;
        }
    }
    samples.last().map(|s| s.0).unwrap_or(0.0)
}

/// Median of `values`, sorted in place, or `0.0` when empty. An even count averages the middles.
pub fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_unstable_by_dyn(f64::total_cmp);
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        values[mid]
    } else {
        (values[mid - 1] + values[mid]) / 2.0
    }
}

/// Barycentre of `nb[node]` at positions `pos`, or `node`'s own position when it has none.
pub fn barycentre(nb: &[Vec<usize>], node: usize, pos: &[usize]) -> f64 {
    let ns = &nb[node];
    if ns.is_empty() {
        pos[node] as f64
    } else {
        ns.iter().map(|&j| pos[j] as f64).sum::<f64>() / ns.len() as f64
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn weighted_median_lands_on_a_sample() {
        assert_eq!(
            weighted_median(&mut [(1.0, 1.0), (2.0, 1.0), (3.0, 1.0)]),
            2.0
        );
        assert_eq!(weighted_median(&mut [(1.0, 1.0), (3.0, 1.0)]), 1.0);
    }

    #[test]
    fn weighted_median_follows_the_weight() {
        assert_eq!(weighted_median(&mut [(1.0, 1.0), (10.0, 5.0)]), 10.0);
        assert_eq!(weighted_median(&mut [(1.0, 5.0), (10.0, 1.0)]), 1.0);
    }

    #[test]
    fn median_averages_the_middles() {
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&mut [3.0, 1.0]), 2.0);
        assert_eq!(median(&mut []), 0.0);
    }

    #[test]
    fn barycentre_falls_back_to_the_own_position() {
        let nb = vec![vec![1, 2], vec![], vec![]];
        let pos = [5, 1, 4];
        assert_eq!(barycentre(&nb, 0, &pos), 2.5);
        assert_eq!(barycentre(&nb, 1, &pos), 1.0);
    }

    #[test]
    fn weighted_median_is_order_independent() {
        let mut a = [(3.0, 2.0), (1.0, 1.0), (2.0, 1.0)];
        let mut b = [(2.0, 1.0), (3.0, 2.0), (1.0, 1.0)];
        assert_eq!(weighted_median(&mut a), weighted_median(&mut b));
        assert_eq!(weighted_median(&mut []), 0.0);
    }
}

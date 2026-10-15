//! Sort helpers that keep one sort instantiation in the binary instead of one per call site.
//!
//! In a wasm build the per-closure std sort copies outweigh any layout phase in code size. These
//! helpers sort `u32` positions through a `&mut dyn FnMut` comparator, then permute the elements.

use core::cmp::Ordering;

/// Sorting through one shared instantiation. See the module documentation.
pub(crate) trait SortDyn<T> {
    /// `sort_unstable_by`, through one shared instantiation.
    fn sort_unstable_by_dyn(&mut self, cmp: impl FnMut(&T, &T) -> Ordering);
    /// `sort_by` (stable), through one shared instantiation.
    fn sort_by_dyn(&mut self, cmp: impl FnMut(&T, &T) -> Ordering);
    /// `sort_unstable_by_key`, through one shared instantiation.
    fn sort_unstable_by_key_dyn<K: Ord>(&mut self, mut key: impl FnMut(&T) -> K) {
        self.sort_unstable_by_dyn(|a, b| key(a).cmp(&key(b)));
    }
    /// `sort_by_key` (stable), through one shared instantiation.
    fn sort_by_key_dyn<K: Ord>(&mut self, mut key: impl FnMut(&T) -> K) {
        self.sort_by_dyn(|a, b| key(a).cmp(&key(b)));
    }
}

impl<T> SortDyn<T> for [T] {
    fn sort_unstable_by_dyn(&mut self, mut cmp: impl FnMut(&T, &T) -> Ordering) {
        unstable(self, &mut cmp);
    }
    fn sort_by_dyn(&mut self, mut cmp: impl FnMut(&T, &T) -> Ordering) {
        stable(self, &mut cmp);
    }
}

fn unstable<T>(v: &mut [T], cmp: &mut dyn FnMut(&T, &T) -> Ordering) {
    let mut order = positions(v.len());
    sort_positions(
        &mut order,
        &mut |a, b| cmp(&v[a as usize], &v[b as usize]),
        false,
    );
    permute(v, order);
}

fn stable<T>(v: &mut [T], cmp: &mut dyn FnMut(&T, &T) -> Ordering) {
    let mut order = positions(v.len());
    sort_positions(
        &mut order,
        &mut |a, b| cmp(&v[a as usize], &v[b as usize]),
        true,
    );
    permute(v, order);
}

fn positions(n: usize) -> Vec<u32> {
    assert!(
        u32::try_from(n).is_ok(),
        "sorting more than u32::MAX elements"
    );
    (0..n as u32).collect()
}

/// The only two sort instantiations in the crate.
#[inline(never)]
fn sort_positions(order: &mut [u32], cmp: &mut dyn FnMut(u32, u32) -> Ordering, stable: bool) {
    if stable {
        order.sort_by(|&a, &b| cmp(a, b));
    } else {
        order.sort_unstable_by(|&a, &b| cmp(a, b));
    }
}

/// Rearrange `v` so that `v[i]` becomes the old `v[order[i]]`, swapping along each cycle.
/// `order` doubles as the visited marks.
fn permute<T>(v: &mut [T], mut order: Vec<u32>) {
    const DONE: u32 = u32::MAX;
    for start in 0..order.len() {
        if order[start] == DONE {
            continue;
        }
        let mut i = start;
        loop {
            let from = order[i] as usize;
            order[i] = DONE;
            if from == start {
                break;
            }
            v.swap(i, from);
            i = from;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SortDyn;

    #[test]
    fn the_shared_sorts_sort() {
        let mut v = vec![(3, 'a'), (1, 'b'), (3, 'c'), (2, 'd')];
        v.sort_by_key_dyn(|p| p.0);
        assert_eq!(v, [(1, 'b'), (2, 'd'), (3, 'a'), (3, 'c')]);
        v.sort_unstable_by_dyn(|a, b| b.1.cmp(&a.1));
        assert_eq!(v, [(2, 'd'), (3, 'c'), (1, 'b'), (3, 'a')]);
        let mut w = vec![5.0f64, -1.0, 2.5];
        w.sort_unstable_by_dyn(f64::total_cmp);
        assert_eq!(w, [-1.0, 2.5, 5.0]);
    }

    #[test]
    fn the_shared_sorts_agree_with_the_standard_ones() {
        // Many ties, so stability and every cycle shape of the permutation get exercised.
        let mut x = 0x2545_f491u32;
        for len in [0usize, 1, 2, 3, 7, 31, 200, 1000] {
            let v: Vec<(u32, u32)> = (0..len as u32)
                .map(|i| {
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    (x % 17, i)
                })
                .collect();
            let (mut a, mut b) = (v.clone(), v.clone());
            a.sort_by_key_dyn(|p| p.0);
            b.sort_by_key(|p| p.0);
            assert_eq!(a, b, "stable, len {len}");
            let (mut a, mut b) = (v.clone(), v);
            a.sort_unstable_by_dyn(|p, q| p.cmp(q));
            b.sort_unstable();
            assert_eq!(a, b, "unstable, len {len}");
        }
    }
}

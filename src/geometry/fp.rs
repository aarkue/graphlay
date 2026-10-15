//! Float functions that give the same bits on every target.
//!
//! A C library's `hypot`, `atan2`, `sin` and `cos` differ in the last bit between platforms, and
//! one bit can flip a placement tie. So the engine routes them through here: `hypot` is the
//! textbook formula, the rest come from the pure-Rust `libm` crate.

/// `sqrt(a * a + b * b)`. Coordinates are pixels, so there is nothing to protect from
/// overflow.
#[inline]
pub(crate) fn hypot(a: f64, b: f64) -> f64 {
    (a * a + b * b).sqrt()
}

/// The angle of `(x, y)` from the positive x axis, in radians.
#[inline]
pub(crate) fn atan2(y: f64, x: f64) -> f64 {
    libm::atan2(y, x)
}

#[inline]
pub(crate) fn sin(x: f64) -> f64 {
    libm::sin(x)
}

#[inline]
pub(crate) fn cos(x: f64) -> f64 {
    libm::cos(x)
}

/// `x * x`.
#[inline]
pub(crate) fn sq(x: f64) -> f64 {
    x * x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_formulas_agree_with_std_on_plain_values() {
        assert_eq!(hypot(3.0, 4.0), 5.0);
        assert!((atan2(1.0, 1.0) - std::f64::consts::FRAC_PI_4).abs() < 1e-15);
        assert!((sin(std::f64::consts::FRAC_PI_2) - 1.0).abs() < 1e-15);
        assert!((cos(0.0) - 1.0).abs() < 1e-15);
        assert_eq!(sq(-3.0), 9.0);
    }
}

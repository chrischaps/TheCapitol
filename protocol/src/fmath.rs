//! Float maths that gives the same bits on every platform (#180).
//!
//! `f64::sin`, `cos`, `atan2`, `asin` and `hypot` call the platform's libm: the
//! MSVC CRT on Windows, glibc on Linux. They may disagree in the last bit, and
//! world generation turns that into a road vertex more or fewer once a
//! threshold sees it. These go through the `libm` crate instead, a pure-Rust
//! port of musl's, so a world is the same world wherever it's built.
//!
//! `sqrt` is left alone: IEEE 754 requires it correctly rounded, so every
//! platform agrees already. So do `powi`, `to_degrees` and `to_radians`, which
//! are plain multiplication.
//!
//! The backend and protocol crates' `clippy.toml` disallow the `f64` methods,
//! so a new call site can't bring the platform's libm back.

/// Platform-independent replacements for `f64`'s libm-backed methods.
pub trait StableMath {
    fn stable_sin(self) -> f64;
    fn stable_cos(self) -> f64;
    fn stable_asin(self) -> f64;
    fn stable_atan2(self, other: f64) -> f64;
    fn stable_hypot(self, other: f64) -> f64;
    /// `ln(1 + self)`, exact near 0
    fn stable_ln_1p(self) -> f64;
    /// `2^self`
    fn stable_exp2(self) -> f64;
}

impl StableMath for f64 {
    #[inline]
    fn stable_sin(self) -> f64 {
        libm::sin(self)
    }

    #[inline]
    fn stable_cos(self) -> f64 {
        libm::cos(self)
    }

    #[inline]
    fn stable_asin(self) -> f64 {
        libm::asin(self)
    }

    #[inline]
    fn stable_atan2(self, other: f64) -> f64 {
        libm::atan2(self, other)
    }

    #[inline]
    fn stable_hypot(self, other: f64) -> f64 {
        libm::hypot(self, other)
    }

    #[inline]
    fn stable_ln_1p(self) -> f64 {
        libm::log1p(self)
    }

    #[inline]
    fn stable_exp2(self) -> f64 {
        libm::exp2(self)
    }
}

#[cfg(test)]
mod tests {
    use super::StableMath;

    /// Bit patterns from musl, which `libm` ports. They hold on every
    /// platform; the `f64` methods needn't.
    #[test]
    fn results_are_the_same_bits_everywhere() {
        assert_eq!(1.0f64.stable_sin().to_bits(), 0x3FEA_ED54_8F09_0CEE);
        assert_eq!(1.0f64.stable_cos().to_bits(), 0x3FE1_4A28_0FB5_068C);
        assert_eq!(0.5f64.stable_asin().to_bits(), 0x3FE0_C152_382D_7366);
        assert_eq!(1.0f64.stable_atan2(1.0).to_bits(), core::f64::consts::FRAC_PI_4.to_bits());
        assert_eq!(3.0f64.stable_hypot(4.0), 5.0);
    }
}

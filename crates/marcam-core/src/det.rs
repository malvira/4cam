//! The same results on every platform: transcendental functions (`sin`, `hypot`, ...) from
//! the `libm` crate, the musl routines Rust's own WebAssembly build uses, instead of the
//! platform's C library (MSVC's, glibc's, Apple's), whose last bits differ. A point kept or
//! dropped by a tolerance can then differ between the CLI and the web app or a plugin; with
//! these the G-code is byte-identical everywhere. `sqrt` and the arithmetic are exact (IEEE)
//! on every platform and stay as they are.
//!
//! Use the methods with a `d` in front: `x.dsin()`, `y.datan2(x)`, `a.dhypot(b)`.

pub trait Det: Sized {
    fn dsin(self) -> Self;
    fn dcos(self) -> Self;
    fn dtan(self) -> Self;
    fn dasin(self) -> Self;
    fn dacos(self) -> Self;
    fn datan(self) -> Self;
    fn datan2(self, x: Self) -> Self;
    fn dsin_cos(self) -> (Self, Self);
    fn dhypot(self, other: Self) -> Self;
    fn dpowf(self, e: Self) -> Self;
    fn dlog10(self) -> Self;
    fn dlog2(self) -> Self;
}

impl Det for f64 {
    #[inline]
    fn dsin(self) -> f64 {
        libm::sin(self)
    }
    #[inline]
    fn dcos(self) -> f64 {
        libm::cos(self)
    }
    #[inline]
    fn dtan(self) -> f64 {
        libm::tan(self)
    }
    #[inline]
    fn dasin(self) -> f64 {
        libm::asin(self)
    }
    #[inline]
    fn dacos(self) -> f64 {
        libm::acos(self)
    }
    #[inline]
    fn datan(self) -> f64 {
        libm::atan(self)
    }
    #[inline]
    fn datan2(self, x: f64) -> f64 {
        libm::atan2(self, x)
    }
    #[inline]
    fn dsin_cos(self) -> (f64, f64) {
        libm::sincos(self)
    }
    #[inline]
    fn dhypot(self, other: f64) -> f64 {
        libm::hypot(self, other)
    }
    #[inline]
    fn dpowf(self, e: f64) -> f64 {
        libm::pow(self, e)
    }
    #[inline]
    fn dlog10(self) -> f64 {
        libm::log10(self)
    }
    #[inline]
    fn dlog2(self) -> f64 {
        libm::log2(self)
    }
}

impl Det for f32 {
    #[inline]
    fn dsin(self) -> f32 {
        libm::sinf(self)
    }
    #[inline]
    fn dcos(self) -> f32 {
        libm::cosf(self)
    }
    #[inline]
    fn dtan(self) -> f32 {
        libm::tanf(self)
    }
    #[inline]
    fn dasin(self) -> f32 {
        libm::asinf(self)
    }
    #[inline]
    fn dacos(self) -> f32 {
        libm::acosf(self)
    }
    #[inline]
    fn datan(self) -> f32 {
        libm::atanf(self)
    }
    #[inline]
    fn datan2(self, x: f32) -> f32 {
        libm::atan2f(self, x)
    }
    #[inline]
    fn dsin_cos(self) -> (f32, f32) {
        libm::sincosf(self)
    }
    #[inline]
    fn dhypot(self, other: f32) -> f32 {
        libm::hypotf(self, other)
    }
    #[inline]
    fn dpowf(self, e: f32) -> f32 {
        libm::powf(self, e)
    }
    #[inline]
    fn dlog10(self) -> f32 {
        libm::log10f(self)
    }
    #[inline]
    fn dlog2(self) -> f32 {
        libm::log2f(self)
    }
}

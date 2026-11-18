//! A stand-in for the `web-time` crate (patched in by the workspace manifest).
//!
//! Natively it is `std::time`. In WebAssembly (wasm32-unknown-unknown, where `std::time::Instant`
//! panics) the clock stands still: `Instant::now()` is always the same instant, so every
//! duration measured is zero. SolidCraft's kernel only uses it to time itself for its profiler,
//! which the STEP import never reads; in exchange the module imports nothing from JavaScript.

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub use std::time::{Duration, Instant, SystemTime, SystemTimeError, UNIX_EPOCH};

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod still {
    use std::ops::{Add, AddAssign, Sub, SubAssign};
    pub use std::time::Duration;

    /// An instant of a clock that stands still (see the crate's documentation).
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct Instant(Duration);

    impl Instant {
        pub fn now() -> Instant {
            Instant(Duration::ZERO)
        }
        pub fn elapsed(&self) -> Duration {
            Instant::now().saturating_duration_since(*self)
        }
        pub fn duration_since(&self, earlier: Instant) -> Duration {
            self.saturating_duration_since(earlier)
        }
        pub fn checked_duration_since(&self, earlier: Instant) -> Option<Duration> {
            self.0.checked_sub(earlier.0)
        }
        pub fn saturating_duration_since(&self, earlier: Instant) -> Duration {
            self.0.saturating_sub(earlier.0)
        }
        pub fn checked_add(&self, d: Duration) -> Option<Instant> {
            self.0.checked_add(d).map(Instant)
        }
        pub fn checked_sub(&self, d: Duration) -> Option<Instant> {
            self.0.checked_sub(d).map(Instant)
        }
    }
    impl Add<Duration> for Instant {
        type Output = Instant;
        fn add(self, d: Duration) -> Instant {
            Instant(self.0 + d)
        }
    }
    impl AddAssign<Duration> for Instant {
        fn add_assign(&mut self, d: Duration) {
            self.0 += d;
        }
    }
    impl Sub<Duration> for Instant {
        type Output = Instant;
        fn sub(self, d: Duration) -> Instant {
            Instant(self.0.saturating_sub(d))
        }
    }
    impl SubAssign<Duration> for Instant {
        fn sub_assign(&mut self, d: Duration) {
            self.0 = self.0.saturating_sub(d);
        }
    }
    impl Sub<Instant> for Instant {
        type Output = Duration;
        fn sub(self, other: Instant) -> Duration {
            self.saturating_duration_since(other)
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use still::{Duration, Instant};

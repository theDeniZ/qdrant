//! A tiny seam over `Instant::now()` so rate/ETA math is testable without a
//! real sleep. Production code always uses `SystemClock`; tests can supply
//! a `FakeClock` that advances on command.

use std::time::Instant;

pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::sync::Mutex;
    use std::time::Duration;

    /// A clock that only moves when `advance()` is called. `Instant` has no
    /// public constructor for an arbitrary point in time, so this anchors
    /// on one real `Instant::now()` taken at construction and reports
    /// `base + offset`.
    pub struct FakeClock {
        base: Instant,
        offset: Mutex<Duration>,
    }

    impl FakeClock {
        pub fn new() -> Self {
            FakeClock {
                base: Instant::now(),
                offset: Mutex::new(Duration::ZERO),
            }
        }

        pub fn advance(&self, d: Duration) {
            let mut o = self.offset.lock().unwrap();
            *o += d;
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            self.base + *self.offset.lock().unwrap()
        }
    }
}

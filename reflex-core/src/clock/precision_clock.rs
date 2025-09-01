use crate::clock::Clock;
use std::cell::Cell;
use std::time::{Duration, Instant};
use time::OffsetDateTime;

/// A clock that caches wall time and uses fast Instant calls for precision.
///
/// Periodically resyncs wall time to handle clock drift and adjustments,
/// while providing nanosecond precision through Instant arithmetic.
pub struct PrecisionClock {
    /// Cached wall time snapshot
    base_wall_time: Cell<OffsetDateTime>,
    /// Instant when the wall time was snapped
    base_instant: Cell<Instant>,
    /// Last time we performed a resync
    last_resync: Cell<Instant>,
    /// How often to resync wall time
    resync_interval: Duration,
}

impl PrecisionClock {
    pub fn new() -> Self {
        let now_instant = Instant::now();
        Self {
            base_wall_time: Cell::new(OffsetDateTime::now_utc()),
            base_instant: Cell::new(now_instant),
            last_resync: Cell::new(now_instant),
            resync_interval: Duration::from_secs(1),
        }
    }

    pub fn with_resync_interval(mut self, interval: Duration) -> Self {
        self.resync_interval = interval;
        self
    }

    /// Force a resync of the cached wall time
    pub fn resync(&self) {
        let now_instant = Instant::now();
        let now_wall = OffsetDateTime::now_utc();

        self.base_wall_time.set(now_wall);
        self.base_instant.set(now_instant);
        self.last_resync.set(now_instant);
    }

    /// Get Unix timestamp in nanoseconds with high precision
    pub fn unix_nanos(&self) -> i128 {
        let now_instant = Instant::now();

        // Check if resync needed
        if now_instant.duration_since(self.last_resync.get()) >= self.resync_interval {
            self.resync();
        }

        let elapsed = now_instant.duration_since(self.base_instant.get());
        (self.base_wall_time.get() + elapsed).unix_timestamp_nanos()
    }
}

impl Clock for PrecisionClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn trigger_time(&self) -> OffsetDateTime {
        let now_instant = Instant::now();

        // Check if resync needed
        if now_instant.duration_since(self.last_resync.get()) >= self.resync_interval {
            self.resync();
        }

        let elapsed = now_instant.duration_since(self.base_instant.get());
        self.base_wall_time.get() + elapsed
    }
}

impl Default for PrecisionClock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_cached_clock_creation() {
        let clock = PrecisionClock::new();
        assert_eq!(clock.resync_interval, Duration::from_secs(1));
    }

    #[test]
    fn test_custom_resync_interval() {
        let clock = PrecisionClock::new().with_resync_interval(Duration::from_millis(500));

        assert_eq!(clock.resync_interval, Duration::from_millis(500));
    }

    #[test]
    fn test_trigger_time_advances() {
        let clock = PrecisionClock::new();

        let time1 = clock.trigger_time();
        thread::sleep(Duration::from_millis(1));
        let time2 = clock.trigger_time();

        assert!(time2 > time1);
    }

    #[test]
    fn test_instant_consistency() {
        let clock = PrecisionClock::new();

        let instant1 = clock.now();
        thread::sleep(Duration::from_millis(1));
        let instant2 = clock.now();

        assert!(instant2 > instant1);
    }

    #[test]
    fn test_manual_resync() {
        let clock = PrecisionClock::new();
        let initial_base = clock.base_wall_time.get();

        thread::sleep(Duration::from_millis(10));
        clock.resync();

        let after_resync = clock.base_wall_time.get();
        assert!(after_resync > initial_base);
    }

    #[test]
    fn test_unix_nanos() {
        let clock = PrecisionClock::new();
        let nanos = clock.unix_nanos();

        // Should be a reasonable Unix timestamp (> year 2020)
        assert!(nanos > 1_600_000_000_000_000_000); // Sep 2020 in nanos
    }
}

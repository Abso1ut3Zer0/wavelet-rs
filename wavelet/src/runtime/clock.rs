mod historical;
mod precision_clock;

#[cfg(any(test, feature = "testing"))]
mod test_clock;

use std::time::Instant;

pub use historical::{HistoricalClock, Interval};
pub use precision_clock::PrecisionClock;
pub use time::{Duration, OffsetDateTime};

#[cfg(any(test, feature = "testing"))]
pub use test_clock::TestClock;

/// Represents a moment in time with both monotonic and wall clock components.
///
/// This structure captures a specific point in time using:
/// - `instant`: Monotonic time for measuring durations and intervals
/// - `system_time`: Wall clock time for timestamps and human-readable times
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CycleTime {
    instant: Instant,
    unix_time: OffsetDateTime,
}

impl CycleTime {
    /// Create a new TriggerTime with the given instant and system time
    pub fn new(instant: Instant, system_time: OffsetDateTime) -> Self {
        Self {
            instant,
            unix_time: system_time,
        }
    }

    pub const fn now(&self) -> Instant {
        self.instant
    }

    pub const fn unix_timestamp(&self) -> OffsetDateTime {
        self.unix_time
    }

    pub const fn unix_timestamp_nanos(&self) -> i128 {
        self.unix_time.unix_timestamp_nanos()
    }
}

/// Provides timing information for execution contexts.
///
/// Implementations should provide consistent, high-quality time measurements
/// suitable for both performance timing (via `instant`) and timestamping
/// (via `system_time`).
pub trait Clock {
    /// Get the current cycle time containing both monotonic and wall clock times.
    ///
    /// The returned times should represent the same moment, captured as atomically
    /// as possible to minimize skew between the monotonic and wall clock components.
    fn cycle_time(&mut self) -> CycleTime;
}

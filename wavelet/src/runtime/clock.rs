mod historical;
mod precision_clock;

#[cfg(any(test, feature = "testing"))]
mod test_clock;

use std::time::Instant;

pub use historical::HistoricalClock;
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
pub struct TriggerTime {
    pub instant: Instant,
    pub system_time: OffsetDateTime,
}

impl TriggerTime {
    /// Create a new TriggerTime with the given instant and system time
    pub fn new(instant: Instant, system_time: OffsetDateTime) -> Self {
        Self {
            instant,
            system_time,
        }
    }
}

/// Provides timing information for execution contexts.
///
/// Implementations should provide consistent, high-quality time measurements
/// suitable for both performance timing (via `instant`) and timestamping
/// (via `system_time`).
pub trait Clock {
    /// Get the current trigger time containing both monotonic and wall clock times.
    ///
    /// The returned times should represent the same moment, captured as atomically
    /// as possible to minimize skew between the monotonic and wall clock components.
    fn trigger_time(&mut self) -> TriggerTime;
}

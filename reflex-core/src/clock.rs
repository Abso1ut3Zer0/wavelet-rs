mod precision_clock;
mod test_clock;

use std::time::Instant;

pub use precision_clock::PrecisionClock;
pub use test_clock::TestClock;
pub use time::{Duration, OffsetDateTime};

pub trait Clock {
    fn now(&self) -> Instant;

    fn trigger_time(&self) -> OffsetDateTime;
}

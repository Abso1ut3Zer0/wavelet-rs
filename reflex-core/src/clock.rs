use std::time::Instant;
use time::OffsetDateTime;

pub trait Clock {
    fn now(&self) -> Instant;

    fn trigger_time(&self) -> OffsetDateTime;
}

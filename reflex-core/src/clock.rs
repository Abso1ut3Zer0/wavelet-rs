use std::time::Instant;
use time::OffsetDateTime;

pub trait Clock {
    fn now(&self) -> Instant;

    fn wall_time(&self) -> OffsetDateTime;
}

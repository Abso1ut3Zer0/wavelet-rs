use crate::clock::{Clock, TriggerTime};
use std::time::{Duration, Instant};
use time::OffsetDateTime;

/// Represents a time interval with an inclusive start and exclusive end.
///
/// The interval is [start, end) - includes start time but excludes end time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interval {
    pub start: OffsetDateTime,
    pub end: OffsetDateTime,
}

impl Interval {
    /// Create a new interval with the given start and end times
    pub fn new(start: OffsetDateTime, end: OffsetDateTime) -> Self {
        assert!(start < end, "Interval start must be before end");
        Self { start, end }
    }

    /// Get the total duration of this interval
    pub fn duration(&self) -> Duration {
        (self.end - self.start).try_into().unwrap_or(Duration::ZERO)
    }

    /// Check if a given time falls within this interval [start, end)
    pub fn contains(&self, time: OffsetDateTime) -> bool {
        time >= self.start && time < self.end
    }
}

/// A clock that replays historical time by walking through a specified interval.
///
/// The clock starts at the interval's start time and advances in fixed time steps
/// until reaching the end of the interval. Each call to `trigger_time()` advances
/// the clock by one time step.
///
/// The instant component is adjusted so that the baseline instant represents the
/// start of the historical interval, maintaining proper duration relationships.
pub struct HistoricalClock {
    /// The time interval being replayed
    interval: Interval,
    /// How much to advance time on each step
    time_step: Duration,
    /// Current position within the interval
    current_time: OffsetDateTime,
    /// The instant that represents the start of our historical interval
    baseline_instant: Instant,
    /// Whether we've exhausted the interval
    exhausted: bool,
}

impl HistoricalClock {
    /// Create a new historical clock with a default 100μs time step
    pub fn new(interval: Interval) -> Self {
        Self::with_time_step(interval, Duration::from_micros(100))
    }

    /// Create a new historical clock with a custom time step
    pub fn with_time_step(interval: Interval, time_step: Duration) -> Self {
        let baseline_instant = Instant::now(); // This represents interval.start

        Self {
            current_time: interval.start,
            interval,
            time_step,
            baseline_instant,
            exhausted: false,
        }
    }

    /// Check if the clock has exhausted the historical interval
    pub fn is_exhausted(&self) -> bool {
        self.exhausted
    }

    /// Get the current position within the interval
    pub fn current_time(&self) -> OffsetDateTime {
        self.current_time
    }

    /// Get the interval being replayed
    pub fn interval(&self) -> &Interval {
        &self.interval
    }

    /// Get the time step duration
    pub fn time_step(&self) -> Duration {
        self.time_step
    }

    /// Reset the clock back to the start of the interval
    pub fn reset(&mut self) {
        self.current_time = self.interval.start;
        self.exhausted = false;
    }

    /// Calculate the current instant based on our position in the historical interval
    fn current_instant(&self) -> Instant {
        let elapsed_in_interval = (self.current_time - self.interval.start)
            .try_into()
            .unwrap_or(Duration::ZERO);

        self.baseline_instant + elapsed_in_interval
    }

    /// Advance to the next time step
    fn advance(&mut self) {
        if self.exhausted {
            return;
        }

        let next_time = self.current_time + self.time_step;

        // Check if we would go past the end of the interval
        if next_time >= self.interval.end {
            self.exhausted = true;
        } else {
            self.current_time = next_time;
        }
    }
}

impl Clock for HistoricalClock {
    fn trigger_time(&mut self) -> TriggerTime {
        let result = TriggerTime::new(self.current_instant(), self.current_time);

        // Advance for next call (unless already exhausted)
        self.advance();

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interval_creation() {
        let start = OffsetDateTime::UNIX_EPOCH;
        let end = OffsetDateTime::UNIX_EPOCH + Duration::from_secs(3600);
        let interval = Interval::new(start, end);

        assert_eq!(interval.start, start);
        assert_eq!(interval.end, end);
        assert_eq!(interval.duration(), Duration::from_secs(3600));
    }

    #[test]
    #[should_panic(expected = "Interval start must be before end")]
    fn test_interval_invalid_order() {
        let start = OffsetDateTime::UNIX_EPOCH + Duration::from_secs(100);
        let end = OffsetDateTime::UNIX_EPOCH;
        Interval::new(start, end);
    }

    #[test]
    fn test_interval_contains() {
        let start = OffsetDateTime::UNIX_EPOCH;
        let end = OffsetDateTime::UNIX_EPOCH + Duration::from_secs(100);
        let interval = Interval::new(start, end);

        // Start is included
        assert!(interval.contains(start));

        // Middle is included
        assert!(interval.contains(start + Duration::from_secs(50)));

        // End is excluded
        assert!(!interval.contains(end));

        // Past end is excluded
        assert!(!interval.contains(end + Duration::from_secs(1)));
    }

    #[test]
    fn test_historical_clock_creation() {
        let start = OffsetDateTime::UNIX_EPOCH;
        let end = OffsetDateTime::UNIX_EPOCH + Duration::from_secs(100);
        let interval = Interval::new(start, end);

        let clock = HistoricalClock::new(interval.clone());

        assert_eq!(clock.interval(), &interval);
        assert_eq!(clock.time_step(), Duration::from_micros(100));
        assert_eq!(clock.current_time(), start);
        assert!(!clock.is_exhausted());
    }

    #[test]
    fn test_historical_clock_advance() {
        let start = OffsetDateTime::UNIX_EPOCH;
        let end = OffsetDateTime::UNIX_EPOCH + Duration::from_millis(1); // Very short interval
        let interval = Interval::new(start, end);

        let mut clock = HistoricalClock::with_time_step(interval, Duration::from_micros(100));

        // First call should return start time
        let time1 = clock.trigger_time();
        assert_eq!(time1.system_time, start);
        assert!(!clock.is_exhausted());

        // Subsequent calls should advance
        let time2 = clock.trigger_time();
        assert_eq!(time2.system_time, start + Duration::from_micros(100));

        // Continue until exhausted
        let mut call_count = 2;
        while !clock.is_exhausted() && call_count < 100 {
            clock.trigger_time();
            call_count += 1;
        }

        assert!(clock.is_exhausted());
        assert!(call_count < 100); // Should exhaust before 100 calls
    }

    #[test]
    fn test_historical_clock_instant_consistency() {
        let start = OffsetDateTime::UNIX_EPOCH;
        let end = OffsetDateTime::UNIX_EPOCH + Duration::from_secs(1);
        let interval = Interval::new(start, end);
        let step = Duration::from_millis(100);

        let mut clock = HistoricalClock::with_time_step(interval, step);

        let time1 = clock.trigger_time();
        let time2 = clock.trigger_time();

        // Instant should advance by exactly the time step
        assert_eq!(time2.instant.duration_since(time1.instant), step);

        // System time should also advance by the time step
        assert_eq!(time2.system_time - time1.system_time, step);
    }

    #[test]
    fn test_historical_clock_reset() {
        let start = OffsetDateTime::UNIX_EPOCH;
        let end = OffsetDateTime::UNIX_EPOCH + Duration::from_secs(1);
        let interval = Interval::new(start, end);

        let mut clock = HistoricalClock::new(interval);

        // Advance several steps
        clock.trigger_time();
        clock.trigger_time();
        clock.trigger_time();

        assert_ne!(clock.current_time(), start);

        // Reset should go back to start
        clock.reset();
        assert_eq!(clock.current_time(), start);
        assert!(!clock.is_exhausted());
    }

    #[test]
    fn test_historical_clock_exhaustion() {
        let start = OffsetDateTime::UNIX_EPOCH;
        let end = OffsetDateTime::UNIX_EPOCH + Duration::from_micros(250); // 2.5 steps
        let interval = Interval::new(start, end);
        let step = Duration::from_micros(100);

        let mut clock = HistoricalClock::with_time_step(interval, step);

        // Should get exactly 2 calls before exhaustion
        let time1 = clock.trigger_time(); // 0μs
        assert!(!clock.is_exhausted());
        assert_eq!(time1.system_time, start);

        let time2 = clock.trigger_time(); // 100μs
        assert!(!clock.is_exhausted());
        assert_eq!(time2.system_time, start + Duration::from_micros(100));

        let time3 = clock.trigger_time(); // 200μs
        assert!(clock.is_exhausted()); // Next step would be 300μs which >= 250μs end
        assert_eq!(time3.system_time, start + Duration::from_micros(200));
    }

    #[test]
    fn test_exhausted_clock_no_further_advance() {
        let start = OffsetDateTime::UNIX_EPOCH;
        let end = OffsetDateTime::UNIX_EPOCH + Duration::from_micros(100);
        let interval = Interval::new(start, end);
        let step = Duration::from_micros(200); // Step larger than interval

        let mut clock = HistoricalClock::with_time_step(interval, step);

        let time1 = clock.trigger_time();
        assert!(clock.is_exhausted());

        // Further calls shouldn't change anything
        let time2 = clock.trigger_time();
        assert_eq!(time1.system_time, time2.system_time);
        assert_eq!(time1.instant, time2.instant);
    }
}

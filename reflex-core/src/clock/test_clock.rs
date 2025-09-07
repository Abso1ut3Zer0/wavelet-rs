use crate::clock::{Clock, TriggerTime};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};
use time::OffsetDateTime;

/// A clock implementation for testing that starts at a baseline and advances from there.
///
/// By default, the clock starts at Unix epoch (1970-01-01 00:00:00 UTC) for both
/// wall time and a synthetic "instant zero". This makes test assertions predictable
/// and readable. Time advances are relative to these baselines.
pub struct TestClock(Rc<RefCell<TestClockInner>>);

impl TestClock {
    /// Create a new test clock starting at Unix epoch (time zero)
    pub fn new() -> Self {
        TestClock(Rc::new(RefCell::new(TestClockInner::new())))
    }

    /// Create a test clock starting at a specific wall time
    pub fn starting_at(wall_time: OffsetDateTime) -> Self {
        TestClock(Rc::new(RefCell::new(TestClockInner::starting_at(
            wall_time,
        ))))
    }

    /// Advance both monotonic and wall time by the specified duration
    pub fn advance(&self, duration: Duration) {
        self.0.borrow_mut().advance(duration);
    }

    /// Set the current time to a specific offset from the baseline
    pub fn set_elapsed(&self, elapsed: Duration) {
        self.0.borrow_mut().set_elapsed(elapsed);
    }

    /// Get the current elapsed time from baseline
    pub fn elapsed(&self) -> Duration {
        self.0.borrow().elapsed
    }

    /// Get current times without mutation (useful for assertions)
    pub fn current_times(&self) -> (Instant, OffsetDateTime) {
        let inner = self.0.borrow();
        (inner.current_instant(), inner.current_wall_time())
    }

    /// Reset back to baseline (time zero)
    pub fn reset(&self) {
        self.0.borrow_mut().reset();
    }
}

impl Clock for TestClock {
    fn trigger_time(&mut self) -> TriggerTime {
        let inner = self.0.borrow();
        TriggerTime {
            instant: inner.current_instant(),
            system_time: inner.current_wall_time(),
        }
    }
}

impl Default for TestClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for TestClock {
    fn clone(&self) -> Self {
        TestClock(self.0.clone())
    }
}

struct TestClockInner {
    /// The baseline wall time (default: Unix epoch)
    baseline_wall_time: OffsetDateTime,
    /// Synthetic baseline instant (creation_instant represents this time)
    baseline_instant: Instant,
    /// How much time has elapsed from the baseline
    elapsed: Duration,
}

impl TestClockInner {
    fn new() -> Self {
        // Start at Unix epoch for predictable testing
        let baseline_wall_time = OffsetDateTime::UNIX_EPOCH;
        // Create a synthetic "instant zero"
        let baseline_instant = Instant::now();

        Self {
            baseline_wall_time,
            baseline_instant,
            elapsed: Duration::ZERO,
        }
    }

    fn starting_at(baseline_wall_time: OffsetDateTime) -> Self {
        let baseline_instant = Instant::now();

        Self {
            baseline_wall_time,
            baseline_instant,
            elapsed: Duration::ZERO,
        }
    }

    fn advance(&mut self, duration: Duration) {
        self.elapsed += duration;
    }

    fn set_elapsed(&mut self, elapsed: Duration) {
        self.elapsed = elapsed;
    }

    fn reset(&mut self) {
        self.elapsed = Duration::ZERO;
    }

    fn current_instant(&self) -> Instant {
        // Return synthetic instant = baseline + elapsed
        self.baseline_instant + self.elapsed
    }

    fn current_wall_time(&self) -> OffsetDateTime {
        // Return wall time = baseline + elapsed
        self.baseline_wall_time + self.elapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clock_starts_at_epoch() {
        let mut clock = TestClock::new();
        let time = clock.trigger_time();

        // Should start at Unix epoch
        assert_eq!(time.system_time, OffsetDateTime::UNIX_EPOCH);
        assert_eq!(clock.elapsed(), Duration::ZERO);
    }

    #[test]
    fn test_clock_advance() {
        let mut clock = TestClock::new();

        // Advance by 1 hour
        clock.advance(Duration::from_secs(3600));
        let time = clock.trigger_time();

        // Should be 1 hour after epoch
        assert_eq!(
            time.system_time,
            OffsetDateTime::UNIX_EPOCH + Duration::from_secs(3600)
        );
        assert_eq!(clock.elapsed(), Duration::from_secs(3600));
    }

    #[test]
    fn test_predictable_instants() {
        let mut clock = TestClock::new();
        let time1 = clock.trigger_time();

        clock.advance(Duration::from_millis(500));
        let time2 = clock.trigger_time();

        // Instant difference should be exactly 500ms
        assert_eq!(
            time2.instant.duration_since(time1.instant),
            Duration::from_millis(500)
        );
    }

    #[test]
    fn test_starting_at_custom_time() {
        let start_time = OffsetDateTime::from_unix_timestamp(1000000000).unwrap(); // 2001-09-09
        let mut clock = TestClock::starting_at(start_time);

        let time = clock.trigger_time();
        assert_eq!(time.system_time, start_time);
        assert_eq!(clock.elapsed(), Duration::ZERO);

        // Advance and check
        clock.advance(Duration::from_secs(60));
        let time2 = clock.trigger_time();
        assert_eq!(time2.system_time, start_time + Duration::from_secs(60));
    }

    #[test]
    fn test_set_elapsed() {
        let mut clock = TestClock::new();

        // Jump to 5 minutes elapsed
        clock.set_elapsed(Duration::from_secs(300));
        let time = clock.trigger_time();

        assert_eq!(
            time.system_time,
            OffsetDateTime::UNIX_EPOCH + Duration::from_secs(300)
        );
        assert_eq!(clock.elapsed(), Duration::from_secs(300));
    }

    #[test]
    fn test_reset() {
        let mut clock = TestClock::new();

        // Advance time
        clock.advance(Duration::from_secs(1000));
        assert_eq!(clock.elapsed(), Duration::from_secs(1000));

        // Reset back to baseline
        clock.reset();
        let time = clock.trigger_time();

        assert_eq!(time.system_time, OffsetDateTime::UNIX_EPOCH);
        assert_eq!(clock.elapsed(), Duration::ZERO);
    }

    #[test]
    fn test_readable_test_assertions() {
        let mut clock = TestClock::new();

        // This is much more readable in tests!
        clock.set_elapsed(Duration::from_secs(3600)); // 1 hour
        let time = clock.trigger_time();

        // Clear, predictable assertions
        assert_eq!(time.system_time.unix_timestamp(), 3600); // 1970-01-01 01:00:00 UTC

        clock.advance(Duration::from_secs(1800)); // +30 minutes
        let time2 = clock.trigger_time();
        assert_eq!(time2.system_time.unix_timestamp(), 5400); // 1970-01-01 01:30:00 UTC
    }

    #[test]
    fn test_clone_behavior() {
        let clock1 = TestClock::new();
        let clock2 = clock1.clone();

        // Both should share the same inner state
        clock1.advance(Duration::from_secs(100));

        assert_eq!(clock1.elapsed(), Duration::from_secs(100));
        assert_eq!(clock2.elapsed(), Duration::from_secs(100));
    }

    #[test]
    fn test_instant_consistency() {
        let mut clock = TestClock::new();

        // Multiple calls should return the same instant at the same elapsed time
        let time1 = clock.trigger_time();
        let time2 = clock.trigger_time();

        assert_eq!(time1.instant, time2.instant);
        assert_eq!(time1.system_time, time2.system_time);
    }
}

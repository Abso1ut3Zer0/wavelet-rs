use crate::runtime::clock::{Clock, CycleTime};
use std::time::{Duration, Instant};
use time::OffsetDateTime;

// OS-specific timing defaults based on platform characteristics
#[cfg(target_os = "linux")]
const DEFAULT_MEASUREMENT_THRESHOLD_NS: u64 = 500; // Linux has fast, consistent timing

#[cfg(target_os = "windows")]
const DEFAULT_MEASUREMENT_THRESHOLD_NS: u64 = 2000; // Windows can be more variable

#[cfg(target_os = "macos")]
const DEFAULT_MEASUREMENT_THRESHOLD_NS: u64 = 1000; // macOS is generally good but can vary

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
const DEFAULT_MEASUREMENT_THRESHOLD_NS: u64 = 1500; // Conservative default for other platforms

#[cfg(target_os = "linux")]
const DEFAULT_MAX_RETRIES: u32 = 5; // Linux timing is very predictable

#[cfg(target_os = "windows")]
const DEFAULT_MAX_RETRIES: u32 = 20; // Windows needs more attempts for good measurements

#[cfg(target_os = "macos")]
const DEFAULT_MAX_RETRIES: u32 = 10; // macOS is in between

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
const DEFAULT_MAX_RETRIES: u32 = 15; // Conservative default for other platforms

const DEFAULT_RESYNC_INTERVAL_SECS: u64 = 3600; // 1 hour like Agrona

/// A high-precision clock that combines wall time accuracy with monotonic performance.
///
/// This implementation is inspired by Agrona's OffsetEpochNanoClock and uses:
/// - Multiple sampling attempts to minimize syscall timing variance
/// - Automatic resyncing to handle clock drift and NTP adjustments
/// - Quality metrics to ensure accurate baseline measurements
/// - OS-specific defaults optimized for platform timing characteristics
///
/// The clock establishes a baseline by carefully measuring both wall time and
/// monotonic time, then uses the monotonic clock for high-precision timing
/// while periodically resyncing to maintain wall time accuracy.
pub struct PrecisionClock {
    /// Cached wall time at baseline measurement
    base_wall_time: OffsetDateTime,
    /// Monotonic time at baseline measurement
    base_instant: Instant,
    /// When we last performed a resync
    last_resync: Instant,
    /// How often to automatically resync
    resync_interval: Duration,
    /// Maximum timing variance acceptable for a measurement
    measurement_threshold: Duration,
    /// Maximum attempts to get an accurate measurement
    max_measurement_retries: u32,
    /// Whether the last sampling achieved the desired accuracy
    is_accurate: bool,
}

impl PrecisionClock {
    /// Create a new precision clock with OS-optimized defaults
    pub fn new() -> Self {
        let mut clock = Self {
            base_wall_time: OffsetDateTime::now_utc(),
            base_instant: Instant::now(),
            last_resync: Instant::now(),
            resync_interval: Duration::from_secs(DEFAULT_RESYNC_INTERVAL_SECS),
            measurement_threshold: Duration::from_nanos(DEFAULT_MEASUREMENT_THRESHOLD_NS),
            max_measurement_retries: DEFAULT_MAX_RETRIES,
            is_accurate: false,
        };

        // Perform initial accurate sampling
        clock.resync_with_quality_control();
        clock
    }

    /// Create a precision clock with custom configuration
    pub fn with_config(
        resync_interval: Duration,
        measurement_threshold: Duration,
        max_retries: u32,
    ) -> Self {
        let mut clock = Self {
            base_wall_time: OffsetDateTime::now_utc(),
            base_instant: Instant::now(),
            last_resync: Instant::now(),
            resync_interval,
            measurement_threshold,
            max_measurement_retries: max_retries,
            is_accurate: false,
        };

        clock.resync_with_quality_control();
        clock
    }

    /// Force a manual resync using quality control sampling
    #[inline]
    pub fn resync(&mut self) {
        self.resync_with_quality_control();
    }

    /// Whether the clock is operating with good measurement accuracy
    pub const fn is_accurate(&self) -> bool {
        self.is_accurate
    }

    /// Get the measurement threshold being used
    pub const fn measurement_threshold(&self) -> Duration {
        self.measurement_threshold
    }

    /// Perform quality-controlled sampling to establish accurate baseline
    fn resync_with_quality_control(&mut self) {
        let mut best_measurement: Option<(Instant, OffsetDateTime, Duration)> = None;
        let mut best_variance = Duration::from_secs(1); // Start with 1 second as "worst"

        for attempt in 0..self.max_measurement_retries {
            // Bracket the wall time syscall with two instant measurements
            let instant_before = Instant::now();
            let wall_time = OffsetDateTime::now_utc();
            let instant_after = Instant::now();

            let measurement_variance = instant_after - instant_before;

            // If this measurement has less timing variance, it's more accurate
            if measurement_variance < best_variance {
                best_variance = measurement_variance;

                // Calculate midpoint - our best estimate of when wall_time was captured
                let estimated_instant = instant_before + measurement_variance / 2;
                best_measurement = Some((estimated_instant, wall_time, measurement_variance));

                // If we hit our accuracy threshold, we're done
                if measurement_variance <= self.measurement_threshold {
                    break;
                }
            }

            // Small delay between attempts to avoid tight loop artifacts
            if attempt < self.max_measurement_retries - 1 {
                std::hint::spin_loop();
            }
        }

        // Apply the best measurement we found
        if let Some((estimated_instant, wall_time, variance)) = best_measurement {
            self.base_wall_time = wall_time;
            self.base_instant = estimated_instant;
            self.last_resync = estimated_instant;
            self.is_accurate = variance <= self.measurement_threshold;

            #[cfg(debug_assertions)]
            if !self.is_accurate {
                eprintln!(
                    "PrecisionClock: Warning - could not achieve desired accuracy. \
                     Best measurement: {:?}, threshold: {:?}",
                    variance, self.measurement_threshold
                );
            }
        } else {
            // Fallback - this should never happen but better safe than sorry
            let now_instant = Instant::now();
            self.base_wall_time = OffsetDateTime::now_utc();
            self.base_instant = now_instant;
            self.last_resync = now_instant;
            self.is_accurate = false;
        }
    }

    /// Check if an automatic resync is needed and perform it
    #[inline]
    fn check_and_resync(&mut self, now: Instant) {
        if now.duration_since(self.last_resync) >= self.resync_interval {
            self.resync_with_quality_control();
        }
    }
}

impl Clock for PrecisionClock {
    fn cycle_time(&mut self) -> CycleTime {
        let now_instant = Instant::now();

        // Check if we need to automatically resync
        self.check_and_resync(now_instant);

        // Calculate wall time using our baseline offset
        let elapsed_since_baseline = now_instant.duration_since(self.base_instant);
        let estimated_cycle_time = self.base_wall_time + elapsed_since_baseline;

        CycleTime {
            instant: now_instant,
            unix_time: estimated_cycle_time,
        }
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
    fn test_precision_clock_creation() {
        let clock = PrecisionClock::new();

        // Should have OS-specific defaults
        assert_eq!(
            clock.resync_interval,
            Duration::from_secs(DEFAULT_RESYNC_INTERVAL_SECS)
        );
        assert_eq!(
            clock.measurement_threshold,
            Duration::from_nanos(DEFAULT_MEASUREMENT_THRESHOLD_NS)
        );
        assert_eq!(clock.max_measurement_retries, DEFAULT_MAX_RETRIES);
    }

    #[test]
    fn test_custom_configuration() {
        let clock = PrecisionClock::with_config(
            Duration::from_secs(1800), // 30 minutes
            Duration::from_nanos(500), // 500ns threshold
            50,                        // 50 retries
        );

        assert_eq!(clock.resync_interval, Duration::from_secs(1800));
        assert_eq!(clock.measurement_threshold, Duration::from_nanos(500));
        assert_eq!(clock.max_measurement_retries, 50);
    }

    #[test]
    fn test_time_advances() {
        let mut clock = PrecisionClock::new();

        let time1 = clock.cycle_time();
        thread::sleep(Duration::from_millis(1));
        let time2 = clock.cycle_time();

        // Time should advance
        assert!(time2.instant > time1.instant);
        assert!(time2.unix_time > time1.unix_time);
    }

    #[test]
    fn test_manual_resync() {
        let mut clock = PrecisionClock::new();
        let initial_accuracy = clock.is_accurate();

        // Force a resync
        clock.resync();

        // Should still work
        let time = clock.cycle_time();
        assert!(time.unix_time.unix_timestamp() > 0);

        // Accuracy might change but should still be reasonable
        assert!(clock.is_accurate() == initial_accuracy || !clock.is_accurate());
    }

    #[test]
    fn test_time_consistency() {
        let mut clock = PrecisionClock::new();

        // Multiple rapid calls should maintain reasonable consistency
        let times: Vec<_> = (0..10).map(|_| clock.cycle_time()).collect();

        // All times should be increasing
        for window in times.windows(2) {
            assert!(window[1].instant >= window[0].instant);
            assert!(window[1].unix_time >= window[0].unix_time);
        }
    }

    #[test]
    fn test_measurement_quality_reporting() {
        let clock = PrecisionClock::new();

        // Should report whether measurement was accurate
        let _is_accurate = clock.is_accurate();
        let _threshold = clock.measurement_threshold();

        // These calls should not panic
    }

    #[test]
    fn test_os_specific_defaults() {
        let clock = PrecisionClock::new();

        // Verify the OS-specific defaults are applied
        #[cfg(target_os = "linux")]
        {
            assert_eq!(clock.measurement_threshold, Duration::from_nanos(500));
            assert_eq!(clock.max_measurement_retries, 5);
        }

        #[cfg(target_os = "windows")]
        {
            assert_eq!(clock.measurement_threshold, Duration::from_nanos(2000));
            assert_eq!(clock.max_measurement_retries, 20);
        }

        #[cfg(target_os = "macos")]
        {
            assert_eq!(clock.measurement_threshold, Duration::from_nanos(1000));
            assert_eq!(clock.max_measurement_retries, 10);
        }
    }
}

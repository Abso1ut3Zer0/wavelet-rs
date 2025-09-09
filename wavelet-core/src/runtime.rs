use crate::clock::{Clock, HistoricalClock, PrecisionClock, TestClock};
use crate::executor::{Executor, ExecutorState};
use std::time::Duration;

const MINIMUM_TIMER_PRECISION: Duration = Duration::from_millis(1);

/// Marker trait for runtime execution strategies.
///
/// Defines how the runtime should behave when no nodes are ready for execution.
/// Different execution modes provide different trade-offs between CPU usage,
/// latency, and power consumption.
pub trait ExecutionMode {}

/// Busy-wait execution mode that continuously polls without yielding CPU.
///
/// Provides the lowest possible latency at the cost of high CPU usage.
/// Best for latency-critical applications where CPU resources are dedicated.
pub struct Spin;

/// Sleep-based execution mode that yields CPU for a maximum duration.
///
/// Balances latency and CPU usage by sleeping for the shorter of:
/// - The configured maximum sleep duration
/// - Time until the next timer expires
///
/// Good for most applications that need reasonable latency without burning CPU.
pub struct Sleep(Duration);

/// Blocking execution mode that waits indefinitely for events.
///
/// Provides the most CPU-efficient operation by blocking until events occur
/// or timers expire. Higher latency but minimal CPU usage when idle.
/// Best for background processing or low-frequency event handling.
pub struct Block;
impl ExecutionMode for Spin {}
impl ExecutionMode for Sleep {}
impl ExecutionMode for Block {}

impl Sleep {
    /// Creates a new Sleep execution mode with the specified maximum duration.
    ///
    /// The duration must be at least 1ms to prevent excessive polling.
    pub fn new(duration: Duration) -> Self {
        assert!(duration >= Duration::from_millis(1));
        Self(duration)
    }
}

/// Helper trait that implements single-cycle execution for different runtime configurations.
///
/// `CycleOnce` abstracts over the different timeout calculation strategies needed
/// for various clock and execution mode combinations. Each implementation handles:
/// - **Time snapshot**: Getting current time from the clock
/// - **Timeout calculation**: Determining how long to wait for I/O events
/// - **Error handling**: Converting I/O errors to executor state
///
/// This trait enables generic execution patterns like `run_forever()` while
/// allowing each runtime configuration to optimize its polling behavior.
///
/// # Implementation Strategies
///
/// - **`Spin`**: Always uses `Duration::ZERO` timeout for immediate polling
/// - **`Sleep(max_duration)`**: Uses the minimum of max duration and next timer
/// - **`Block`**: Waits indefinitely or until next timer (whichever comes first)
/// - **`TestClock`**: Always uses immediate polling regardless of execution mode
pub trait CycleOnce {
    /// Executes one complete cycle and returns the executor state.
    fn cycle_once(&mut self) -> ExecutorState;
}

/// Errors that can occur during runtime construction.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeBuilderError {
    #[error("no clock provided")]
    NoClock,
    #[error("no execution mode provided")]
    NoExecutionMode,
}

/// Builder for constructing a runtime with specific clock and execution mode.
///
/// Uses the type system to ensure both clock and execution mode are provided
/// before building the runtime.
pub struct RuntimeBuilder<C: Clock, M: ExecutionMode> {
    clock: Option<C>,
    mode: Option<M>,
}

impl<C: Clock, M: ExecutionMode> RuntimeBuilder<C, M> {
    pub const fn new() -> Self {
        Self {
            clock: None,
            mode: None,
        }
    }

    pub fn with_clock(mut self, clock: C) -> Self {
        self.clock = Some(clock);
        self
    }

    pub fn with_mode(mut self, mode: M) -> Self {
        self.mode = Some(mode);
        self
    }

    pub fn build(self) -> Result<Runtime<C, M>, RuntimeBuilderError> {
        let clock = self.clock.ok_or(RuntimeBuilderError::NoClock)?;
        let mode = self.mode.ok_or(RuntimeBuilderError::NoExecutionMode)?;
        Ok(Runtime {
            executor: Executor::new(),
            clock,
            mode,
        })
    }
}

/// A complete runtime instance that combines executor, clock, and execution mode.
///
/// The `Runtime` orchestrates the execution loop by:
/// - Using the clock to provide consistent time snapshots
/// - Running the executor with the configured execution mode
/// - Handling different clock types (Precision, Historical, Test) appropriately
///
/// The type parameters ensure compile-time guarantees about clock and mode
/// compatibility.
pub struct Runtime<C: Clock, M: ExecutionMode> {
    /// The core computation engine
    executor: Executor,

    /// Time source for execution cycles
    clock: C,

    /// Execution strategy for the main loop
    mode: M,
}

impl<C: Clock, M: ExecutionMode> Runtime<C, M> {
    pub const fn builder() -> RuntimeBuilder<C, M> {
        RuntimeBuilder::new()
    }

    pub fn executor(&mut self) -> &mut Executor {
        &mut self.executor
    }
}

impl<M: ExecutionMode> Runtime<PrecisionClock, M>
where
    Self: CycleOnce,
{
    pub fn run_forever(mut self) {
        while self.cycle_once().is_running() {
            // continue
        }
    }
}

impl<M: ExecutionMode> Runtime<HistoricalClock, M> {
    pub fn run_until_completion(mut self) {
        while !self.clock.is_exhausted() {
            let state = self
                .executor
                .cycle(self.clock.trigger_time(), None)
                .unwrap_or(ExecutorState::Running);

            if state.is_terminated() {
                return;
            }
        }
    }
}

impl<M: ExecutionMode> Runtime<TestClock, M> {
    pub fn run_one_cycle(&mut self) -> ExecutorState {
        self.cycle_once()
    }
}

impl CycleOnce for Runtime<PrecisionClock, Spin> {
    #[inline(always)]
    fn cycle_once(&mut self) -> ExecutorState {
        self.executor
            .cycle(self.clock.trigger_time(), Some(Duration::ZERO))
            .unwrap_or(ExecutorState::Running)
    }
}

impl CycleOnce for Runtime<PrecisionClock, Sleep> {
    #[inline(always)]
    fn cycle_once(&mut self) -> ExecutorState {
        let now = self.clock.trigger_time();
        let duration = self
            .executor
            .next_timer()
            .map(|when| (when.saturating_duration_since(now.instant)).min(self.mode.0));
        self.executor
            .cycle(now, duration)
            .unwrap_or(ExecutorState::Running)
    }
}

impl CycleOnce for Runtime<PrecisionClock, Block> {
    #[inline(always)]
    fn cycle_once(&mut self) -> ExecutorState {
        let now = self.clock.trigger_time();
        let duration = self.executor.next_timer().map(|when| {
            when.saturating_duration_since(now.instant)
                .max(MINIMUM_TIMER_PRECISION)
        });
        self.executor
            .cycle(now, duration)
            .unwrap_or(ExecutorState::Running)
    }
}

impl<M: ExecutionMode> CycleOnce for Runtime<TestClock, M> {
    #[inline(always)]
    fn cycle_once(&mut self) -> ExecutorState {
        self.executor
            .cycle(self.clock.trigger_time(), Some(Duration::ZERO))
            .unwrap_or(ExecutorState::Running)
    }
}

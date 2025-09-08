use crate::clock::{Clock, HistoricalClock, PrecisionClock, TestClock};
use crate::executor::Executor;
use std::time::Duration;

const MINIMUM_TIMER_PRECISION: Duration = Duration::from_millis(1);

pub trait ExecutionMode {}
pub struct Spin;
pub struct Sleep(Duration);
pub struct Block;
impl ExecutionMode for Spin {}
impl ExecutionMode for Sleep {}
impl ExecutionMode for Block {}

impl Sleep {
    pub fn new(duration: Duration) -> Self {
        assert!(duration >= Duration::from_millis(1));
        Self(duration)
    }
}

pub trait CycleOnce {
    fn cycle_once(&mut self);
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeBuilderError {
    #[error("no clock provided")]
    NoClock,
    #[error("no execution mode provided")]
    NoExecutionMode,
}

pub struct RuntimeBuilder<C: Clock, M: ExecutionMode> {
    clock: Option<C>,
    mode: Option<M>,
}

impl<C: Clock, M: ExecutionMode> RuntimeBuilder<C, M> {
    pub fn new() -> Self {
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

pub struct Runtime<C: Clock, M: ExecutionMode> {
    executor: Executor,
    clock: C,
    mode: M,
}

impl<M: ExecutionMode> Runtime<PrecisionClock, M>
where
    Self: CycleOnce,
{
    pub fn run_forever(mut self) {
        // TODO - add a way to stop the runtime
        loop {
            self.cycle_once();
        }
    }
}

impl<M: ExecutionMode> Runtime<HistoricalClock, M> {
    pub fn run_until_completion(mut self) {
        while !self.clock.is_exhausted() {
            self.executor
                .cycle(self.clock.trigger_time(), Some(Duration::ZERO))
                .ok();
        }
    }
}

impl<M: ExecutionMode> Runtime<TestClock, M> {
    pub fn run_one_cycle(&mut self) {
        self.cycle_once();
    }
}

impl CycleOnce for Runtime<PrecisionClock, Spin> {
    #[inline(always)]
    fn cycle_once(&mut self) {
        self.executor
            .cycle(self.clock.trigger_time(), Some(Duration::ZERO))
            .ok();
    }
}

impl CycleOnce for Runtime<PrecisionClock, Sleep> {
    #[inline(always)]
    fn cycle_once(&mut self) {
        let now = self.clock.trigger_time();
        let duration = self
            .executor
            .next_timer()
            .map(|when| (when.saturating_duration_since(now.instant)).min(self.mode.0));
        self.executor.cycle(now, duration).ok();
    }
}

impl CycleOnce for Runtime<PrecisionClock, Block> {
    #[inline(always)]
    fn cycle_once(&mut self) {
        let now = self.clock.trigger_time();
        let duration = self.executor.next_timer().map(|when| {
            when.saturating_duration_since(now.instant)
                .max(MINIMUM_TIMER_PRECISION)
        });
        self.executor.cycle(now, duration).ok();
    }
}

impl<M: ExecutionMode> CycleOnce for Runtime<TestClock, M> {
    #[inline(always)]
    fn cycle_once(&mut self) {
        self.executor
            .cycle(self.clock.trigger_time(), Some(Duration::ZERO))
            .ok();
    }
}

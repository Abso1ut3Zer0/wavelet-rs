use crate::clock::{Clock, HistoricalClock, PrecisionClock, TestClock};
use crate::executor::{Executor, ExecutorState};
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
    fn cycle_once(&mut self) -> ExecutorState;
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
            halted: false,
        })
    }
}

pub struct Runtime<C: Clock, M: ExecutionMode> {
    executor: Executor,
    clock: C,
    mode: M,
    halted: bool,
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

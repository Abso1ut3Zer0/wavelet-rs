mod io_driver;
mod timer_driver;
mod yield_driver;

pub use crate::event_driver::io_driver::*;
pub use crate::event_driver::timer_driver::*;
pub use crate::event_driver::yield_driver::*;
use crate::graph::Graph;
use crate::scheduler::Scheduler;
use std::io;
use std::time::{Duration, Instant};

const IO_CAPACITY: usize = 1024;

pub struct EventDriver {
    io_driver: IoDriver,
    timer_driver: TimerDriver,
    yield_driver: YieldDriver,
}

impl EventDriver {
    pub(crate) fn new() -> Self {
        Self::with_capacity(IO_CAPACITY)
    }

    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            io_driver: IoDriver::with_capacity(capacity),
            timer_driver: TimerDriver::new(),
            yield_driver: YieldDriver::new(),
        }
    }

    pub const fn io_driver(&mut self) -> &mut IoDriver {
        &mut self.io_driver
    }

    pub const fn timer_driver(&mut self) -> &mut TimerDriver {
        &mut self.timer_driver
    }

    pub const fn yield_driver(&mut self) -> &mut YieldDriver {
        &mut self.yield_driver
    }

    #[inline(always)]
    pub(crate) fn poll(
        &mut self,
        graph: &mut Graph,
        scheduler: &mut Scheduler,
        timeout: Option<Duration>,
        now: Instant,
        epoch: usize,
    ) -> io::Result<()> {
        self.yield_driver.poll(graph, scheduler, epoch);
        self.timer_driver.poll(graph, scheduler, now, epoch);
        self.io_driver.poll(graph, scheduler, timeout, epoch)
    }
}

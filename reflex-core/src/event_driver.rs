mod io_driver;
mod timer_driver;

pub use crate::event_driver::io_driver::IoDriver;
pub use crate::event_driver::timer_driver::TimerDriver;
use crate::graph_manager::GraphManager;
use std::io;
use std::time::{Duration, Instant};

const IO_CAPACITY: usize = 1024;

pub struct EventDriver {
    io_driver: IoDriver,
    timer_driver: TimerDriver,
}

impl EventDriver {
    pub fn new() -> Self {
        Self::with_capacity(IO_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            io_driver: IoDriver::with_capacity(capacity),
            timer_driver: TimerDriver::new(),
        }
    }

    pub const fn io_driver(&mut self) -> &mut IoDriver {
        &mut self.io_driver
    }

    pub const fn timer_driver(&mut self) -> &mut TimerDriver {
        &mut self.timer_driver
    }

    #[inline(always)]
    pub fn poll(
        &mut self,
        graph_manager: &mut GraphManager,
        timeout: Option<Duration>,
        now: Instant,
        epoch: usize,
    ) -> io::Result<()> {
        self.timer_driver.poll(graph_manager, now, epoch);
        self.io_driver.poll(graph_manager, timeout, epoch)
    }
}

mod io_driver;
mod timer_driver;
mod yield_driver;

pub use crate::runtime::event_driver::io_driver::*;
pub use crate::runtime::event_driver::timer_driver::*;
pub use crate::runtime::event_driver::yield_driver::*;
use crate::runtime::graph::Graph;
use crate::runtime::scheduler::Scheduler;
use std::io;
use std::time::{Duration, Instant};

const IO_CAPACITY: usize = 1024;

/// Unified event management system that coordinates all event sources.
///
/// The `EventDriver` orchestrates the three core event types in the runtime:
/// - **Yield events**: Immediate scheduling requests from nodes
/// - **Timer events**: Time-based scheduling for delayed execution
/// - **I/O events**: External events from network, files, or user input
///
/// By centralizing event management, the `EventDriver` ensures consistent
/// scheduling behavior and provides a single polling interface for the
/// main execution loop.
pub struct EventDriver {
    /// Handles I/O sources and external notifications
    io_driver: IoDriver,

    /// Manages time-based node scheduling
    timer_driver: TimerDriver,

    /// Processes immediate yield requests
    yield_driver: YieldDriver,
}

impl EventDriver {
    /// Creates a new event driver with default I/O capacity.
    pub(crate) fn new() -> Self {
        Self::with_capacity(IO_CAPACITY)
    }

    /// Creates a new event driver with the specified I/O event capacity.
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            io_driver: IoDriver::with_capacity(capacity),
            timer_driver: TimerDriver::new(),
            yield_driver: YieldDriver::new(),
        }
    }

    /// Provides mutable access to the I/O driver for source registration.
    pub const fn io_driver(&mut self) -> &mut IoDriver {
        &mut self.io_driver
    }

    /// Provides mutable access to the timer driver for timer management.
    pub const fn timer_driver(&mut self) -> &mut TimerDriver {
        &mut self.timer_driver
    }

    /// Provides mutable access to the yield driver for immediate scheduling.
    pub const fn yield_driver(&mut self) -> &mut YieldDriver {
        &mut self.yield_driver
    }

    /// Polls all event sources and schedules ready nodes.
    ///
    /// Coordinates event polling across all three drivers in a specific order:
    /// 1. **Yield events**: Process immediate scheduling requests first
    /// 2. **Timer events**: Check for expired timers based on current time
    /// 3. **I/O events**: Poll external sources with the specified timeout
    ///
    /// This ordering ensures that internal scheduling takes precedence over
    /// external events, and that I/O polling uses any remaining timeout budget
    /// after processing internal events.
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

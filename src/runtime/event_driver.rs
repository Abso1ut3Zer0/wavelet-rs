mod io_driver;
mod timer_driver;
mod yield_driver;

pub use crate::runtime::event_driver::io_driver::*;
pub use crate::runtime::event_driver::timer_driver::*;
pub use crate::runtime::event_driver::yield_driver::*;
use crate::runtime::graph::Graph;
use crate::runtime::scheduler::Scheduler;
use ahash::{HashSet, HashSetExt};
use petgraph::prelude::NodeIndex;
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

const IO_CAPACITY: usize = 1024;
const EVENT_CAPACITY: usize = 1024;

/// A handle for waking a node from external threads or contexts.
///
/// `Notifier` provides a thread-safe way to schedule a node for execution
/// from outside the main event loop. Useful for integrating with external
/// libraries, user input, or cross-thread communication.
pub struct Notifier {
    raw_events: Arc<spin::Mutex<HashSet<NodeIndex>>>,
    waker: Option<Arc<mio::Waker>>,
    node_index: NodeIndex,
}

impl Clone for Notifier {
    fn clone(&self) -> Self {
        Self::new(self.raw_events.clone(), self.waker.clone(), self.node_index)
    }
}

impl Notifier {
    /// Creates a new notifier handle (internal use only).
    const fn new(
        raw_events: Arc<spin::Mutex<HashSet<NodeIndex>>>,
        waker: Option<Arc<mio::Waker>>,
        node_index: NodeIndex,
    ) -> Self {
        Self {
            raw_events,
            waker,
            node_index,
        }
    }

    /// Wakes the associated node, causing it to be scheduled for execution.
    ///
    /// This method is thread-safe and can be called from any thread.
    /// The node will be scheduled on the next polling cycle.
    #[inline(always)]
    pub fn notify(&self) {
        self.raw_events.lock().insert(self.node_index);
        self.waker.as_ref().map(|waker| waker.wake().ok());
    }
}

/// Unified event management system that coordinates all event sources.
///
/// The `EventDriver` orchestrates the three core event types in the runtime:
/// - **Yield events**: Immediate scheduling requests from wsnl
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

    /// Tracks deduplicated raw events that have been received
    raw_events: Arc<spin::Mutex<HashSet<NodeIndex>>>,
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
            raw_events: Arc::new(spin::Mutex::new(HashSet::with_capacity(EVENT_CAPACITY))),
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

    /// Creates a new `Notifier` to inform the event driver of a raw event.
    #[inline(always)]
    pub fn register_notifier(&self, node_index: NodeIndex) -> Notifier {
        Notifier::new(
            self.raw_events.clone(),
            Some(self.io_driver.waker().clone()),
            node_index,
        )
    }

    /// Polls all event sources and schedules ready wsnl.
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
        {
            let mut raw_events = self.raw_events.lock();
            raw_events.drain().for_each(|node_idx| {
                if let Some(depth) = graph.can_schedule(node_idx, epoch) {
                    scheduler.schedule(node_idx, depth).ok();
                }
            });
        }
        self.yield_driver.poll(graph, scheduler, epoch);
        self.timer_driver.poll(graph, scheduler, now, epoch);
        self.io_driver.poll(graph, scheduler, timeout, epoch)
    }
}

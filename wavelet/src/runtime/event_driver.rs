mod io_driver;
mod timer_driver;
mod yield_driver;

use crate::prelude::ExecutionMode;
pub use crate::runtime::event_driver::io_driver::*;
pub use crate::runtime::event_driver::timer_driver::*;
pub use crate::runtime::event_driver::yield_driver::*;
use crate::runtime::graph::Graph;
use crate::runtime::scheduler::Scheduler;
use crate::runtime::{Clock, CycleTime};
use crossbeam_queue::ArrayQueue;
use derive_builder::Builder;
use petgraph::prelude::NodeIndex;
use std::io;
use std::sync::Arc;
use std::time::Duration;

const MINIMUM_TIMER_PRECISION: Duration = Duration::from_millis(1);

/// A handle for waking a node from external threads or contexts.
///
/// `Notifier` provides a thread-safe way to schedule a node for execution
/// from outside the main event loop. Useful for integrating with external
/// libraries, user input, or cross-thread communication.
pub struct Notifier {
    notifications: Arc<ArrayQueue<NodeIndex>>,
    waker: Option<Arc<mio::Waker>>,
    node_index: NodeIndex,
}

impl Clone for Notifier {
    fn clone(&self) -> Self {
        Self::new(
            self.notifications.clone(),
            self.waker.clone(),
            self.node_index,
        )
    }
}

impl Notifier {
    /// Creates a new notifier handle (internal use only).
    const fn new(
        notifications: Arc<ArrayQueue<NodeIndex>>,
        waker: Option<Arc<mio::Waker>>,
        node_index: NodeIndex,
    ) -> Self {
        Self {
            notifications,
            waker,
            node_index,
        }
    }

    /// Wakes the associated node, causing it to be scheduled for execution.
    ///
    /// This method is thread-safe and can be called from any thread.
    /// The node will be scheduled on the next polling cycle.
    ///
    /// Note: we assume that notification events will coalesce,
    /// so we only attempt to write to the notification queue.
    #[inline(always)]
    pub fn notify(&self) {
        self.notifications.push(self.node_index).ok();
        std::sync::atomic::fence(std::sync::atomic::Ordering::Release);
        self.waker.as_ref().map(|waker| waker.wake().ok());
    }
}

/// Driver configuration options.
#[derive(Builder)]
pub struct EventDriverConfig {
    #[builder(default = true)]
    pub io_enabled: bool,
    #[builder(default = true)]
    pub timer_enabled: bool,
    #[builder(default = 256)]
    pub notification_capacity: usize,
    #[builder(default = 1024)]
    pub io_capacity: usize,
    #[builder(default = 16)]
    pub poll_limit: usize,
}

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
    io_driver: Option<IoDriver>,

    /// Manages time-based node scheduling
    timer_driver: Option<TimerDriver>,

    /// Processes immediate yield requests
    yield_driver: YieldDriver,

    /// Notification queue tracking events on node indices
    notifications: Arc<ArrayQueue<NodeIndex>>,

    /// Poll limit for pulling node indices off of
    /// the notifications queue
    poll_limit: usize,

    /// Execution mode
    mode: ExecutionMode,
}

impl EventDriver {
    /// Creates a new event driver with default I/O capacity.
    pub(crate) fn new(mode: ExecutionMode) -> Self {
        Self::with_config(
            EventDriverConfigBuilder::default()
                .build()
                .expect("expected default builder"),
            mode,
        )
    }

    /// Creates a new event driver with the specified notification capacity.
    pub(crate) fn with_config(cfg: EventDriverConfig, mode: ExecutionMode) -> Self {
        Self {
            io_driver: if cfg.io_enabled {
                Some(IoDriver::with_capacity(cfg.io_capacity))
            } else {
                None
            },
            timer_driver: if cfg.timer_enabled {
                Some(TimerDriver::new())
            } else {
                None
            },
            yield_driver: YieldDriver::new(),
            notifications: Arc::new(ArrayQueue::new(cfg.notification_capacity)),
            poll_limit: cfg.poll_limit,
            mode,
        }
    }

    /// Provides mutable access to the I/O driver for source registration.
    pub const fn io_driver(&mut self) -> &mut IoDriver {
        self.io_driver.as_mut().expect("no io driver configured")
    }

    /// Provides mutable access to the timer driver for timer management.
    pub const fn timer_driver(&mut self) -> &mut TimerDriver {
        self.timer_driver
            .as_mut()
            .expect("no timer driver configured")
    }

    /// Provides mutable access to the yield driver for immediate scheduling.
    pub const fn yield_driver(&mut self) -> &mut YieldDriver {
        &mut self.yield_driver
    }

    /// Creates a new `Notifier` to inform the event driver of a raw event.
    #[inline(always)]
    pub fn register_notifier(&self, node_index: NodeIndex) -> Notifier {
        Notifier::new(
            self.notifications.clone(),
            self.io_driver.as_ref().map(|io| io.waker()),
            node_index,
        )
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
        clock: &mut impl Clock,
        timeout: Option<Duration>,
        epoch: usize,
    ) -> io::Result<CycleTime> {
        // Ensure notification queue updates are visible across threads
        // before polling the notification queue. This has shown to reduce
        // long tails on the scheduler.
        if self.io_driver.is_none() {
            std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);
        }

        for _ in 0..self.poll_limit {
            match self.notifications.pop() {
                None => break,
                Some(node_idx) => {
                    if let Some(depth) = graph.can_schedule(node_idx, epoch) {
                        scheduler
                            .schedule(node_idx, depth)
                            .expect("failed to schedule node");
                    }
                }
            }
        }

        let cycle_time = clock.cycle_time();

        // Poll yield driver (always enabled)
        self.yield_driver.poll(graph, scheduler, epoch);

        // Poll timer driver (if enabled)
        if let Some(ref mut driver) = self.timer_driver {
            driver.poll(graph, scheduler, cycle_time.now(), epoch);
        }

        // Early exit if work is ready or a zero timeout is specified
        if scheduler.has_pending_event() || timeout == Some(Duration::ZERO) {
            return self.fast_poll(graph, scheduler, cycle_time, epoch);
        }

        self.slow_poll(graph, scheduler, clock, cycle_time, timeout, epoch)
    }

    #[inline(always)]
    fn fast_poll(
        &mut self,
        graph: &mut Graph,
        scheduler: &mut Scheduler,
        cycle_time: CycleTime,
        epoch: usize,
    ) -> io::Result<CycleTime> {
        if let Some(ref mut driver) = self.io_driver {
            driver.poll(graph, scheduler, Some(Duration::ZERO), epoch)?;
        }
        Ok(cycle_time)
    }

    #[cold]
    #[inline(never)]
    fn slow_poll(
        &mut self,
        graph: &mut Graph,
        scheduler: &mut Scheduler,
        clock: &mut impl Clock,
        cycle_time: CycleTime,
        timeout: Option<Duration>,
        epoch: usize,
    ) -> io::Result<CycleTime> {
        // Calculate effective timeout
        let effective_timeout = match (&self.timer_driver, timeout) {
            (Some(timer), Some(t)) => timer
                .next_timer()
                .map(|deadline| {
                    let d = deadline.saturating_duration_since(cycle_time.now());
                    t.min(d).max(MINIMUM_TIMER_PRECISION)
                })
                .or(Some(t)),
            (Some(timer), None) => timer
                .next_timer()
                .map(|deadline| deadline.saturating_duration_since(cycle_time.now())),
            (None, t) => t,
        };

        if let Some(ref mut driver) = self.io_driver {
            driver.poll(graph, scheduler, effective_timeout, epoch)?;
            if self.mode.is_park() && effective_timeout != Some(Duration::ZERO) {
                Ok(clock.cycle_time())
            } else {
                Ok(cycle_time)
            }
        } else {
            Ok(cycle_time)
        }
    }
}

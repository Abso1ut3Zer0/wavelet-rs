use crate::clock::Clock;
use crate::event_driver::{EventDriver, IoDriver, TimerDriver};
use crate::graph::Graph;
use crate::node::Node;
use crate::prelude::{Scheduler, YieldDriver};
use petgraph::graph::NodeIndex;
use std::cell::UnsafeCell;
use std::io;
use std::time::{Duration, Instant};
use time::OffsetDateTime;

const BUFFER_CAPACITY: usize = 32;

pub struct ExecutionContext<'a> {
    event_driver: &'a mut EventDriver,
    scheduler: &'a UnsafeCell<Scheduler>,
    current: NodeIndex,
    now: Instant,
    trigger_time: OffsetDateTime,
    epoch: usize,
}

impl<'a> ExecutionContext<'a> {
    pub(crate) fn new(
        event_driver: &'a mut EventDriver,
        scheduler: &'a UnsafeCell<Scheduler>,
        now: Instant,
        trigger_time: OffsetDateTime,
        epoch: usize,
    ) -> Self {
        Self {
            event_driver,
            scheduler,
            current: NodeIndex::new(0),
            now,
            trigger_time,
            epoch,
        }
    }

    pub const fn io_driver(&mut self) -> &mut IoDriver {
        self.event_driver.io_driver()
    }

    pub const fn timer_driver(&mut self) -> &mut TimerDriver {
        self.event_driver.timer_driver()
    }

    pub const fn yield_driver(&mut self) -> &mut YieldDriver {
        self.event_driver.yield_driver()
    }

    pub const fn current(&self) -> NodeIndex {
        self.current
    }

    pub const fn now(&self) -> Instant {
        self.now
    }

    pub const fn trigger_time(&self) -> OffsetDateTime {
        self.trigger_time
    }

    const fn set_current(&mut self, node_index: NodeIndex) {
        self.current = node_index;
    }

    #[inline(always)]
    pub fn schedule_node<T>(&mut self, node: &Node<T>) {
        unsafe { (&mut *self.scheduler.get()).schedule(node.index(), node.depth()) }
    }

    #[inline(always)]
    pub fn has_mutated<T>(&self, parent: Node<T>) -> bool {
        parent.mut_epoch() == self.epoch
    }
}

pub struct Executor {
    graph: Graph,
    /// SAFETY: Scheduler access through UnsafeCell is safe because:
    /// 1. Single-threaded execution only
    /// 2. All mutable accesses are temporally separated (no simultaneous borrows)
    /// 3. Access pattern: pop() → node execution → schedule() → repeat
    /// Each step releases its mutable reference before the next begins.
    scheduler: UnsafeCell<Scheduler>,
    event_driver: EventDriver,
    edge_buffer: Vec<NodeIndex>,
    epoch: usize,
}

impl Executor {
    pub fn new() -> Self {
        Self {
            graph: Graph::new(),
            scheduler: UnsafeCell::new(Scheduler::new()),
            event_driver: EventDriver::new(),
            edge_buffer: Vec::with_capacity(BUFFER_CAPACITY),
            epoch: 0,
        }
    }

    #[inline(always)]
    pub fn next_timer(&mut self) -> Option<Instant> {
        self.event_driver.timer_driver().next_timer()
    }

    pub fn cycle(&mut self, clock: &impl Clock, timeout: Option<Duration>) -> io::Result<()> {
        // Increment executor epoch
        self.epoch = self.epoch.wrapping_add(1);

        // Snap clock times
        let now = clock.now();
        let trigger_time = clock.trigger_time();

        // Poll for external events
        self.event_driver.poll(
            &mut self.graph,
            unsafe { &mut *self.scheduler.get() },
            timeout,
            now,
            self.epoch,
        )?;

        let mut ctx = ExecutionContext::new(
            &mut self.event_driver,
            &self.scheduler,
            now,
            trigger_time,
            self.epoch,
        );

        // Process nodes in the graph
        while let Some(node_idx) = unsafe { (&mut *self.scheduler.get()).pop() } {
            ctx.set_current(node_idx);
            if self.graph.mutate(&mut ctx, node_idx) {
                self.edge_buffer
                    .extend(self.graph.triggering_edges(node_idx));
                self.edge_buffer.drain(..).for_each(|child| {
                    self.graph
                        .can_schedule(child, self.epoch)
                        .map(|depth| unsafe {
                            (&mut *self.scheduler.get()).schedule(child, depth)
                        });
                })
            }
        }

        // TODO - can add in the garbage collector and node spawner to be run here
        Ok(())
    }
}

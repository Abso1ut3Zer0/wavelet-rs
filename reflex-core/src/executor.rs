use crate::clock::Clock;
use crate::event_driver::{EventDriver, IoDriver, TimerDriver};
use crate::graph::Graph;
use crate::node::Node;
use crate::prelude::Scheduler;
use petgraph::graph::NodeIndex;
use std::io;
use std::time::{Duration, Instant};
use time::OffsetDateTime;

const EDGE_BUFFER_CAPACITY: usize = 32;

pub struct ExecutionContext<'a> {
    event_driver: &'a mut EventDriver,
    scheduler: &'a mut Scheduler,
    now: Instant,
    wall_time: OffsetDateTime,
    epoch: usize,
}

impl<'a> ExecutionContext<'a> {
    pub(crate) const fn new(
        event_driver: &'a mut EventDriver,
        scheduler: &'a mut Scheduler,
        now: Instant,
        wall_time: OffsetDateTime,
        epoch: usize,
    ) -> Self {
        Self {
            event_driver,
            scheduler,
            now,
            wall_time,
            epoch,
        }
    }

    pub const fn io_driver(&mut self) -> &mut IoDriver {
        self.event_driver.io_driver()
    }

    pub const fn timer_driver(&mut self) -> &mut TimerDriver {
        self.event_driver.timer_driver()
    }

    #[inline(always)]
    pub fn schedule_node(&mut self, node_index: NodeIndex, depth: u32) {
        self.scheduler.schedule(node_index, depth);
    }

    #[inline(always)]
    pub fn has_mutated<T>(&self, parent: Node<T>) -> bool {
        parent.mut_epoch() == self.epoch
    }
}

pub struct Executor {
    graph: Graph,
    scheduler: Scheduler,
    event_driver: EventDriver,
    edge_buffer: Vec<NodeIndex>,
    epoch: usize,
}

impl Executor {
    pub fn new() -> Self {
        Self {
            graph: Graph::new(),
            scheduler: Scheduler::new(),
            event_driver: EventDriver::new(),
            edge_buffer: Vec::with_capacity(EDGE_BUFFER_CAPACITY),
            epoch: 0,
        }
    }

    pub fn cycle(&mut self, clock: &impl Clock, timeout: Option<Duration>) -> io::Result<()> {
        // Increment executor epoch
        self.epoch = self.epoch.wrapping_add(1);

        // Snap clock times
        let now = clock.now();
        let wall_time = clock.wall_time();

        // Poll for external events
        self.event_driver.poll(
            &mut self.graph,
            &mut self.scheduler,
            timeout,
            now,
            self.epoch,
        )?;

        // Process nodes in the graph
        while let Some(node_idx) = self.scheduler.pop() {
            let ctx = ExecutionContext::new(
                &mut self.event_driver,
                &mut self.scheduler,
                now,
                wall_time,
                self.epoch,
            );

            if self.graph.mutate(ctx, node_idx) {
                self.edge_buffer
                    .extend(self.graph.triggering_edges(node_idx));
                self.edge_buffer.drain(..).for_each(|child| {
                    self.graph
                        .can_schedule(child, self.epoch)
                        .map(|depth| self.scheduler.schedule(child, depth));
                })
            }
        }

        Ok(())
    }
}

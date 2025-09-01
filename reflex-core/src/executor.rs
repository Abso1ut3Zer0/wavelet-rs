use crate::event_driver::{EventDriver, IoDriver, TimerDriver};
use crate::node::Node;
use crate::prelude::Scheduler;
use petgraph::graph::NodeIndex;

pub struct ExecutionContext<'a> {
    event_driver: &'a mut EventDriver,
    scheduler: &'a mut Scheduler,
    epoch: usize,
}

impl<'a> ExecutionContext<'a> {
    pub(crate) const fn new(
        event_driver: &'a mut EventDriver,
        scheduler: &'a mut Scheduler,
        epoch: usize,
    ) -> Self {
        Self {
            event_driver,
            scheduler,
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

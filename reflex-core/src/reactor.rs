use crate::node::Node;
use petgraph::prelude::NodeIndex;
use std::io;
use std::time::Duration;

const INITIAL_EVENT_CAPACITY: usize = 1024;

pub struct Reactor {
    poller: mio::Poll,
    events: mio::Events,
    garbage: Vec<NodeIndex>,
    current_epoch: usize,
    trigger_time: i128,
}

impl Reactor {
    pub fn new() -> Self {
        Self::with_capacity(INITIAL_EVENT_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Reactor {
        Reactor {
            poller: mio::Poll::new().expect("failed to create mio poll"),
            events: mio::Events::with_capacity(capacity),
            garbage: Vec::new(),
            current_epoch: 0,
            trigger_time: 0,
        }
    }

    pub const fn trigger_time(&self) -> i128 {
        self.trigger_time
    }

    #[inline(always)]
    pub fn has_mutated<T>(&self, node: &Node<T>) -> bool {
        node.has_mutated(self.current_epoch)
    }

    #[inline(always)]
    pub(crate) fn poll(&mut self, timeout: Option<Duration>) -> Result<(), io::Error> {
        self.poller.poll(&mut self.events, timeout)
    }

    pub(crate) const fn increment_epoch(&mut self) {
        self.current_epoch = self.current_epoch.wrapping_add(1);
    }

    #[inline(always)]
    pub(crate) fn garbage_collect(&mut self, f: impl FnMut(NodeIndex)) {
        self.garbage.drain(..).for_each(f)
    }

    #[inline(always)]
    pub(crate) fn register_garbage(&mut self, node: NodeIndex) {
        self.garbage.push(node);
    }
}

mod io_driver;
mod timer_driver;

use crate::node::Node;
use crate::reactor::timer_driver::TimerDriver;
use petgraph::prelude::NodeIndex;
use slab::Slab;
use std::io;
use std::time::Duration;

const INITIAL_EVENT_CAPACITY: usize = 1024;

pub struct Reactor {
    timer_wheel: TimerDriver,
    poller: mio::Poll,
    events: mio::Events,
    slab: Slab<NodeIndex>,
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
            slab: Slab::with_capacity(capacity),
            timer_wheel: TimerDriver::new(),
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

    // #[inline(always)]
    // pub fn register_timer(&mut self, idx: NodeIndex, when: i128) -> TimerRegistration {
    //     self.timer_wheel.register_timer(idx, when)
    // }
    //
    // #[inline(always)]
    // pub fn deregister_timer(&mut self, registration: TimerRegistration) {
    //     self.timer_wheel.deregister_timer(registration)
    // }

    #[inline(always)]
    pub fn register_waker(&mut self, idx: NodeIndex) -> Result<mio::Waker, io::Error> {
        let entry = self.slab.vacant_entry();
        let token = mio::Token(entry.key());
        entry.insert(idx);
        mio::Waker::new(self.poller.registry(), token)
    }

    #[inline(always)]
    pub fn deregister_waker(&mut self, token: mio::Token) {
        self.slab.remove(token.0);
    }

    #[inline(always)]
    pub(crate) fn poll(&mut self, timeout: Option<Duration>) -> Result<(), io::Error> {
        self.poller.poll(&mut self.events, timeout)
    }

    pub(crate) const fn increment_epoch(&mut self) {
        self.current_epoch = self.current_epoch.wrapping_add(1);
    }

    #[inline(always)]
    pub(crate) fn collect_garbage(&mut self, f: impl FnMut(NodeIndex)) {
        self.garbage.drain(..).for_each(f)
    }

    #[inline(always)]
    pub(crate) fn register_garbage(&mut self, node: NodeIndex) {
        self.garbage.push(node);
    }
}

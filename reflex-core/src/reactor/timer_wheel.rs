use crate::scheduler::Scheduler;
use petgraph::prelude::NodeIndex;
use std::collections::BTreeMap;
use std::time::Instant;

pub struct TimerHandle(TimerRegistration);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct TimerRegistration {
    pub when: Instant,
    pub id: usize,
}

pub struct TimerWheel {
    timers: BTreeMap<TimerRegistration, NodeIndex>,
    sequence: usize,
}

impl TimerWheel {
    pub fn new() -> Self {
        Self {
            timers: BTreeMap::new(),
            sequence: 0,
        }
    }

    #[inline(always)]
    pub fn register_timer(&mut self, idx: NodeIndex, when: Instant) -> TimerRegistration {
        let registration = TimerRegistration {
            when,
            id: self.sequence,
        };
        self.sequence = self.sequence.wrapping_add(1);
        self.timers.insert(registration.clone(), idx);
        registration
    }

    #[inline(always)]
    pub fn deregister_timer(&mut self, registration: TimerRegistration) {
        self.timers.remove(&registration);
    }

    #[inline(always)]
    pub fn poll(&mut self, scheduler: &mut Scheduler, now: Instant, epoch: usize) {
        while let Some(entry) = self.timers.first_entry() {
            if entry.key().when <= now {
                let (_, node_idx) = entry.remove_entry();
                scheduler.schedule_node(node_idx, epoch);
                continue;
            }
            return;
        }
    }
}

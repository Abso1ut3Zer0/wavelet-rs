use petgraph::prelude::NodeIndex;
use std::collections::BTreeMap;

pub struct TimerHandle(TimerRegistration);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct TimerRegistration {
    pub when: i128,
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
    pub fn register_timer(&mut self, idx: NodeIndex, when: i128) -> TimerRegistration {
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
    pub fn poll(&mut self, ) {}
}

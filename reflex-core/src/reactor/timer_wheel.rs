use crate::graph_manager::GraphManager;
use petgraph::prelude::NodeIndex;
use std::collections::BTreeMap;
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimerSource {
    when: Instant,
    id: usize,
}

pub struct TimerWheel {
    timers: BTreeMap<TimerSource, NodeIndex>,
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
    pub fn register_timer(&mut self, idx: NodeIndex, when: Instant) -> TimerSource {
        let registration = TimerSource {
            when,
            id: self.sequence,
        };
        self.sequence = self.sequence.wrapping_add(1);
        self.timers.insert(registration.clone(), idx);
        registration
    }

    #[inline(always)]
    pub fn deregister_timer(&mut self, registration: TimerSource) {
        self.timers.remove(&registration);
    }

    #[inline(always)]
    pub fn poll(&mut self, scheduler: &mut GraphManager, now: Instant, epoch: usize) {
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

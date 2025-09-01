use crate::graph::Graph;
use crate::scheduler::Scheduler;
use petgraph::prelude::NodeIndex;
use std::collections::BTreeMap;
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimerSource {
    when: Instant,
    id: usize,
}

pub struct TimerDriver {
    timers: BTreeMap<TimerSource, NodeIndex>,
    sequence: usize,
}

impl TimerDriver {
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
    pub fn poll(
        &mut self,
        graph: &mut Graph,
        scheduler: &mut Scheduler,
        now: Instant,
        epoch: usize,
    ) {
        while let Some(entry) = self.timers.first_entry() {
            if entry.key().when <= now {
                let (_, node_idx) = entry.remove_entry();
                if let Some(depth) = graph.can_schedule(node_idx, epoch) {
                    scheduler.schedule(node_idx, depth);
                }
                continue;
            }
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{Graph, NodeContext};
    use crate::scheduler::Scheduler;
    use std::time::{Duration, Instant};

    #[test]
    fn test_timer_driver_creation() {
        let driver = TimerDriver::new();
        assert_eq!(driver.sequence, 0);
        assert!(driver.timers.is_empty());
    }

    #[test]
    fn test_register_timer() {
        let mut driver = TimerDriver::new();
        let node_idx = NodeIndex::from(42);
        let when = Instant::now() + Duration::from_millis(100);

        let registration = driver.register_timer(node_idx, when);

        // Should have incremented sequence
        assert_eq!(driver.sequence, 1);
        assert_eq!(registration.when, when);
        assert_eq!(registration.id, 0); // First registration gets id 0

        // Should be stored in timers map
        assert_eq!(driver.timers[&registration], node_idx);
    }

    #[test]
    fn test_multiple_timer_registrations() {
        let mut driver = TimerDriver::new();
        let now = Instant::now();

        let reg1 = driver.register_timer(NodeIndex::from(1), now + Duration::from_millis(100));
        let reg2 = driver.register_timer(NodeIndex::from(2), now + Duration::from_millis(200));
        let reg3 = driver.register_timer(NodeIndex::from(3), now + Duration::from_millis(100)); // Same time as reg1

        // Should have unique IDs even with same timestamp
        assert_eq!(reg1.id, 0);
        assert_eq!(reg2.id, 1);
        assert_eq!(reg3.id, 2);

        // Should be ordered correctly (earlier times first, then by ID)
        let keys: Vec<_> = driver.timers.keys().collect();
        assert_eq!(keys[0], &reg1); // Earlier time
        assert_eq!(keys[1], &reg3); // Same time as reg1, but higher ID
        assert_eq!(keys[2], &reg2); // Latest time
    }

    #[test]
    fn test_deregister_timer() {
        let mut driver = TimerDriver::new();
        let node_idx = NodeIndex::from(42);
        let when = Instant::now() + Duration::from_millis(100);

        let registration = driver.register_timer(node_idx, when);
        assert_eq!(driver.timers.len(), 1);

        driver.deregister_timer(registration);
        assert!(driver.timers.is_empty());
    }

    #[test]
    fn test_poll_no_expired_timers() {
        let mut driver = TimerDriver::new();
        let mut graph = Graph::new();
        let mut scheduler = Scheduler::new();
        scheduler.resize(5);

        let node_ctx = NodeContext::new(Box::new(|_| false), 1);
        let node_idx = graph.add_node(node_ctx);

        // Register a timer in the future
        let future_time = Instant::now() + Duration::from_secs(10);
        let _registration = driver.register_timer(node_idx, future_time);

        // Poll with current time - nothing should be scheduled
        driver.poll(&mut graph, &mut scheduler, Instant::now(), 1);

        assert!(scheduler.pop().is_none());
        assert_eq!(driver.timers.len(), 1); // Timer should still be there
    }

    #[test]
    fn test_poll_expired_timers() {
        let mut driver = TimerDriver::new();
        let mut graph = Graph::new();
        let mut scheduler = Scheduler::new();
        scheduler.resize(5);

        let node_ctx = NodeContext::new(Box::new(|_| false), 2);
        let node_idx = graph.add_node(node_ctx);

        // Register a timer in the past
        let past_time = Instant::now() - Duration::from_millis(100);
        let _registration = driver.register_timer(node_idx, past_time);

        // Poll with current time - should schedule the node
        driver.poll(&mut graph, &mut scheduler, Instant::now(), 1);

        let scheduled_node = scheduler.pop();
        assert_eq!(scheduled_node, Some(node_idx));
        assert!(driver.timers.is_empty()); // Timer should be removed after firing
    }

    #[test]
    fn test_poll_multiple_expired_timers() {
        let mut driver = TimerDriver::new();
        let mut graph = Graph::new();
        let mut scheduler = Scheduler::new();
        scheduler.resize(5);

        // Add multiple nodes
        let node1_ctx = NodeContext::new(Box::new(|_| false), 1);
        let node2_ctx = NodeContext::new(Box::new(|_| false), 3);
        let node3_ctx = NodeContext::new(Box::new(|_| false), 2);
        let node1_idx = graph.add_node(node1_ctx);
        let node2_idx = graph.add_node(node2_ctx);
        let node3_idx = graph.add_node(node3_ctx);

        let now = Instant::now();
        let past_time1 = now - Duration::from_millis(300);
        let past_time2 = now - Duration::from_millis(200);
        let past_time3 = now - Duration::from_millis(100);

        // Register timers in reverse order to test BTreeMap ordering
        driver.register_timer(node3_idx, past_time3);
        driver.register_timer(node1_idx, past_time1);
        driver.register_timer(node2_idx, past_time2);

        // Poll - should schedule all expired timers
        driver.poll(&mut graph, &mut scheduler, now, 1);

        // Should schedule in depth order (scheduler handles this)
        let mut scheduled = vec![];
        while let Some(node) = scheduler.pop() {
            scheduled.push(node);
        }

        assert_eq!(scheduled.len(), 3);
        // Scheduler should return in depth order: 1, 2, 3
        assert_eq!(scheduled[0], node1_idx); // depth 1
        assert_eq!(scheduled[1], node3_idx); // depth 2
        assert_eq!(scheduled[2], node2_idx); // depth 3

        assert!(driver.timers.is_empty()); // All timers should be consumed
    }

    #[test]
    fn test_poll_partial_expiry() {
        let mut driver = TimerDriver::new();
        let mut graph = Graph::new();
        let mut scheduler = Scheduler::new();
        scheduler.resize(5);

        let node1_ctx = NodeContext::new(Box::new(|_| false), 1);
        let node2_ctx = NodeContext::new(Box::new(|_| false), 1);
        let node1_idx = graph.add_node(node1_ctx);
        let node2_idx = graph.add_node(node2_ctx);

        let now = Instant::now();
        let past_time = now - Duration::from_millis(100);
        let future_time = now + Duration::from_millis(100);

        // One expired, one future
        driver.register_timer(node1_idx, past_time);
        driver.register_timer(node2_idx, future_time);

        driver.poll(&mut graph, &mut scheduler, now, 1);

        // Should only schedule the expired timer
        assert_eq!(scheduler.pop(), Some(node1_idx));
        assert!(scheduler.pop().is_none());

        // Future timer should still be in the map
        assert_eq!(driver.timers.len(), 1);
    }

    #[test]
    fn test_sequence_wraparound() {
        let mut driver = TimerDriver::new();
        driver.sequence = usize::MAX;

        let when = Instant::now();
        let reg = driver.register_timer(NodeIndex::from(1), when);

        // Should wrap around to 0
        assert_eq!(reg.id, usize::MAX);
        assert_eq!(driver.sequence, 0);
    }
}

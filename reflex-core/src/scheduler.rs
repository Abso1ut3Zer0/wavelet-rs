use petgraph::prelude::NodeIndex;
use std::collections::VecDeque;

const INITIAL_CAPACITY: usize = 256;

pub(crate) struct Scheduler {
    multi_queue: Vec<VecDeque<NodeIndex>>,
    curr_depth: usize,
}

impl Scheduler {
    pub(crate) fn new() -> Self {
        Self {
            multi_queue: Vec::new(),
            curr_depth: 0,
        }
    }

    pub(crate) fn resize(&mut self, max_depth: u32) {
        self.multi_queue.resize(
            max_depth as usize,
            VecDeque::with_capacity(INITIAL_CAPACITY),
        );
    }

    pub(crate) fn reserve(&mut self, max_depth: u32) {
        let len = self.multi_queue.len();
        self.multi_queue
            .reserve((max_depth as usize).saturating_sub(len));
    }

    #[inline(always)]
    pub fn schedule(&mut self, node_index: NodeIndex, depth: u32) {
        // We assert here since scheduling above
        // the current depth is undefined behavior
        // that can cause certain execution paths
        // no never run. This can lead to dropped
        // messages in processing.
        assert!(
            (depth as usize) >= self.curr_depth,
            "cannot schedule at a depth above the current queue"
        );
        self.multi_queue[depth as usize].push_back(node_index);
    }

    #[inline(always)]
    pub(crate) fn pop(&mut self) -> Option<NodeIndex> {
        // we must exhaust - see tests for an example
        while self.curr_depth < self.multi_queue.len() {
            if let Some(item) = self.multi_queue[self.curr_depth].pop_front() {
                return Some(item);
            }
            self.curr_depth += 1;
        }

        // reset the queue since we are done
        self.curr_depth = 0;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scheduler_basic() {
        let mut scheduler = Scheduler::new();
        scheduler.resize(5);

        scheduler.schedule(NodeIndex::from(0), 0);
        scheduler.schedule(NodeIndex::from(1), 1);
        scheduler.schedule(NodeIndex::from(2), 4);
        scheduler.schedule(NodeIndex::from(3), 2);
        scheduler.schedule(NodeIndex::from(4), 3);
        scheduler.schedule(NodeIndex::from(5), 1);
        scheduler.schedule(NodeIndex::from(6), 2);
        scheduler.schedule(NodeIndex::from(7), 4);

        // rank 0
        let item = scheduler.pop();
        assert!(item.is_some());
        let item = item.unwrap();
        assert_eq!(item, NodeIndex::from(0));
        assert_eq!(scheduler.curr_depth, 0);

        // rank 1
        let item = scheduler.pop();
        assert!(item.is_some());
        let item = item.unwrap();
        assert_eq!(item, NodeIndex::from(1));
        assert_eq!(scheduler.curr_depth, 1);

        // rank 1
        let item = scheduler.pop();
        assert!(item.is_some());
        let item = item.unwrap();
        assert_eq!(item, NodeIndex::from(5));
        assert_eq!(scheduler.curr_depth, 1);

        // rank 2
        let item = scheduler.pop();
        assert!(item.is_some());
        let item = item.unwrap();
        assert_eq!(item, NodeIndex::from(3));
        assert_eq!(scheduler.curr_depth, 2);

        // rank 2
        let item = scheduler.pop();
        assert!(item.is_some());
        let item = item.unwrap();
        assert_eq!(item, NodeIndex::from(6));
        assert_eq!(scheduler.curr_depth, 2);

        // rank 3
        let item = scheduler.pop();
        assert!(item.is_some());
        let item = item.unwrap();
        assert_eq!(item, NodeIndex::from(4));
        assert_eq!(scheduler.curr_depth, 3);

        // rank 4
        let item = scheduler.pop();
        assert!(item.is_some());
        let item = item.unwrap();
        assert_eq!(item, NodeIndex::from(2));
        assert_eq!(scheduler.curr_depth, 4);

        // rank 4
        let item = scheduler.pop();
        assert!(item.is_some());
        let item = item.unwrap();
        assert_eq!(item, NodeIndex::from(7));
        assert_eq!(scheduler.curr_depth, 4);

        let item = scheduler.pop();
        assert!(item.is_none());
    }

    /// Test Case: Observe vs Trigger Relationships
    ///
    /// ```text
    ///                               ┌──────────┐
    ///                               │Base Node │
    ///                               │ Depth 0  │
    ///                               └────┬─────┘
    ///                                    │
    ///                  ┌─────────────────┴──────────────┐
    ///                  │                                │
    ///              Observe                          Trigger
    ///                  │                                │
    ///                  ▼                                ▼
    ///           ┌────────────┐                  ┌────────────┐
    ///           │  Node A    │                  │  Node B    │
    ///           │  Depth 1   │───────────────▶  │  Depth 2   │
    ///           └────────────┘    Observe       └────────────┘
    /// ```
    ///
    /// When Base Node executes and mutates:
    /// - Node A is not scheduled (Observe relationship)
    /// - Node B is scheduled directly (Trigger relationship)
    /// - Scheduler must check Depth 2 even though Depth 1 queue is empty
    ///
    /// This test case demonstrates why the scheduler must check all depths
    /// rather than stopping at the first empty depth.
    #[test]
    fn test_scheduler_needs_to_exhaust() {
        let mut scheduler = Scheduler::new();
        scheduler.resize(3);

        scheduler.schedule(NodeIndex::from(0), 0);
        scheduler.schedule(NodeIndex::from(2), 2);
        // Node A never scheduled (observe)

        let item = scheduler.pop();
        assert!(item.is_some());
        let item = item.unwrap();
        assert_eq!(item, NodeIndex::from(0));
        assert_eq!(scheduler.curr_depth, 0);

        let item = scheduler.pop();
        assert!(item.is_some());
        let item = item.unwrap();
        assert_eq!(item, NodeIndex::from(2));
        assert_eq!(scheduler.curr_depth, 2);

        let item = scheduler.pop();
        assert!(item.is_none());
    }
}

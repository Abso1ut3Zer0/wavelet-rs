use petgraph::prelude::NodeIndex;
use std::collections::VecDeque;

const INITIAL_CAPACITY: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("cannot schedule a node above the current processing depth")]
pub struct SchedulerError;

#[derive(Debug)]
pub struct Scheduler {
    multi_queue: Vec<VecDeque<NodeIndex>>,
    curr_depth: usize,
    pending_events: usize,
}

impl Scheduler {
    pub(crate) fn new() -> Self {
        Self {
            multi_queue: Vec::new(),
            curr_depth: 0,
            pending_events: 0,
        }
    }

    pub(crate) const fn has_pending_event(&self) -> bool {
        self.pending_events > 0
    }

    pub(crate) fn resize(&mut self, max_depth: u32) {
        self.multi_queue.resize(
            max_depth as usize,
            VecDeque::with_capacity(INITIAL_CAPACITY),
        );
    }

    pub(crate) fn enable_depth(&mut self, depth: u32) {
        let required_len = depth + 1;
        if self.multi_queue.len() < required_len as usize {
            self.resize(required_len);
        }
    }

    #[inline(always)]
    pub(crate) fn schedule(
        &mut self,
        node_index: NodeIndex,
        depth: u32,
    ) -> Result<(), SchedulerError> {
        // Scheduling above the current depth is
        // considered undefined behavior that can
        // cause certain execution paths to never
        // run. This can lead to dropped messages
        // in processing.
        if (depth as usize) < self.curr_depth {
            return Err(SchedulerError);
        }
        self.multi_queue[depth as usize].push_back(node_index);
        self.pending_events += 1;
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn pop(&mut self) -> Option<NodeIndex> {
        // we must exhaust - see tests for an example
        while self.curr_depth < self.multi_queue.len() && self.has_pending_event() {
            if let Some(item) = self.multi_queue[self.curr_depth].pop_front() {
                debug_assert!(
                    self.pending_events > 0,
                    "pending_events underflow in scheduler pop"
                );
                self.pending_events -= 1;
                return Some(item);
            }
            self.curr_depth += 1;
        }

        // reset the queue since we are done
        self.curr_depth = 0;
        debug_assert!(
            self.pending_events == 0,
            "pending_events not reset after exhausting queue"
        );
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

        let _ = scheduler.schedule(NodeIndex::from(0), 0);
        let _ = scheduler.schedule(NodeIndex::from(1), 1);
        let _ = scheduler.schedule(NodeIndex::from(2), 4);
        let _ = scheduler.schedule(NodeIndex::from(3), 2);
        let _ = scheduler.schedule(NodeIndex::from(4), 3);
        let _ = scheduler.schedule(NodeIndex::from(5), 1);
        let _ = scheduler.schedule(NodeIndex::from(6), 2);
        let _ = scheduler.schedule(NodeIndex::from(7), 4);

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

        let _ = scheduler.schedule(NodeIndex::from(0), 0);
        let _ = scheduler.schedule(NodeIndex::from(2), 2);
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

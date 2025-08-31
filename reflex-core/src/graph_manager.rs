use crate::Relationship;
use crate::graph_manager::scheduler::Scheduler;
use crate::event_driver::EventDriver;
use petgraph::prelude::NodeIndex;

pub type CycleFn = Box<dyn FnMut(&mut EventDriver) -> bool + 'static>;

pub(crate) struct Context {
    pub(crate) cycle_fn: CycleFn,
    pub(crate) sched_epoch: usize,
    pub(crate) depth: u32,
}

impl Context {
    pub(crate) const fn new(cycle_fn: CycleFn, depth: u32) -> Self {
        Self {
            cycle_fn,
            sched_epoch: 0,
            depth,
        }
    }
}

pub struct GraphManager {
    graph: petgraph::Graph<Context, Relationship>,
    scheduler: Scheduler<NodeIndex>,
}

impl GraphManager {
    pub fn new() -> Self {
        Self {
            graph: petgraph::Graph::new(),
            scheduler: Scheduler::new(),
        }
    }

    #[inline(always)]
    pub fn schedule_node(&mut self, node: NodeIndex, epoch: usize) {
        let ctx = &mut self.graph[node];
        if ctx.sched_epoch != epoch {
            self.scheduler.push(node, ctx.depth);
            ctx.sched_epoch = epoch;
        }
    }

    #[inline(always)]
    pub(crate) fn add_node(&mut self, weight: Context) -> NodeIndex {
        self.scheduler.reserve(weight.depth);
        self.graph.add_node(weight)
    }

    #[inline(always)]
    pub(crate) fn add_edge(
        &mut self,
        parent: NodeIndex,
        child: NodeIndex,
        relationship: Relationship,
    ) {
        self.graph.add_edge(parent, child, relationship);
    }
}

mod scheduler {
    use std::collections::VecDeque;

    const INITIAL_CAPACITY: usize = 256;

    pub(crate) struct Scheduler<T> {
        pub multi_queue: Vec<VecDeque<T>>,
        curr_depth: usize,
    }

    impl<T: Clone> Scheduler<T> {
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
        pub(crate) fn push(&mut self, item: T, depth: u32) {
            self.multi_queue[depth as usize].push_back(item);
        }

        #[inline(always)]
        pub(crate) fn pop(&mut self) -> Option<T> {
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
        fn test_scheduler() {
            let mut scheduler = Scheduler::new();
            scheduler.resize(5);

            scheduler.push('a', 0);
            scheduler.push('b', 1);
            scheduler.push('c', 4);
            scheduler.push('d', 2);
            scheduler.push('e', 3);
            scheduler.push('f', 1);
            scheduler.push('g', 2);
            scheduler.push('h', 4);

            // rank 0
            let item = scheduler.pop();
            assert!(item.is_some());
            let item = item.unwrap();
            assert_eq!(item, 'a');
            assert_eq!(scheduler.curr_depth, 0);

            // rank 1
            let item = scheduler.pop();
            assert!(item.is_some());
            let item = item.unwrap();
            assert_eq!(item, 'b');
            assert_eq!(scheduler.curr_depth, 1);

            // rank 1
            let item = scheduler.pop();
            assert!(item.is_some());
            let item = item.unwrap();
            assert_eq!(item, 'f');
            assert_eq!(scheduler.curr_depth, 1);

            // rank 2
            let item = scheduler.pop();
            assert!(item.is_some());
            let item = item.unwrap();
            assert_eq!(item, 'd');
            assert_eq!(scheduler.curr_depth, 2);

            // rank 2
            let item = scheduler.pop();
            assert!(item.is_some());
            let item = item.unwrap();
            assert_eq!(item, 'g');
            assert_eq!(scheduler.curr_depth, 2);

            // rank 3
            let item = scheduler.pop();
            assert!(item.is_some());
            let item = item.unwrap();
            assert_eq!(item, 'e');
            assert_eq!(scheduler.curr_depth, 3);

            // rank 4
            let item = scheduler.pop();
            assert!(item.is_some());
            let item = item.unwrap();
            assert_eq!(item, 'c');
            assert_eq!(scheduler.curr_depth, 4);

            // rank 4
            let item = scheduler.pop();
            assert!(item.is_some());
            let item = item.unwrap();
            assert_eq!(item, 'h');
            assert_eq!(scheduler.curr_depth, 4);

            let item = scheduler.pop();
            assert!(item.is_none());
        }

        /// Test Case: Observe vs Trigger Relationships
        ///
        /// ```text
        ///                               ┌──────────┐
        ///                               │Base Node │
        ///                               │  Rank 0  │
        ///                               └────┬─────┘
        ///                                    │
        ///                  ┌─────────────────┴──────────────┐
        ///                  │                                │
        ///              Observe                          Trigger
        ///                  │                                │
        ///                  ▼                                ▼
        ///           ┌────────────┐                  ┌────────────┐
        ///           │  Node A    │                  │  Node B    │
        ///           │  Rank 1    │───────────────▶  │  Rank 2    │
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

            scheduler.push("Base Node", 0);
            scheduler.push("Node B", 2);
            // Node A never scheduled (observe)

            let item = scheduler.pop();
            assert!(item.is_some());
            let item = item.unwrap();
            assert_eq!(item, "Base Node");
            assert_eq!(scheduler.curr_depth, 0);

            let item = scheduler.pop();
            assert!(item.is_some());
            let item = item.unwrap();
            assert_eq!(item, "Node B");
            assert_eq!(scheduler.curr_depth, 2);

            let item = scheduler.pop();
            assert!(item.is_none());
        }
    }
}

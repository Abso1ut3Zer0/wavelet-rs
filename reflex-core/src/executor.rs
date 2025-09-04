use crate::clock::Clock;
use crate::event_driver::{EventDriver, IoDriver, TimerDriver};
use crate::graph::Graph;
use crate::node::Node;
use crate::prelude::{Scheduler, YieldDriver};
use petgraph::graph::NodeIndex;
use std::cell::UnsafeCell;
use std::io;
use std::time::{Duration, Instant};
use time::OffsetDateTime;

const BUFFER_CAPACITY: usize = 32;

pub struct ExecutionContext<'a> {
    event_driver: &'a mut EventDriver,
    scheduler: &'a UnsafeCell<Scheduler>,
    current: NodeIndex,
    now: Instant,
    trigger_time: OffsetDateTime,
    epoch: usize,
}

impl<'a> ExecutionContext<'a> {
    pub(crate) fn new(
        event_driver: &'a mut EventDriver,
        scheduler: &'a UnsafeCell<Scheduler>,
        now: Instant,
        trigger_time: OffsetDateTime,
        epoch: usize,
    ) -> Self {
        Self {
            event_driver,
            scheduler,
            current: NodeIndex::new(0),
            now,
            trigger_time,
            epoch,
        }
    }

    pub const fn io_driver(&mut self) -> &mut IoDriver {
        self.event_driver.io_driver()
    }

    pub const fn timer_driver(&mut self) -> &mut TimerDriver {
        self.event_driver.timer_driver()
    }

    pub const fn yield_driver(&mut self) -> &mut YieldDriver {
        self.event_driver.yield_driver()
    }

    pub const fn current(&self) -> NodeIndex {
        self.current
    }

    pub const fn now(&self) -> Instant {
        self.now
    }

    pub const fn trigger_time(&self) -> OffsetDateTime {
        self.trigger_time
    }

    const fn set_current(&mut self, node_index: NodeIndex) {
        self.current = node_index;
    }

    #[inline(always)]
    pub fn schedule_node<T>(&mut self, node: &Node<T>) {
        unsafe { (&mut *self.scheduler.get()).schedule(node.index(), node.depth()) }
    }

    #[inline(always)]
    pub fn has_mutated<T>(&self, parent: Node<T>) -> bool {
        parent.mut_epoch() == self.epoch
    }
}

pub struct Executor {
    graph: Graph,
    /// SAFETY: Scheduler access through UnsafeCell is safe because:
    /// 1. Single-threaded execution only
    /// 2. All mutable accesses are temporally separated (no simultaneous borrows)
    /// 3. Access pattern: pop() → node execution → schedule() → repeat
    /// Each step releases its mutable reference before the next begins.
    scheduler: UnsafeCell<Scheduler>,
    event_driver: EventDriver,
    edge_buffer: Vec<NodeIndex>,
    epoch: usize,
}

impl Executor {
    pub fn new() -> Self {
        Self {
            graph: Graph::new(),
            scheduler: UnsafeCell::new(Scheduler::new()),
            event_driver: EventDriver::new(),
            edge_buffer: Vec::with_capacity(BUFFER_CAPACITY),
            epoch: 0,
        }
    }

    pub const fn io_driver(&mut self) -> &mut IoDriver {
        self.event_driver.io_driver()
    }

    pub const fn timer_driver(&mut self) -> &mut TimerDriver {
        self.event_driver.timer_driver()
    }

    pub const fn yield_driver(&mut self) -> &mut YieldDriver {
        self.event_driver.yield_driver()
    }

    pub(crate) const fn graph(&mut self) -> &mut Graph {
        &mut self.graph
    }

    pub(crate) const fn scheduler(&mut self) -> &mut Scheduler {
        unsafe { &mut *self.scheduler.get() }
    }

    #[inline(always)]
    pub(crate) fn next_timer(&mut self) -> Option<Instant> {
        self.event_driver.timer_driver().next_timer()
    }

    pub fn cycle(&mut self, clock: &impl Clock, timeout: Option<Duration>) -> io::Result<()> {
        // Increment executor epoch
        self.epoch = self.epoch.wrapping_add(1);

        // Snap clock times
        let now = clock.now();
        let trigger_time = clock.trigger_time();

        // Poll for external events
        self.event_driver.poll(
            &mut self.graph,
            unsafe { &mut *self.scheduler.get() },
            timeout,
            now,
            self.epoch,
        )?;

        // Create execution context for the current epoch
        let mut ctx = ExecutionContext::new(
            &mut self.event_driver,
            &self.scheduler,
            now,
            trigger_time,
            self.epoch,
        );

        // Process nodes in the graph
        while let Some(node_idx) = unsafe { (&mut *self.scheduler.get()).pop() } {
            ctx.set_current(node_idx);
            if self.graph.mutate(&mut ctx, node_idx) {
                self.edge_buffer
                    .extend(self.graph.triggering_edges(node_idx));
                self.edge_buffer.drain(..).for_each(|child| {
                    self.graph
                        .can_schedule(child, self.epoch)
                        .map(|depth| unsafe {
                            (&mut *self.scheduler.get()).schedule(child, depth)
                        });
                })
            }
        }

        // TODO - can add in the garbage collector and node spawner to be run here
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::TestClock;
    use crate::graph::{NodeContext, Relationship};
    use crate::node::NodeBuilder;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    fn create_test_node_with_closure<F>(closure: F, depth: u32) -> NodeContext
    where
        F: FnMut(&mut ExecutionContext) -> bool + 'static,
    {
        NodeContext::new(Box::new(closure), depth)
    }

    #[test]
    fn test_executor_creation() {
        let executor = Executor::new();
        assert_eq!(executor.epoch, 0);
    }

    #[test]
    fn test_basic_cycle_execution() {
        let mut executor = Executor::new();
        let clock = TestClock::new();

        let call_count = Rc::new(Cell::new(0));

        // Create a simple node that increments a counter
        let node = NodeBuilder::new(call_count)
            .on_init(|executor, _, idx| {
                executor.yield_driver().yield_now(idx);
            })
            .build(&mut executor, move |data, ctx| {
                data.set(data.get() + 1);
                let current = ctx.current();
                ctx.yield_driver().yield_now(current);
                false
            });

        // Run a cycle
        executor.cycle(&clock, Some(Duration::ZERO)).unwrap();

        // Verify the node was called
        assert_eq!(node.borrow().get(), 1);
        assert_eq!(executor.epoch, 1);

        executor.cycle(&clock, Some(Duration::ZERO)).unwrap();

        // Verify the node was called a second time
        assert_eq!(node.borrow().get(), 2);
        assert_eq!(executor.epoch, 2);
    }

    #[test]
    fn test_triggering_relationship() {
        let mut executor = Executor::new();
        let clock = TestClock::new();

        let parent_calls = Rc::new(Cell::new(0));
        let child_calls = Rc::new(Cell::new(0));

        let parent_calls_clone = parent_calls.clone();
        let child_calls_clone = child_calls.clone();

        // Create parent node that mutates (returns true)
        let parent_ctx = create_test_node_with_closure(
            move |_ctx| {
                parent_calls_clone.set(parent_calls_clone.get() + 1);
                true // This should trigger child
            },
            0,
        );

        // Create child node
        let child_ctx = create_test_node_with_closure(
            move |_ctx| {
                child_calls_clone.set(child_calls_clone.get() + 1);
                false
            },
            1,
        );

        let parent_idx = executor.graph.add_node(parent_ctx);
        let child_idx = executor.graph.add_node(child_ctx);

        // Add trigger relationship
        executor
            .graph
            .add_edge(parent_idx, child_idx, Relationship::Trigger);

        // Schedule only the parent
        unsafe {
            (&mut *executor.scheduler.get()).schedule(parent_idx, 0);
        }

        // Run cycle
        executor.cycle(&clock, Some(Duration::ZERO)).unwrap();

        // Both parent and child should have been called
        assert_eq!(parent_calls.get(), 1);
        assert_eq!(child_calls.get(), 1);
    }

    #[test]
    fn test_observe_relationship_does_not_trigger() {
        let mut executor = Executor::new();
        let clock = TestClock::new();

        let parent_calls = Rc::new(Cell::new(0));
        let child_calls = Rc::new(Cell::new(0));

        let parent_calls_clone = parent_calls.clone();
        let child_calls_clone = child_calls.clone();

        // Create parent node that mutates
        let parent_ctx = create_test_node_with_closure(
            move |_ctx| {
                parent_calls_clone.set(parent_calls_clone.get() + 1);
                true // This would trigger if relationship was Trigger
            },
            0,
        );

        // Create a child node
        let child_ctx = create_test_node_with_closure(
            move |_ctx| {
                child_calls_clone.set(child_calls_clone.get() + 1);
                false
            },
            1,
        );

        let parent_idx = executor.graph.add_node(parent_ctx);
        let child_idx = executor.graph.add_node(child_ctx);

        // Add observe relationship (should NOT trigger)
        executor
            .graph
            .add_edge(parent_idx, child_idx, Relationship::Observe);

        // Schedule only the parent
        unsafe {
            (&mut *executor.scheduler.get()).schedule(parent_idx, 0);
        }

        // Run cycle
        executor.cycle(&clock, Some(Duration::ZERO)).unwrap();

        // Only parent should have been called, child should not
        assert_eq!(parent_calls.get(), 1);
        assert_eq!(child_calls.get(), 0);
    }

    #[test]
    fn test_io_event_handling() {
        let mut executor = Executor::new();
        let clock = TestClock::new();

        let node_calls = Rc::new(Cell::new(0));
        let node_calls_clone = node_calls.clone();

        // Create a node that will be triggered by I/O
        let node_ctx = create_test_node_with_closure(
            move |_ctx| {
                node_calls_clone.set(node_calls_clone.get() + 1);
                false
            },
            1,
        );

        let node_idx = executor.graph.add_node(node_ctx);

        // Register a notifier for the node
        let notifier = executor
            .event_driver
            .io_driver()
            .register_notifier(node_idx)
            .expect("Failed to register notifier");

        // Trigger the notification
        notifier.notify().expect("Failed to notify");

        // Run cycle - this should pick up the I/O event and schedule the node
        executor
            .cycle(&clock, Some(Duration::from_millis(10)))
            .unwrap();

        // Verify the node was called due to I/O event
        assert_eq!(node_calls.get(), 1);

        // Clean up
        executor
            .event_driver
            .io_driver()
            .deregister_notifier(notifier);
    }

    #[test]
    fn test_timer_event_handling() {
        let mut executor = Executor::new();
        let clock = TestClock::new();

        let node_calls = Rc::new(Cell::new(0));
        let node_calls_clone = node_calls.clone();

        // Create a node that will be triggered by timer
        let node_ctx = create_test_node_with_closure(
            move |_ctx| {
                node_calls_clone.set(node_calls_clone.get() + 1);
                false
            },
            1,
        );

        let node_idx = executor.graph.add_node(node_ctx);

        // Register a timer for immediate execution
        let timer_time = clock.now();
        let _timer_registration = executor
            .event_driver
            .timer_driver()
            .register_timer(node_idx, timer_time);

        // Run cycle - timer should have expired and scheduled the node
        executor
            .cycle(&clock, Some(Duration::from_millis(10)))
            .unwrap();

        // Verify the node was called due to timer event
        assert_eq!(node_calls.get(), 1);
    }

    #[test]
    fn test_timer_not_expired_yet() {
        let mut executor = Executor::new();
        let clock = TestClock::new();

        let node_calls = Rc::new(Cell::new(0));
        let node_calls_clone = node_calls.clone();

        // Create a node
        let node_ctx = create_test_node_with_closure(
            move |_ctx| {
                node_calls_clone.set(node_calls_clone.get() + 1);
                false
            },
            1,
        );

        let node_idx = executor.graph.add_node(node_ctx);

        // Register a timer for future execution
        let future_time = clock.now() + Duration::from_secs(10);
        let _timer_registration = executor
            .event_driver
            .timer_driver()
            .register_timer(node_idx, future_time);

        // Run cycle - timer should not have expired
        executor.cycle(&clock, Some(Duration::ZERO)).unwrap();

        // Node should not have been called
        assert_eq!(node_calls.get(), 0);
    }

    #[test]
    fn test_multiple_cycles_increment_epoch() {
        let mut executor = Executor::new();
        let clock = TestClock::new();

        assert_eq!(executor.epoch, 0);

        // Run first cycle
        executor.cycle(&clock, Some(Duration::ZERO)).unwrap();
        assert_eq!(executor.epoch, 1);

        // Run second cycle
        executor.cycle(&clock, Some(Duration::ZERO)).unwrap();
        assert_eq!(executor.epoch, 2);

        // Run third cycle
        executor.cycle(&clock, Some(Duration::ZERO)).unwrap();
        assert_eq!(executor.epoch, 3);
    }

    #[test]
    fn test_execution_context_access() {
        let mut executor = Executor::new();
        let clock = TestClock::new();

        let context_checks = Rc::new(RefCell::new(Vec::new()));
        let context_checks_clone = context_checks.clone();

        let node_ctx = create_test_node_with_closure(
            move |ctx| {
                let mut checks = context_checks_clone.borrow_mut();

                // Test various context methods
                checks.push(format!("current: {:?}", ctx.current()));
                checks.push(format!("now: {:?}", ctx.now()));
                checks.push(format!("trigger_time: {:?}", ctx.trigger_time()));

                // Test that we can access drivers
                let _io_driver = ctx.io_driver();
                let _timer_driver = ctx.timer_driver();
                let _yield_driver = ctx.yield_driver();

                checks.push("drivers_accessible: true".to_string());

                false
            },
            0,
        );

        let node_idx = executor.graph.add_node(node_ctx);

        // Schedule the node
        unsafe {
            (&mut *executor.scheduler.get()).schedule(node_idx, 0);
        }

        // Run cycle
        executor.cycle(&clock, Some(Duration::ZERO)).unwrap();

        let checks = context_checks.borrow();
        assert!(checks.len() > 0);
        assert!(
            checks
                .iter()
                .any(|s| s.contains("drivers_accessible: true"))
        );
    }

    #[test]
    fn test_chain_of_triggering_nodes() {
        let mut executor = Executor::new();
        let clock = TestClock::new();

        let call_order = Rc::new(RefCell::new(Vec::new()));

        // Create chain: node1 -> node2 -> node3
        let call_order_1 = call_order.clone();
        let node1_ctx = create_test_node_with_closure(
            move |_ctx| {
                call_order_1.borrow_mut().push(1);
                true // Trigger downstream
            },
            0,
        );

        let call_order_2 = call_order.clone();
        let node2_ctx = create_test_node_with_closure(
            move |_ctx| {
                call_order_2.borrow_mut().push(2);
                true // Trigger downstream
            },
            1,
        );

        let call_order_3 = call_order.clone();
        let node3_ctx = create_test_node_with_closure(
            move |_ctx| {
                call_order_3.borrow_mut().push(3);
                false // End of chain
            },
            2,
        );

        let node1_idx = executor.graph.add_node(node1_ctx);
        let node2_idx = executor.graph.add_node(node2_ctx);
        let node3_idx = executor.graph.add_node(node3_ctx);

        // Connect the chain
        executor
            .graph
            .add_edge(node1_idx, node2_idx, Relationship::Trigger);
        executor
            .graph
            .add_edge(node2_idx, node3_idx, Relationship::Trigger);

        // Start the chain by scheduling node1
        unsafe {
            (&mut *executor.scheduler.get()).schedule(node1_idx, 0);
        }

        // Run cycle
        executor.cycle(&clock, Some(Duration::ZERO)).unwrap();

        // All nodes should have been called in order
        let order = call_order.borrow();
        assert_eq!(*order, vec![1, 2, 3]);
    }

    #[test]
    fn test_next_timer_tracking() {
        let mut executor = Executor::new();
        let clock = TestClock::new();

        // Initially no timers
        assert_eq!(executor.next_timer(), None);

        let node_ctx = create_test_node_with_closure(|_| false, 1);
        let node_idx = executor.graph.add_node(node_ctx);

        // Register a timer
        let future_time = clock.now() + Duration::from_millis(500);
        let _registration = executor
            .event_driver
            .timer_driver()
            .register_timer(node_idx, future_time);

        // Should now return the timer time
        assert_eq!(executor.next_timer(), Some(future_time));
    }

    #[test]
    fn test_yield_driver_integration() {
        let mut executor = Executor::new();
        let clock = TestClock::new();

        let node_calls = Rc::new(Cell::new(0));
        let node_calls_clone = node_calls.clone();

        let node_ctx = create_test_node_with_closure(
            move |_ctx| {
                node_calls_clone.set(node_calls_clone.get() + 1);
                false
            },
            1,
        );

        let node_idx = executor.graph.add_node(node_ctx);

        // Use yield driver to schedule the node
        executor.event_driver.yield_driver().yield_now(node_idx);

        // Run cycle - yield driver should schedule the node
        executor.cycle(&clock, Some(Duration::ZERO)).unwrap();

        // Verify the node was called
        assert_eq!(node_calls.get(), 1);
    }
}

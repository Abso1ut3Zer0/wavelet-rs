use crate::Control;
use crate::runtime::clock::TriggerTime;
use crate::runtime::event_driver::YieldDriver;
use crate::runtime::event_driver::{EventDriver, IoDriver, IoSource, TimerDriver, TimerSource};
use crate::runtime::garbage_collector::GarbageCollector;
use crate::runtime::graph::Graph;
use crate::runtime::node::Node;
use crate::runtime::scheduler::{Scheduler, SchedulerError};
use enum_as_inner::EnumAsInner;
use mio::Interest;
use mio::event::Source;
use petgraph::graph::NodeIndex;
use std::cell::UnsafeCell;
use std::collections::VecDeque;
use std::io;
use std::time::{Duration, Instant};
use time::OffsetDateTime;

const BUFFER_CAPACITY: usize = 32;

type SpawnFn = Box<dyn FnOnce(&mut Executor) + 'static>;

/// Indicates whether the executor should continue running or terminate.
///
/// Returned by the main execution cycle to signal the runtime's desired state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumAsInner)]
pub enum ExecutorState {
    /// The executor should continue processing cycles
    Running,

    /// The executor should shut down gracefully (triggered by `Control::Terminate`)
    Terminated,
}

/// Execution context provided to nodes during their cycle function.
///
/// `ExecutionContext` is the primary interface that nodes use to interact
/// with the runtime during execution. It provides access to:
/// - **Event registration**: I/O sources, timers, and yield scheduling
/// - **Graph operations**: Scheduling other nodes and spawning subgraphs
/// - **Time information**: Current time snapshots for consistent timing
/// - **Mutation tracking**: Check if dependencies have changed this cycle
///
/// The context is only valid during the current graph cycle and borrows
/// from the main executor, ensuring safe access to runtime resources.
pub struct ExecutionContext<'a> {
    /// Access to all event drivers for registration
    event_driver: &'a mut EventDriver,

    /// Scheduler for immediate node scheduling
    scheduler: &'a UnsafeCell<Scheduler>,

    /// Queue for deferred subgraph creation
    deferred_spawns: &'a mut VecDeque<SpawnFn>,

    /// The currently executing node
    current: NodeIndex,

    /// Consistent time snapshot for this execution cycle
    time_snapshot: TriggerTime,

    /// Current execution epoch for change tracking
    epoch: usize,
}

impl<'a> ExecutionContext<'a> {
    /// Creates a new execution context (internal use only).
    pub(crate) fn new(
        event_driver: &'a mut EventDriver,
        scheduler: &'a UnsafeCell<Scheduler>,
        deferred_spawns: &'a mut VecDeque<SpawnFn>,
        time_snapshot: TriggerTime,
        epoch: usize,
    ) -> Self {
        Self {
            event_driver,
            scheduler,
            deferred_spawns,
            current: NodeIndex::new(0),
            time_snapshot,
            epoch,
        }
    }

    /// Registers an I/O source to receive events for the specified node.
    #[inline(always)]
    pub fn register_io<S: Source>(
        &mut self,
        source: S,
        idx: NodeIndex,
        interest: Interest,
    ) -> io::Result<IoSource<S>> {
        self.event_driver
            .io_driver()
            .register_source(source, idx, interest)
    }

    /// Removes an I/O source from event monitoring.
    #[inline(always)]
    pub fn deregister_io<S: Source>(&mut self, source: IoSource<S>) -> io::Result<NodeIndex> {
        self.event_driver.io_driver().deregister_source(source)
    }

    /// Changes the interest flags for an existing I/O source.
    #[inline(always)]
    pub fn reregister_io<S: Source>(
        &mut self,
        handle: &mut IoSource<S>,
        interest: Interest,
    ) -> io::Result<()> {
        self.event_driver
            .io_driver()
            .reregister_source(handle, interest)
    }

    /// Registers a timer to schedule a node at the specified time.
    #[inline(always)]
    pub fn register_timer(&mut self, node_index: NodeIndex, when: Instant) -> TimerSource {
        self.event_driver
            .timer_driver()
            .register_timer(node_index, when)
    }

    /// Cancels a previously registered timer.
    #[inline(always)]
    pub fn deregister_timer(&mut self, source: TimerSource) {
        self.event_driver.timer_driver().deregister_timer(source)
    }

    /// Schedules a node for execution in the current cycle.
    ///
    /// This is the framework's `yield_now()` equivalent for immediate scheduling.
    #[inline(always)]
    pub fn yield_now(&mut self, node_index: NodeIndex) {
        self.event_driver.yield_driver().yield_now(node_index)
    }

    /// Returns the index of the currently executing node.
    pub const fn current(&self) -> NodeIndex {
        self.current
    }

    /// Returns the monotonic time snapshot for this execution cycle.
    ///
    /// All nodes in the same cycle see the same time value for consistency.
    pub const fn now(&self) -> Instant {
        self.time_snapshot.instant
    }

    /// Returns the wall clock time snapshot for this execution cycle.
    ///
    /// All nodes in the same cycle see the same time value for consistency.
    pub const fn trigger_time(&self) -> OffsetDateTime {
        self.time_snapshot.system_time
    }

    /// Updates the currently executing node (internal use only).
    const fn set_current(&mut self, node_index: NodeIndex) {
        self.current = node_index;
    }

    /// Schedules another node for execution in the current cycle.
    ///
    /// Bypasses the normal dependency-based scheduling to manually trigger
    /// a specific node. Use sparingly as it can break incremental computation
    /// guarantees if not used carefully.
    ///
    /// Note: this call will panic if you are scheduling a node at a depth
    /// higher than the current depth in the graph.
    #[inline(always)]
    pub fn schedule_node<T>(&mut self, node: &Node<T>) -> Result<(), SchedulerError> {
        unsafe { (&mut *self.scheduler.get()).schedule(node.index(), node.depth()) }
    }

    /// Returns the epoch value associated with the current execution cycle.
    pub const fn epoch(&self) -> usize {
        self.epoch
    }

    /// Checks if a parent node has mutated in the current execution cycle.
    ///
    /// Used by nodes with `Observe` relationships to determine when their
    /// dependencies have changed and action may be needed.
    #[inline(always)]
    pub fn has_mutated<T>(&self, parent: Node<T>) -> bool {
        parent.mut_epoch() == self.epoch
    }

    /// Defers the creation of a subgraph until after the current cycle completes.
    ///
    /// The provided closure will be called with full executor access after all
    /// nodes in the current cycle have finished executing. This prevents
    /// graph modification during active execution to ensure the graph remains
    /// consistent across the current processing cycle (avoids any reentrancy
    /// related issues).
    #[inline(always)]
    pub fn spawn_subgraph<F>(&mut self, spawn_fn: F)
    where
        F: FnOnce(&mut Executor) + 'static,
    {
        self.deferred_spawns.push_back(Box::new(spawn_fn));
    }
}

/// The core execution engine that manages the computation graph and runtime state.
///
/// The `Executor` is the heart of the wavelet runtime, responsible for:
/// - **Graph management**: Storing nodes, relationships, and topology
/// - **Event coordination**: Integrating I/O, timers, and yield events
/// - **Scheduling**: Orchestrating dependency-ordered node execution
/// - **Lifecycle management**: Handling node creation, execution, and cleanup
/// - **Resource management**: Coordinating garbage collection and memory cleanup
///
/// # Architecture
/// The executor operates on a single-threaded, cooperative model where nodes
/// execute in dependency order and voluntarily yield control. This provides:
/// - **Deterministic execution**: Predictable ordering and timing
/// - **Zero-cost scheduling**: Direct function calls without async overhead
/// - **Controlled resource usage**: No hidden thread spawning or allocation
///
/// # Safety Notes
/// Uses `UnsafeCell<Scheduler>` for interior mutability in the single-threaded
/// context. The access pattern ensures safety through temporal separation:
/// `pop()` → node execution → `schedule()` → repeat, with no overlapping borrows.
pub struct Executor {
    /// The computation graph containing all nodes and relationships
    graph: Graph,

    /// Node scheduler with safe interior mutability
    ///
    /// SAFETY: Scheduler access through UnsafeCell is safe because:
    /// 1. Single-threaded execution only
    /// 2. All mutable accesses are temporally separated (no simultaneous borrows)
    /// 3. Access pattern: pop() → node execution → schedule() → repeat
    /// Each step releases its mutable reference before the next begins.
    scheduler: UnsafeCell<Scheduler>,

    /// Unified event management for I/O, timers, and yields
    event_driver: EventDriver,

    /// Reusable buffer for collecting triggering edges during broadcast
    edge_buffer: Vec<NodeIndex>,

    /// Queue of deferred subgraph creation functions
    deferred_spawns: VecDeque<SpawnFn>,

    /// Garbage collector for coordinating node cleanup
    gc: GarbageCollector,

    /// Current execution epoch for change tracking and deduplication
    epoch: usize,
}

impl Executor {
    pub fn new() -> Self {
        Self {
            graph: Graph::new(),
            scheduler: UnsafeCell::new(Scheduler::new()),
            event_driver: EventDriver::new(),
            edge_buffer: Vec::with_capacity(BUFFER_CAPACITY),
            deferred_spawns: VecDeque::new(),
            gc: GarbageCollector::new(),
            epoch: 0,
        }
    }

    /// Provides another way to check if a node has mutated.
    ///
    /// This is typically used for testing scenarios.
    #[inline(always)]
    pub fn has_mutated<T>(&self, node: &Node<T>) -> bool {
        node.mut_epoch() == self.epoch
    }

    /// Provides access to the I/O event driver for source registration.
    ///
    /// Use this to register network sockets, file handles, or other I/O sources
    /// that should trigger node execution when ready. fm
    pub const fn io_driver(&mut self) -> &mut IoDriver {
        self.event_driver.io_driver()
    }

    /// Provides access to the timer driver for time-based scheduling.
    ///
    /// Use this to register timers that will schedule nodes at specific times
    /// or after delays.
    pub const fn timer_driver(&mut self) -> &mut TimerDriver {
        self.event_driver.timer_driver()
    }

    /// Provides access to the yield driver for immediate scheduling.
    ///
    /// Use this to schedule nodes for execution in the current cycle, typically
    /// during node initialization or for self-triggering patterns.
    pub const fn yield_driver(&mut self) -> &mut YieldDriver {
        self.event_driver.yield_driver()
    }

    /// Provides internal access to the computation graph (runtime internal only).
    pub(crate) const fn graph(&mut self) -> &mut Graph {
        &mut self.graph
    }

    /// Provides internal access to the scheduler (runtime internal only).
    ///
    /// # Safety
    /// Accesses the scheduler through UnsafeCell. Safe due to single-threaded
    /// execution and non-overlapping access patterns in the execution loop.
    pub(crate) const fn scheduler(&mut self) -> &mut Scheduler {
        unsafe { &mut *self.scheduler.get() }
    }

    /// Returns a cloneable handle to the garbage collector.
    ///
    /// Used by nodes to register themselves for cleanup when dropped.
    /// The garbage collector coordinates deferred removal after cycle completion.
    pub(crate) fn garbage_collector(&mut self) -> GarbageCollector {
        self.gc.clone()
    }

    /// Returns when the next timer will expire, if any.
    ///
    /// Used by the runtime to optimize polling timeouts - if there's a timer
    /// expiring soon, polling can be shortened to wake up at the right time.
    #[inline(always)]
    pub(crate) fn next_timer(&mut self) -> Option<Instant> {
        self.event_driver.timer_driver().next_timer()
    }

    /// Executes a single cycle of the computation graph.
    ///
    /// This is the core execution method that orchestrates one complete cycle
    /// of event processing, node execution, and cleanup. Each cycle follows
    /// a precise sequence to ensure deterministic behavior and correct
    /// dependency ordering.
    ///
    /// # Execution Flow
    ///
    /// ## 1. Epoch Management
    /// ```text
    /// epoch = epoch.wrapping_add(1)
    /// ```
    /// Increments the global epoch counter used for:
    /// - Change tracking (`has_mutated()` queries)
    /// - Scheduling deduplication (prevents double-scheduling nodes)
    /// - Mutation epoch stamping when nodes execute
    ///
    /// ## 2. Event Polling
    /// ```text
    /// event_driver.poll() → schedules ready nodes
    /// ```
    /// Processes all event sources in priority order:
    /// - **Yield events**: Immediate scheduling requests (highest priority)
    /// - **Timer events**: Expired timers based on current time
    /// - **I/O events**: Ready network/file sources (uses timeout parameter)
    ///
    /// ## 3. Node Execution Loop
    /// ```text
    /// while scheduler.pop() → Some(node_idx):
    ///     ctx.set_current(node_idx)
    ///     result = node.cycle(ctx)
    ///     handle_control_result(result)
    /// ```
    ///
    /// Executes nodes in dependency order (depth-first) until scheduler is empty:
    /// - **Update context**: Set current node for context queries
    /// - **Execute node**: Call the node's cycle function with mutable data access
    /// - **Process result**: Handle the returned `Control` value:
    ///
    /// ### Control Flow Handling
    /// - **`Control::Broadcast`**:
    ///   ```text
    ///   collect triggering_edges(node) → edge_buffer
    ///   for each child: try schedule(child, depth)
    ///   ```
    ///   Node has mutated - schedule all children with `Trigger` relationships
    ///
    /// - **`Control::Unchanged`**:
    ///   Node processed but didn't change - no downstream scheduling
    ///
    /// - **`Control::Sweep`**:
    ///   ```text
    ///   gc.mark_for_sweep(node_idx)
    ///   ```
    ///   Node requests removal - mark for cleanup after cycle
    ///
    /// - **`Control::Terminate`**:
    ///   ```text
    ///   return ExecutorState::Terminated
    ///   ```
    ///   Graceful shutdown requested - exit immediately
    ///
    /// ## 4. Cleanup Phase
    /// ```text
    /// while gc.next_to_sweep() → Some(node):
    ///     graph.remove_node(node)
    /// ```
    /// Remove all nodes marked for garbage collection during execution.
    /// Safe to modify the graph structure after all nodes have finished executing.
    ///
    /// ## 5. Deferred Operations
    /// ```text
    /// while deferred_spawns.pop_front() → Some(spawn_fn):
    ///     spawn_fn(executor)
    /// ```
    /// Execute any subgraph creation functions queued during node execution.
    /// This allows dynamic graph modification without disrupting the current cycle.
    ///
    /// # Parameters
    /// - `time_snapshot`: Consistent time values for all nodes in this cycle
    /// - `timeout`: Maximum time to wait for I/O events (None = no timeout)
    ///
    /// # Returns
    /// - `Ok(ExecutorState::Running)`: Cycle completed normally, ready for next cycle
    /// - `Ok(ExecutorState::Terminated)`: Graceful shutdown requested by a node
    /// - `Err(io::Error)`: I/O polling failed
    pub fn cycle(
        &mut self,
        time_snapshot: TriggerTime,
        timeout: Option<Duration>,
    ) -> io::Result<ExecutorState> {
        // Increment executor epoch
        self.epoch = self.epoch.wrapping_add(1);

        // Poll for external events
        self.event_driver.poll(
            &mut self.graph,
            unsafe { &mut *self.scheduler.get() },
            timeout,
            time_snapshot.instant,
            self.epoch,
        )?;

        // Create execution context for the current epoch
        let mut ctx = ExecutionContext::new(
            &mut self.event_driver,
            &self.scheduler,
            &mut self.deferred_spawns,
            time_snapshot,
            self.epoch,
        );

        // Process nodes in the graph
        while let Some(node_idx) = unsafe { (&mut *self.scheduler.get()).pop() } {
            ctx.set_current(node_idx);
            match self.graph.cycle(&mut ctx, node_idx) {
                Control::Broadcast => {
                    // Collect all triggering edges for the current node,
                    // then schedule the children for execution.
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
                Control::Unchanged => {
                    // do nothing
                }
                Control::Sweep => {
                    // Mark the current node for cleanup, poison all children, then schedule
                    // them to be cleaned up when they are processed later in the cycle.
                    self.gc.mark_for_sweep(node_idx);
                    self.edge_buffer.extend(self.graph.edges(node_idx));
                    self.edge_buffer.drain(..).for_each(|child| {
                        self.graph.mark_poisoned(child);
                        self.graph
                            .can_schedule(child, self.epoch)
                            .map(|depth| unsafe {
                                let _ = (&mut *self.scheduler.get()).schedule(child, depth);
                            });
                    });
                }
                Control::Terminate => {
                    return Ok(ExecutorState::Terminated); // inform the runtime that we should exit
                }
            }
        }

        // Sweep all nodes marked for cleanup
        while let Some(marked_node) = self.gc.next_to_sweep() {
            self.graph.remove_node(marked_node);
        }

        // Execute any deferred subgraph creation functions
        while let Some(spawn_fn) = self.deferred_spawns.pop_front() {
            spawn_fn(self)
        }
        Ok(ExecutorState::Running)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Relationship;
    use crate::runtime::clock::{Clock, TestClock};
    use crate::runtime::node::NodeBuilder;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    #[test]
    fn test_executor_creation() {
        let executor = Executor::new();
        assert_eq!(executor.epoch, 0);
    }

    #[test]
    fn test_basic_cycle_execution() {
        let mut executor = Executor::new();
        let mut clock = TestClock::new();

        // Create a simple node that increments a counter
        let node = NodeBuilder::new(0)
            .on_init(|executor, _, idx| {
                executor.yield_driver().yield_now(idx);
            })
            .build(&mut executor, |data, ctx| {
                *data += 1;
                let current = ctx.current();
                ctx.yield_now(current);
                Control::Unchanged
            });

        // Run a cycle
        let now = clock.trigger_time();
        executor.cycle(now, Some(Duration::ZERO)).unwrap();

        // Verify the node was called
        assert_eq!(*node.borrow(), 1);
        assert_eq!(executor.epoch, 1);

        let now = clock.trigger_time();
        executor.cycle(now, Some(Duration::ZERO)).unwrap();

        // Verify the node was called a second time
        assert_eq!(*node.borrow(), 2);
        assert_eq!(executor.epoch, 2);
    }

    #[test]
    fn test_triggering_relationship() {
        let mut executor = Executor::new();
        let mut clock = TestClock::new();

        // Create parent node that mutates (returns true)
        let parent_node = NodeBuilder::new(0)
            .on_init(|executor, _, idx| {
                // Schedule the parent to start the chain
                executor.yield_driver().yield_now(idx);
            })
            .build(&mut executor, |data, _ctx| {
                *data += 1;
                Control::Broadcast // This should trigger the child
            });

        // Create a child node that depends on parent
        let child_node = NodeBuilder::new(0)
            .add_relationship(&parent_node, Relationship::Trigger)
            .build(&mut executor, |data, _ctx| {
                *data += 1;
                Control::Unchanged
            });

        // Run cycle
        let now = clock.trigger_time();
        executor.cycle(now, Some(Duration::ZERO)).unwrap();

        // Both parent and child should have been called
        assert_eq!(*parent_node.borrow(), 1);
        assert_eq!(*child_node.borrow(), 1);
    }

    #[test]
    fn test_observe_relationship_does_not_trigger() {
        let mut executor = Executor::new();
        let mut clock = TestClock::new();

        // Create parent node that mutates
        let parent_node = NodeBuilder::new(0)
            .on_init(|executor, _, idx| {
                // Schedule the parent to start
                executor.yield_driver().yield_now(idx);
            })
            .build(&mut executor, |data, _ctx| {
                *data += 1;
                Control::Broadcast // This would trigger if the relationship was Trigger
            });

        // Create a child node with Observe relationship
        let child_node = NodeBuilder::new(0)
            .add_relationship(&parent_node, Relationship::Observe)
            .build(&mut executor, |data, _ctx| {
                *data += 1;
                Control::Unchanged
            });

        // Run cycle
        let now = clock.trigger_time();
        executor.cycle(now, Some(Duration::ZERO)).unwrap();

        // Only parent should have been called, child should not
        assert_eq!(*parent_node.borrow(), 1);
        assert_eq!(*child_node.borrow(), 0); // Should not be triggered
    }

    #[test]
    fn test_io_event_handling() {
        let mut executor = Executor::new();
        let mut clock = TestClock::new();

        // Create a node that will be triggered by I/O
        let (node, notifier) = NodeBuilder::new(0)
            .build_with_notifier(&mut executor, |data, _ctx| {
                *data += 1;
                Control::Unchanged
            })
            .unwrap();

        notifier.notify().unwrap();

        // Run cycle - this should pick up the I/O event and schedule the node
        let now = clock.trigger_time();
        executor
            .cycle(now, Some(Duration::from_millis(10)))
            .unwrap();

        // Verify the node was called due to an I/O event
        assert_eq!(*node.borrow(), 1);
    }

    #[test]
    fn test_timer_event_handling() {
        let mut executor = Executor::new();
        let mut clock = TestClock::new();

        // Create a timer node that sets up its own recurring timer
        let node = NodeBuilder::new(0)
            .on_init(|executor, _, idx| {
                // Just kick-start the node - let it register its own timer
                executor.yield_driver().yield_now(idx);
            })
            .build(&mut executor, |data, ctx| {
                *data += 1;

                // On the first run, register a timer for the next execution
                if *data == 1 {
                    let timer_time = ctx.now() + Duration::from_millis(100);
                    let _timer_reg = ctx.register_timer(ctx.current(), timer_time);
                    // In real scenarios, you'd store timer_reg in node data for cleanup
                }

                Control::Unchanged
            });

        // First cycle - node runs via yield, registers timer
        let now = clock.trigger_time();
        executor.cycle(now, Some(Duration::ZERO)).unwrap();
        assert_eq!(*node.borrow(), 1);

        // Advance clock and run again - timer should fire
        clock.advance(Duration::from_millis(150));
        let now = clock.trigger_time();
        executor.cycle(now, Some(Duration::ZERO)).unwrap();
        assert_eq!(*node.borrow(), 2);
    }

    #[test]
    fn test_timer_not_expired_yet() {
        let mut executor = Executor::new();
        let mut clock = TestClock::new();

        // Create a node with the future timer
        let node = NodeBuilder::new(0)
            .on_init(|executor, _, idx| {
                // Just kick-start the node - let it register its own timer
                executor.yield_driver().yield_now(idx);
            })
            .build(&mut executor, |data, ctx| {
                *data += 1;
                let next_time = ctx.now() + Duration::from_secs(3);
                ctx.register_timer(ctx.current(), next_time);
                Control::Unchanged
            });

        // Run cycle - timer should not have expired
        let now = clock.trigger_time();
        executor.cycle(now, Some(Duration::ZERO)).unwrap();

        // Node should have been called on the first cycle
        assert_eq!(*node.borrow(), 1);

        let now = clock.trigger_time();
        executor.cycle(now, Some(Duration::ZERO)).unwrap();
        // Node is not called again, timer not expired
        assert_eq!(*node.borrow(), 1);
    }

    #[test]
    fn test_multiple_cycles_increment_epoch() {
        let mut executor = Executor::new();
        let mut clock = TestClock::new();

        assert_eq!(executor.epoch, 0);

        // Run the first cycle
        let now = clock.trigger_time();
        executor.cycle(now, Some(Duration::ZERO)).unwrap();
        assert_eq!(executor.epoch, 1);

        // Run the second cycle
        let now = clock.trigger_time();
        executor.cycle(now, Some(Duration::ZERO)).unwrap();
        assert_eq!(executor.epoch, 2);

        // Run the third cycle
        let now = clock.trigger_time();
        executor.cycle(now, Some(Duration::ZERO)).unwrap();
        assert_eq!(executor.epoch, 3);
    }

    #[test]
    fn test_chain_of_triggering_nodes() {
        let mut executor = Executor::new();
        let mut clock = TestClock::new();

        let call_order = Rc::new(RefCell::new(Vec::new()));

        // Create chain: node1 -> node2 -> node3
        let call_order_1 = call_order.clone();
        let node1 = NodeBuilder::new(0)
            .on_init(|executor, _, idx| {
                // Start the chain
                executor.yield_driver().yield_now(idx);
            })
            .build(&mut executor, move |data, _ctx| {
                *data += 1;
                call_order_1.borrow_mut().push(1);
                Control::Broadcast // Trigger downstream
            });

        let call_order_2 = call_order.clone();
        let node2 = NodeBuilder::new(0)
            .add_relationship(&node1, Relationship::Trigger)
            .build(&mut executor, move |data, _ctx| {
                *data += 1;
                call_order_2.borrow_mut().push(2);
                Control::Broadcast // Trigger downstream
            });

        let call_order_3 = call_order.clone();
        let _node3 = NodeBuilder::new(0)
            .add_relationship(&node2, Relationship::Trigger)
            .build(&mut executor, move |data, _ctx| {
                *data += 1;
                call_order_3.borrow_mut().push(3);
                Control::Unchanged // End of the chain
            });

        // Run cycle
        let now = clock.trigger_time();
        executor.cycle(now, Some(Duration::ZERO)).unwrap();

        // All nodes should have been called in order
        let order = call_order.borrow();
        assert_eq!(*order, vec![1, 2, 3]);
    }

    #[test]
    fn test_next_timer_tracking() {
        let mut executor = Executor::new();
        let mut clock = TestClock::new();

        // Initially no timers
        assert_eq!(executor.next_timer(), None);

        let _node = NodeBuilder::new(0)
            .on_init(|executor, _, idx| {
                // Force trigger the node
                executor.yield_driver().yield_now(idx);
            })
            .build(&mut executor, |data, ctx| {
                *data += 1;
                let future_time = ctx.now() + Duration::from_millis(500);
                ctx.register_timer(ctx.current(), future_time);
                Control::Unchanged
            });

        // Don't have a registered timer yet
        assert_eq!(executor.next_timer(), None);

        let now = clock.trigger_time();
        executor.cycle(now, Some(Duration::ZERO)).unwrap();

        // Should now return the timer time
        let _expected_time = clock.trigger_time().instant + Duration::from_millis(500);
    }

    #[test]
    fn test_yield_driver_integration() {
        let mut executor = Executor::new();
        let mut clock = TestClock::new();

        let node = NodeBuilder::new(0)
            .on_init(|executor, _, idx| {
                // Use yield driver to schedule the node during init
                executor.yield_driver().yield_now(idx);
            })
            .build(&mut executor, |data, _ctx| {
                *data += 1;
                Control::Unchanged
            });

        // Run cycle - the yield driver should schedule the node
        let now = clock.trigger_time();
        executor.cycle(now, Some(Duration::ZERO)).unwrap();

        // Verify the node was called
        assert_eq!(*node.borrow(), 1);
    }

    #[test]
    fn test_termination_state() {
        let mut executor = Executor::new();
        let mut clock = TestClock::new();

        let _node = NodeBuilder::new(0)
            .on_init(|executor, _, idx| {
                executor.yield_driver().yield_now(idx);
            })
            .build(&mut executor, |_, _ctx| Control::Terminate);

        let now = clock.trigger_time();
        let result = executor.cycle(now, Some(Duration::ZERO));
        assert!(result.is_ok());

        let state = result.unwrap();
        assert!(state.is_terminated());
    }

    #[test]
    fn test_on_drop() {
        let mut executor = Executor::new();
        let flag = Rc::new(Cell::new(false));
        let node = NodeBuilder::new(flag.clone())
            .on_drop(|data| {
                println!("setting flag to true");
                data.set(true);
                println!("flag is now {}", data.get());
            })
            .build(&mut executor, |_, _| Control::Unchanged);

        let idx = node.index();
        drop(node);
        executor.graph.remove_node(idx);

        println!("flag is {} after drop", flag.get());
        assert!(flag.get());
    }

    #[test]
    fn test_on_drop_executor_exit() {
        let mut executor = Executor::new();
        let flag = Rc::new(Cell::new(false));
        let node = NodeBuilder::new(flag.clone())
            .on_drop(|data| {
                println!("setting flag to true");
                data.set(true);
                println!("flag is now {}", data.get());
            })
            .build(&mut executor, |_, _| Control::Unchanged);

        drop(node);
        drop(executor);

        println!("flag is {} after drop", flag.get());
        assert!(flag.get());
    }

    #[test]
    fn test_garbage_collection() {
        let mut executor = Executor::new();
        let mut clock = TestClock::new();

        let gc_count = Rc::new(Cell::new(0));
        let node1 = NodeBuilder::new(gc_count.clone())
            .on_init(|executor, _, idx| {
                executor.yield_driver().yield_now(idx);
            })
            .on_drop(|data| {
                println!("removing node1");
                data.update(|count| count + 1);
            })
            .build(&mut executor, |_, _| Control::Broadcast);

        let node2 = NodeBuilder::new(gc_count.clone())
            .triggered_by(&node1)
            .on_drop(|data| {
                println!("removing node2");
                data.update(|count| count + 1);
            })
            .build(&mut executor, move |_, _| {
                println!("node1 data: {}", node1.borrow().get());
                Control::Broadcast
            });

        NodeBuilder::new(gc_count.clone())
            .triggered_by(&node2)
            .on_drop(|data| {
                println!("removing node3");
                data.update(|count| count + 1);
            })
            .spawn(&mut executor, move |_, _| {
                println!("node2 data: {}", node2.borrow().get());
                Control::Sweep
            });

        assert_eq!(executor.graph.node_count(), 3);
        executor
            .cycle(clock.trigger_time(), Some(Duration::ZERO))
            .unwrap();
        assert_eq!(executor.graph.node_count(), 0);
    }

    #[test]
    fn test_node_spawn_with_cleanup() {
        let mut executor = Executor::new();
        let mut clock = TestClock::new();

        let spawned = Rc::new(Cell::new(false));
        let _root = NodeBuilder::new(spawned.clone())
            .on_init(|executor, _, idx| {
                executor.yield_driver().yield_now(idx);
            })
            .on_drop(|_| {
                println!("removing root");
            })
            .build(&mut executor, |spawned, ctx| {
                let flag = spawned.clone();
                ctx.spawn_subgraph(move |ex| {
                    NodeBuilder::new(flag)
                        .on_init(|executor, _, idx| {
                            executor.yield_driver().yield_now(idx);
                        })
                        .spawn(ex, |data, _| {
                            data.set(true);
                            Control::Sweep
                        })
                });
                Control::Broadcast
            });

        assert_eq!(executor.graph.node_count(), 1);
        executor
            .cycle(clock.trigger_time(), Some(Duration::ZERO))
            .unwrap();
        assert_eq!(executor.graph.node_count(), 2); // root + spawned
        assert!(!spawned.get()); // spawned not yet called

        executor
            .cycle(clock.trigger_time(), Some(Duration::ZERO))
            .unwrap();
        assert_eq!(executor.graph.node_count(), 1); // root, since spawned gets swept
        assert!(spawned.get()); // spawned now called
    }

    #[test]
    fn test_node_spawn_with_cleanup_on_panic() {
        let mut executor = Executor::new();
        let mut clock = TestClock::new();

        let spawned = Rc::new(Cell::new(false));
        let _root = NodeBuilder::new(spawned.clone())
            .on_init(|executor, _, idx| {
                executor.yield_driver().yield_now(idx);
            })
            .on_drop(|_| {
                println!("removing root");
            })
            .build(&mut executor, |spawned, ctx| {
                let flag = spawned.clone();
                ctx.spawn_subgraph(move |ex| {
                    NodeBuilder::new(flag)
                        .on_init(|executor, _, idx| {
                            executor.yield_driver().yield_now(idx);
                        })
                        .spawn(ex, |data, _| {
                            data.set(true);
                            panic!("panic!");
                            #[allow(unreachable_code)]
                            Control::Unchanged
                        })
                });
                Control::Broadcast
            });

        assert_eq!(executor.graph.node_count(), 1);
        executor
            .cycle(clock.trigger_time(), Some(Duration::ZERO))
            .unwrap();
        assert_eq!(executor.graph.node_count(), 2); // root + spawned
        assert!(!spawned.get()); // spawned not yet called

        executor
            .cycle(clock.trigger_time(), Some(Duration::ZERO))
            .unwrap();
        assert_eq!(executor.graph.node_count(), 1); // root, since spawned gets swept
        assert!(spawned.get()); // spawned now called
    }

    #[test]
    fn test_garbage_collection_on_poisoned() {
        let mut executor = Executor::new();
        let mut clock = TestClock::new();

        let gc_count = Rc::new(Cell::new(0));
        let node0 = NodeBuilder::new(0)
            .on_init(|ex, _, idx| {
                ex.yield_driver().yield_now(idx);
            })
            .build(&mut executor, |_, _| Control::Broadcast);

        let node1 = NodeBuilder::new(gc_count.clone())
            .on_init(|executor, _, idx| {
                executor.yield_driver().yield_now(idx);
            })
            .on_drop(|data| {
                println!("removing node1");
                data.update(|count| count + 1);
            })
            .build(&mut executor, |_, _| {
                // panic ahead of downstream child nodes,
                // which should trigger garbage collection
                // for the entire subgraph
                panic!("panic!");
                #[allow(unreachable_code)]
                Control::Broadcast
            });

        let node2 = NodeBuilder::new(gc_count.clone())
            .triggered_by(&node0)
            .observer_of(&node1)
            .on_drop(|data| {
                println!("removing node2");
                data.update(|count| count + 1);
            })
            .build(&mut executor, move |_, _| {
                println!("node1 data: {}", node1.borrow().get());
                Control::Broadcast
            });

        NodeBuilder::new(gc_count.clone())
            .observer_of(&node2)
            .on_drop(|data| {
                println!("removing node3");
                data.update(|count| count + 1);
            })
            .spawn(&mut executor, move |_, _| {
                println!("node2 data: {}", node2.borrow().get());
                Control::Broadcast
            });

        assert_eq!(executor.graph.node_count(), 4);
        executor
            .cycle(clock.trigger_time(), Some(Duration::ZERO))
            .unwrap();
        assert_eq!(executor.graph.node_count(), 1);
    }
}

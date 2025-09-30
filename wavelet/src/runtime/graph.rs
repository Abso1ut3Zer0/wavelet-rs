use crate::runtime::executor::ExecutionContext;
use crate::{Control, Relationship};
use petgraph::prelude::{EdgeRef, NodeIndex};
use petgraph::stable_graph::StableGraph;

pub(crate) type CycleFn = Box<dyn FnMut(&mut ExecutionContext) -> Control + 'static>;

/// Runtime context for a node within the computation graph.
///
/// Stores the node's execution function, scheduling metadata, and depth
/// information. This is the internal representation used by the graph
/// to manage node execution.
pub(crate) struct NodeContext {
    /// The node's computation logic
    pub(crate) cycle_fn: CycleFn,

    /// Last epoch this node was scheduled (prevents duplicate scheduling)
    pub(crate) sched_epoch: usize,

    /// Node's depth level in the dependency graph
    pub(crate) depth: u32,

    /// Indicates whether the node has been poisoned due to a panic
    /// or an upstream node being garbage collected.
    pub(crate) poisoned: bool,
}

impl NodeContext {
    /// Creates a new node context with the given cycle function and depth.
    ///
    /// The scheduling epoch starts at 0 and will be updated by the runtime.
    pub(crate) const fn new(cycle_fn: CycleFn, depth: u32) -> Self {
        Self {
            cycle_fn,
            sched_epoch: 0,
            depth,
            poisoned: false,
        }
    }

    /// Executes the node's cycle function with the provided execution context.
    #[inline(always)]
    fn cycle(&mut self, ctx: &mut ExecutionContext) -> Control {
        // Return early if the node is poisoned, so we
        // ensure the node gets cleaned up.
        if self.poisoned {
            return Control::Sweep;
        }

        (self.cycle_fn)(ctx)
    }
}

/// The computation graph that manages node relationships and execution.
///
/// Uses `petgraph::StableGraph` as the backing store, which provides:
/// - **Stable indices**: `NodeIndex` values remain valid even after other nodes are removed
/// - **Efficient removal**: Removed nodes leave gaps that can be reused for new nodes
/// - **O(1) access**: Direct indexing into node and edge data
///
/// The stable indices are crucial for the runtime's design - nodes can safely
/// hold references to other nodes via `NodeIndex` without worrying about
/// invalidation during garbage collection.
pub struct Graph {
    /// Backing graph storage with stable node indices
    inner: StableGraph<NodeContext, Relationship>,
}

impl Graph {
    pub(crate) fn new() -> Self {
        Self {
            inner: StableGraph::new(),
        }
    }

    /// Checks if a node can be scheduled and marks it for the current epoch.
    ///
    /// Returns `Some(depth)` if the node hasn't been scheduled this epoch,
    /// or `None` if it's already been scheduled. This prevents duplicate
    /// scheduling within a single execution cycle.
    ///
    /// Note: we safely check here if the node exists to avoid any data
    /// races with background threads informing the graph of an update
    /// to a node that was garbage collected.
    #[inline(always)]
    pub(crate) fn can_schedule(&mut self, node_index: NodeIndex, epoch: usize) -> Option<u32> {
        if let Some(ctx) = self.inner.node_weight_mut(node_index) {
            if ctx.sched_epoch == epoch {
                return None;
            }

            ctx.sched_epoch = epoch;
            return Some(ctx.depth);
        }

        None
    }

    /// Returns an iterator over child nodes with `Trigger` relationships.
    ///
    /// Used during broadcast propagation to find which nodes should be
    /// scheduled when the current node mutates.
    #[inline(always)]
    pub(crate) fn triggering_edges(
        &self,
        node_index: NodeIndex,
    ) -> impl Iterator<Item = NodeIndex> {
        self.inner
            .edges_directed(node_index, petgraph::Direction::Outgoing)
            .filter(|edge| edge.weight().is_trigger())
            .map(|edge| edge.target())
    }

    /// Returns an iterator over all child nodes.
    ///
    /// Used during broadcast propagation, particularly when we need to clean
    /// up child relationships in the event of a `Control::Sweep`.
    #[inline(always)]
    pub(crate) fn edges(&self, node_index: NodeIndex) -> impl Iterator<Item = NodeIndex> {
        self.inner
            .edges_directed(node_index, petgraph::Direction::Outgoing)
            .map(|edge| edge.target())
    }

    /// Executes the cycle function for the specified node.
    #[inline(always)]
    pub(crate) fn cycle(&mut self, ctx: &mut ExecutionContext, node_index: NodeIndex) -> Control {
        self.inner[node_index].cycle(ctx)
    }

    /// Mark a node as poisoned.
    #[inline(always)]
    pub(crate) fn mark_poisoned(&mut self, node_index: NodeIndex) {
        self.inner[node_index].poisoned = true;
    }

    /// Adds a new node to the graph and returns its stable index.
    #[inline(always)]
    pub(crate) fn add_node(&mut self, weight: NodeContext) -> NodeIndex {
        self.inner.add_node(weight)
    }

    /// Creates a directed edge between two nodes with the specified relationship.
    #[inline(always)]
    pub(crate) fn add_edge(
        &mut self,
        parent: NodeIndex,
        child: NodeIndex,
        relationship: Relationship,
    ) {
        self.inner.add_edge(parent, child, relationship);
    }

    /// Removes a node from the graph, leaving a reusable gap.
    ///
    /// The node's index becomes available for reuse by future `add_node` calls.
    /// All edges connected to this node are automatically removed.
    #[inline(always)]
    pub(crate) fn remove_node(&mut self, node_index: NodeIndex) {
        self.inner.remove_node(node_index);
    }

    /// Returns the current number of nodes in the graph.
    #[allow(dead_code)]
    pub(crate) fn node_count(&self) -> usize {
        self.inner.node_count()
    }

    #[allow(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        self.node_count() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::clock::CycleTime;
    use crate::runtime::event_driver::EventDriver;
    use crate::runtime::executor::ExecutionContext;
    use crate::runtime::scheduler::Scheduler;
    use std::cell::{Cell, UnsafeCell};
    use std::collections::VecDeque;
    use std::rc::Rc;
    use std::time::Instant;
    use time::OffsetDateTime;

    fn create_test_node(call_count: Rc<Cell<i32>>, should_mutate: bool) -> NodeContext {
        NodeContext::new(
            Box::new(move |mut _ctx: &mut ExecutionContext| {
                call_count.set(call_count.get() + 1);
                if should_mutate {
                    Control::Broadcast
                } else {
                    Control::Unchanged
                }
            }),
            1, // depth
        )
    }

    #[test]
    fn test_add_node_and_basic_structure() {
        let mut graph = Graph::new();

        let call_count = Rc::new(Cell::new(0));
        let node = create_test_node(call_count, true);

        let node_id = graph.add_node(node);
        assert_eq!(node_id.index(), 0); // First node should get index 0
    }

    #[test]
    fn test_add_edge() {
        let mut graph = Graph::new();

        let parent_calls = Rc::new(Cell::new(0));
        let child_calls = Rc::new(Cell::new(0));

        let parent = graph.add_node(create_test_node(parent_calls, true));
        let child = graph.add_node(create_test_node(child_calls, false));

        // Add edge between them
        graph.add_edge(parent, child, Relationship::Trigger);

        // Should be able to find the edge
        let children: Vec<_> = graph.triggering_edges(parent).collect();
        assert_eq!(children, vec![child]);
    }

    #[test]
    fn test_triggering_edges_filters_correctly() {
        let mut graph = Graph::new();

        let call_count = Rc::new(Cell::new(0));
        let parent = graph.add_node(create_test_node(call_count.clone(), true));
        let trigger_child = graph.add_node(create_test_node(call_count.clone(), false));
        let observe_child = graph.add_node(create_test_node(call_count, false));

        graph.add_edge(parent, trigger_child, Relationship::Trigger);
        graph.add_edge(parent, observe_child, Relationship::Observe);

        let triggering_children: Vec<_> = graph.triggering_edges(parent).collect();

        // Should only return the trigger child, not the observe child
        assert_eq!(triggering_children.len(), 1);
        assert_eq!(triggering_children[0], trigger_child);
    }

    #[test]
    fn test_can_schedule_epoch_deduplication() {
        let mut graph = Graph::new();
        let call_count = Rc::new(Cell::new(0));
        let node = graph.add_node(create_test_node(call_count, true));

        // First call with epoch 1 should return depth
        let result1 = graph.can_schedule(node, 1);
        assert_eq!(result1, Some(1)); // depth is 1

        // Second call with same epoch should return None (already scheduled)
        let result2 = graph.can_schedule(node, 1);
        assert_eq!(result2, None);

        // Call with different epoch should work again
        let result3 = graph.can_schedule(node, 2);
        assert_eq!(result3, Some(1));
    }

    #[test]
    fn test_mutate_calls_node_function() {
        let mut graph = Graph::new();
        let call_count = Rc::new(Cell::new(0));
        let should_mutate = true;

        let node = graph.add_node(create_test_node(call_count.clone(), should_mutate));

        // Create real components for ExecutionContext
        let mut event_driver = EventDriver::new();
        let scheduler = UnsafeCell::new(Scheduler::new());
        let mut deferred_spawns = VecDeque::new();

        let mut exec_ctx = ExecutionContext::new(
            &mut event_driver,
            &scheduler,
            &mut deferred_spawns,
            CycleTime::new(Instant::now(), OffsetDateTime::now_utc()),
            1,
        );

        let result = graph.cycle(&mut exec_ctx, node);

        assert_eq!(result, Control::Broadcast);
        assert_eq!(call_count.get(), 1); // Function should have been called once
    }

    #[test]
    fn test_node_context_creation() {
        let call_count = Rc::new(Cell::new(0));
        let context = create_test_node(call_count, false);

        // Check initial values
        assert_eq!(context.sched_epoch, 0);
        assert_eq!(context.depth, 1);
    }

    #[test]
    fn test_multiple_triggering_edges() {
        let mut graph = Graph::new();
        let call_count = Rc::new(Cell::new(0));

        let parent = graph.add_node(create_test_node(call_count.clone(), true));
        let child1 = graph.add_node(create_test_node(call_count.clone(), false));
        let child2 = graph.add_node(create_test_node(call_count.clone(), false));
        let child3 = graph.add_node(create_test_node(call_count, false));

        graph.add_edge(parent, child1, Relationship::Trigger);
        graph.add_edge(parent, child2, Relationship::Trigger);
        graph.add_edge(parent, child3, Relationship::Observe); // This shouldn't appear

        let mut triggering_children: Vec<_> = graph.triggering_edges(parent).collect();
        triggering_children.sort(); // Order might not be guaranteed

        assert_eq!(triggering_children.len(), 2);
        assert!(triggering_children.contains(&child1));
        assert!(triggering_children.contains(&child2));
        assert!(!triggering_children.contains(&child3));
    }
}

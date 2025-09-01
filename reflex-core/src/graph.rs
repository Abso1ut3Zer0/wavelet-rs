use crate::executor::ExecutionContext;
use enum_as_inner::EnumAsInner;
use petgraph::prelude::{EdgeRef, NodeIndex};

pub(crate) type MutateFn = Box<dyn FnMut(&mut ExecutionContext) -> bool + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumAsInner)]
pub enum Relationship {
    Trigger,
    Observe,
}

pub(crate) struct NodeContext {
    pub(crate) mutate_fn: MutateFn,
    pub(crate) sched_epoch: usize,
    pub(crate) depth: u32,
}

impl NodeContext {
    pub(crate) const fn new(mutate_fn: MutateFn, depth: u32) -> Self {
        Self {
            mutate_fn,
            sched_epoch: 0,
            depth,
        }
    }

    #[inline(always)]
    fn mutate(&mut self, ctx: &mut ExecutionContext) -> bool {
        (self.mutate_fn)(ctx)
    }
}

pub struct Graph {
    inner: petgraph::Graph<NodeContext, Relationship>,
}

impl Graph {
    pub(crate) fn new() -> Self {
        Self {
            inner: petgraph::Graph::new(),
        }
    }

    #[inline(always)]
    pub(crate) fn can_schedule(&mut self, node_index: NodeIndex, epoch: usize) -> Option<u32> {
        let ctx = &mut self.inner[node_index];
        if ctx.sched_epoch == epoch {
            return None;
        }

        ctx.sched_epoch = epoch;
        Some(ctx.depth)
    }

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

    #[inline(always)]
    pub(crate) fn mutate(&mut self, ctx: &mut ExecutionContext, node_index: NodeIndex) -> bool {
        self.inner[node_index].mutate(ctx)
    }

    #[inline(always)]
    pub(crate) fn add_node(&mut self, weight: NodeContext) -> NodeIndex {
        self.inner.add_node(weight)
    }

    #[inline(always)]
    pub(crate) fn add_edge(
        &mut self,
        parent: NodeIndex,
        child: NodeIndex,
        relationship: Relationship,
    ) {
        self.inner.add_edge(parent, child, relationship);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_driver::EventDriver;
    use crate::executor::ExecutionContext;
    use crate::prelude::Scheduler;
    use std::cell::Cell;
    use std::rc::Rc;

    // Mock ExecutionContext for testing
    struct MockExecutionContext;

    fn create_test_node(call_count: Rc<Cell<i32>>, should_mutate: bool) -> NodeContext {
        NodeContext::new(
            Box::new(move |_ctx: &mut ExecutionContext| {
                call_count.set(call_count.get() + 1);
                should_mutate
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
        let mut scheduler = Scheduler::new();

        let mut exec_ctx = ExecutionContext::new(&mut event_driver, &mut scheduler, 1);

        let result = graph.mutate(&mut exec_ctx, node);

        assert_eq!(result, should_mutate);
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

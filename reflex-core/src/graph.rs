use crate::executor::ExecutionContext;
use enum_as_inner::EnumAsInner;
use petgraph::prelude::{EdgeRef, NodeIndex};

pub type MutateFn = Box<dyn FnMut(&mut ExecutionContext) -> bool + 'static>;

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
    pub fn new() -> Self {
        Self {
            inner: petgraph::Graph::new(),
        }
    }

    #[inline(always)]
    pub fn can_schedule(&mut self, node_index: NodeIndex, epoch: usize) -> Option<u32> {
        let ctx = &mut self.inner[node_index];
        if ctx.sched_epoch == epoch {
            return None;
        }

        ctx.sched_epoch = epoch;
        Some(ctx.depth)
    }

    #[inline(always)]
    pub fn triggering_edges(&self, node_index: NodeIndex) -> impl Iterator<Item = NodeIndex> {
        self.inner
            .edges_directed(node_index, petgraph::Direction::Outgoing)
            .filter(|edge| edge.weight().is_trigger())
            .map(|edge| edge.target())
    }

    #[inline(always)]
    pub fn mutate(&mut self, ctx: &mut ExecutionContext, node_index: NodeIndex) -> bool {
        self.inner[node_index].mutate(ctx)
    }

    #[inline(always)]
    pub fn add_node(&mut self, weight: NodeContext) -> NodeIndex {
        self.inner.add_node(weight)
    }

    #[inline(always)]
    pub fn add_edge(&mut self, parent: NodeIndex, child: NodeIndex, relationship: Relationship) {
        self.inner.add_edge(parent, child, relationship);
    }
}

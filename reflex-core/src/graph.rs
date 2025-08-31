use crate::event_driver::EventDriver;
use enum_as_inner::EnumAsInner;
use petgraph::prelude::NodeIndex;

pub type MutateFn = Box<dyn FnMut(&mut EventDriver) -> bool + 'static>;

pub(crate) struct Context {
    pub(crate) mutate_fn: MutateFn,
    pub(crate) sched_epoch: usize,
    pub(crate) depth: u32,
}

impl Context {
    pub(crate) const fn new(mutate_fn: MutateFn, depth: u32) -> Self {
        Self {
            mutate_fn,
            sched_epoch: 0,
            depth,
        }
    }

    #[inline(always)]
    fn mutate(&mut self, driver: &mut EventDriver) -> bool {
        (self.mutate_fn)(driver)
    }
}

pub struct Graph {
    graph: petgraph::Graph<Context, Relationship>,
}

impl Graph {
    pub fn new() -> Self {
        Self {
            graph: petgraph::Graph::new(),
        }
    }

    #[inline(always)]
    pub fn can_schedule(&mut self, node_index: NodeIndex, epoch: usize) -> Option<u32> {
        let ctx = &mut self.graph[node_index];
        if ctx.sched_epoch == epoch {
            return None;
        }

        ctx.sched_epoch = epoch;
        Some(ctx.depth)
    }

    #[inline(always)]
    pub fn mutate(&mut self, driver: &mut EventDriver, node_index: NodeIndex) -> bool {
        let ctx = &mut self.graph[node_index];
        ctx.mutate(driver)
    }

    #[inline(always)]
    pub(crate) fn add_node(&mut self, weight: Context) -> NodeIndex {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumAsInner)]
pub enum Relationship {
    Trigger,
    Observe,
}

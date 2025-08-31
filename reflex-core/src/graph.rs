use crate::Relationship;
use crate::event_driver::EventDriver;
use crate::scheduler::Scheduler;
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

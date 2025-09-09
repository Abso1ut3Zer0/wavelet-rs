use crate::graph::Graph;
use crate::scheduler::Scheduler;
use petgraph::prelude::NodeIndex;

pub struct YieldDriver {
    indices: Vec<NodeIndex>,
}

impl YieldDriver {
    pub const fn new() -> Self {
        Self {
            indices: Vec::new(),
        }
    }

    #[inline(always)]
    pub fn yield_now(&mut self, node_index: NodeIndex) {
        self.indices.push(node_index);
    }

    #[inline(always)]
    pub(crate) fn poll(&mut self, graph: &mut Graph, scheduler: &mut Scheduler, epoch: usize) {
        self.indices.drain(..).for_each(|idx| {
            if let Some(depth) = graph.can_schedule(idx, epoch) {
                scheduler.schedule(idx, depth);
            }
        })
    }
}

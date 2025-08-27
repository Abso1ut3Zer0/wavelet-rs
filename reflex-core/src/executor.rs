use crate::Relationship;
use crate::reactor::Reactor;
use petgraph::prelude::NodeIndex;

pub type CycleFn = Box<dyn FnMut(&mut Reactor) -> bool + 'static>;

pub struct Executor {
    graph: petgraph::Graph<CycleFn, Relationship>,
}

impl Executor {
    pub fn new() -> Self {
        Self {
            graph: petgraph::Graph::new(),
        }
    }

    pub(crate) fn add_node(&mut self, cycle: CycleFn) -> NodeIndex {
        self.graph.add_node(cycle)
    }

    pub(crate) fn add_edge(
        &mut self,
        parent: NodeIndex,
        child: NodeIndex,
        relationship: Relationship,
    ) {
        self.graph.add_edge(parent, child, relationship);
    }
}

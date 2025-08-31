use enum_as_inner::EnumAsInner;

mod node;
mod event_driver;
mod graph_manager;
mod scheduler;

pub use petgraph::prelude::NodeIndex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumAsInner)]
pub enum Relationship {
    Trigger,
    Observe,
}

#[derive(Debug, Clone, Copy)]
pub struct NodeEpoch {
    pub mut_epoch: usize,
    pub sched_epoch: usize,
}

// thread_local! {
//     pub(crate) static RUNNING: Cell<bool> = Cell::new(true);
//     pub(crate) static REACTOR: RefCell<Reactor> = RefCell::new(Reactor::new());
//     pub(crate) static CONTEXT: RefCell<Context> = RefCell::new(Context::new());
//     pub(crate) static GRAPH: RefCell<petgraph::Graph<CycleFn, Relationship>> =
//         RefCell::new(petgraph::Graph::new());
// }

use crate::context::Context;
use crate::reactor::Reactor;
use enum_as_inner::EnumAsInner;
use std::cell::{Cell, RefCell};

mod context;
mod node;
mod reactor;

pub type CycleFn = Box<dyn FnMut() -> bool + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumAsInner)]
pub enum Relationship {
    Trigger,
    Observe,
}

thread_local! {
    pub(crate) static RUNNING: Cell<bool> = Cell::new(true);
    pub(crate) static REACTOR: RefCell<Reactor> = RefCell::new(Reactor::new());
    pub(crate) static CONTEXT: RefCell<Context> = RefCell::new(Context::new());
    pub(crate) static GRAPH: RefCell<petgraph::Graph<CycleFn, Relationship>> =
        RefCell::new(petgraph::Graph::new());
}

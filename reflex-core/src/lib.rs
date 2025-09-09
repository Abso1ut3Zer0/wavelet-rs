pub mod clock;
pub mod event_driver;
pub mod executor;
pub mod graph;
pub mod node;
pub mod runtime;
pub mod scheduler;
mod garbage_collector;

use enum_as_inner::EnumAsInner;
pub use petgraph::prelude::NodeIndex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumAsInner)]
pub enum Relationship {
    Trigger,
    Observe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumAsInner)]
pub enum Control {
    Broadcast,
    Unchanged,
    Terminate,
}

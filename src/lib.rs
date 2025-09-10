use enum_as_inner::EnumAsInner;
pub use petgraph::prelude::NodeIndex;

#[cfg(feature = "factories")]
pub mod factory;
#[cfg(feature = "runtime")]
pub mod runtime;

/// Defines how nodes relate to each other in the computation graph.
///
/// Relationships determine when and how changes propagate between nodes.
/// This is fundamental to the incremental computation model - nodes only
/// recompute when their dependencies change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumAsInner)]
pub enum Relationship {
    /// The parent node will schedule this node when it mutates.
    ///
    /// Use `Trigger` when this node should react to changes
    /// in the parent node. This creates an active dependency.
    Trigger,

    /// This node can read from the parent but won't be automatically scheduled.
    ///
    /// Use `Observe` when this node needs access to the parent's data but
    /// doesn't need to react to changes in the parent. The node can check if the
    /// parent has mutated using `ExecutionContext::has_mutated()`. This is most
    /// commonly used when a node reacts to a specific trigger relationship to
    /// one parent but just needs the data of another.
    Observe,
}

/// Controls how a node's execution affects the broader computation graph.
///
/// Returned by a node's cycle function to indicate what should happen
/// after the node completes execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumAsInner)]
pub enum Control {
    /// Schedule all nodes with `Trigger` relationships to this node.
    ///
    /// Use when this node has mutated and downstream nodes should react
    /// to the changes.
    Broadcast,

    /// This node did not change - don't schedule any dependent nodes.
    ///
    /// Use when the node has processed data but its state didn't
    /// change in a way that would affect downstream computations.
    Unchanged,

    /// Signal the entire runtime to terminate gracefully.
    ///
    /// Use when a critical condition is met and the entire stream
    /// processing pipeline should shut down.
    Terminate,

    /// Mark this node for removal from the graph.
    ///
    /// Use when this node has completed its purpose and should be
    /// cleaned up. The node will be removed after the current cycle.
    Sweep,
}

pub mod prelude {
    pub use crate::{Control, Relationship};

    #[cfg(feature = "runtime")]
    pub use crate::runtime::*;

    #[cfg(feature = "factories")]
    pub use crate::factory::*;
}

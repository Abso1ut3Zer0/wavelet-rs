use crate::runtime::graph::Graph;
use crate::runtime::scheduler::Scheduler;
use petgraph::prelude::NodeIndex;

/// Manages immediate node scheduling for deferred execution.
///
/// The `YieldDriver` provides the framework's equivalent of `yield_now()` -
/// allowing nodes to defer their execution to the next cycle or schedule
/// themselves for immediate re-execution. This is particularly useful for:
///
/// - **Initialization**: Nodes can schedule themselves during `on_init`
/// - **Self-triggering**: Nodes that need to continue processing in the next cycle
/// - **Yield semantics**: Breaking up long computations across multiple cycles
/// - **Bootstrap patterns**: Starting execution chains without external triggers
///
/// Unlike I/O or timer events, yield scheduling happens immediately within
/// the current polling cycle, making it ideal for control flow management.
pub struct YieldDriver {
    /// Queue of nodes waiting to be scheduled in the current cycle
    indices: Vec<NodeIndex>,
}

impl YieldDriver {
    pub const fn new() -> Self {
        Self {
            indices: Vec::new(),
        }
    }

    /// Schedules a node for execution in the current cycle.
    ///
    /// This is the framework's `yield_now()` equivalent - the specified node
    /// will be added to the scheduler during the next `poll()` call. Common
    /// use cases:
    ///
    /// ```rust, ignore
    /// // In on_init: kick-start the node
    /// executor.yield_driver().yield_now(node_index);
    ///
    /// // In cycle function: continue processing next cycle
    /// ctx.yield_now(ctx.current());
    /// ```
    #[inline(always)]
    pub fn yield_now(&mut self, node_index: NodeIndex) {
        self.indices.push(node_index);
    }

    /// Processes all queued yield requests and schedules the nodes.
    ///
    /// Drains the internal queue and attempts to schedule each node,
    /// using epoch-based deduplication to prevent double-scheduling.
    /// This happens during each runtime polling cycle.
    #[inline(always)]
    pub(crate) fn poll(&mut self, graph: &mut Graph, scheduler: &mut Scheduler, epoch: usize) {
        self.indices.drain(..).for_each(|idx| {
            if let Some(depth) = graph.can_schedule(idx, epoch) {
                let _ = scheduler.schedule(idx, depth);
            }
        })
    }
}

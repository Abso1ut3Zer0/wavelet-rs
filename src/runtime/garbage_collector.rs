use petgraph::graph::NodeIndex;
use std::cell::UnsafeCell;
use std::collections::VecDeque;
use std::rc::Rc;

/// Manages deferred removal of nodes from the graph after cycle completion.
///
/// Please see the `test_garbage_collection` test for a simple
/// example of how to use the GarbageCollector.
/// ```
#[derive(Debug, Clone)]
pub struct GarbageCollector {
    /// Queue of node indices marked for removal
    ///
    /// SAFETY: UnsafeCell is safe here due to temporal separation:
    /// - Writes occur during cycle execution (node drops)
    /// - Reads/drains occur after cycle completion
    removal_queue: Rc<UnsafeCell<VecDeque<NodeIndex>>>,
}

impl GarbageCollector {
    pub(crate) fn new() -> Self {
        Self {
            removal_queue: Rc::new(UnsafeCell::new(VecDeque::new())),
        }
    }

    /// Mark a node for deferred removal after the current cycle.
    ///
    /// # Arguments
    ///
    /// * `node_index` - The index of the node to remove after cycle completion
    #[inline(always)]
    pub fn mark_for_sweep(&self, node: NodeIndex) {
        unsafe { &mut *self.removal_queue.get() }.push_back(node);
    }

    #[inline(always)]
    pub(crate) fn next_to_sweep(&self) -> Option<NodeIndex> {
        unsafe { &mut *self.removal_queue.get() }.pop_front()
    }
}

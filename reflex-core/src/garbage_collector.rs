use petgraph::graph::NodeIndex;
use std::cell::UnsafeCell;
use std::collections::VecDeque;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct GarbageCollector {
    scheduled: Rc<UnsafeCell<VecDeque<NodeIndex>>>,
}

impl GarbageCollector {
    pub(crate) fn new() -> Self {
        Self {
            scheduled: Rc::new(UnsafeCell::new(VecDeque::new())),
        }
    }

    #[inline(always)]
    pub(crate) fn next_node(&self) -> Option<NodeIndex> {
        unsafe { &mut *self.scheduled.get() }.pop_front()
    }

    #[inline(always)]
    pub fn mark_for_removal(&self, node: NodeIndex) {
        unsafe { &mut *self.scheduled.get() }.push_back(node);
    }
}

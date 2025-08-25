use crate::node::Node;

pub struct Context {
    pub(crate) current_epoch: usize,
    pub(crate) trigger_time: i128,
}

impl Context {
    pub(crate) const fn new() -> Self {
        Self {
            current_epoch: 0,
            trigger_time: 0,
        }
    }

    pub const fn trigger_time(&self) -> i128 {
        self.trigger_time
    }

    #[inline(always)]
    pub fn has_mutated<T>(&self, node: &Node<T>) -> bool {
        node.has_mutated(self.current_epoch)
    }

    pub(crate) const fn increment_epoch(&mut self) {
        self.current_epoch = self.current_epoch.wrapping_add(1);
    }
}

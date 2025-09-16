mod control;

use crate::Control;
use crate::prelude::{Executor, Node, NodeBuilder};

pub fn constant_node<T>(executor: &mut Executor, constant: T) -> Node<T> {
    NodeBuilder::new(constant).build(executor, |_, _| Control::Unchanged)
}

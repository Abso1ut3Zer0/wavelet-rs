use crate::Control;
use crate::prelude::{Executor, Node, NodeBuilder};

pub fn constant_node<T>(executor: &mut Executor, constant: T) -> Node<T> {
    NodeBuilder::new(constant).build(executor, |_, _| Control::Unchanged)
}

#[cfg(feature = "unstable")]
pub fn sweep_node<T, S>(
    executor: &mut Executor,
    node_to_sweep: Node<T>,
    sweep_trigger: Node<S>,
) -> Node<()> {
    NodeBuilder::new(())
        .triggered_by(&sweep_trigger)
        .observer_of(&node_to_sweep)
        .build(executor, move |_, _| {
            std::hint::black_box(node_to_sweep.borrow()); // black box to move the parent
            Control::Sweep
        })
}

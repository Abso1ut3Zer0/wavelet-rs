use crate::Control;
use crate::prelude::{Executor, Node, NodeBuilder};

pub fn periodic_trigger_node(executor: &mut Executor, duration: std::time::Duration) -> Node<()> {
    NodeBuilder::new(())
        .on_init(|ex, _, idx| {
            ex.yield_driver().yield_now(idx);
        })
        .build(executor, move |_, ctx| {
            ctx.register_timer(ctx.current(), ctx.now() + duration);
            Control::Broadcast
        })
}
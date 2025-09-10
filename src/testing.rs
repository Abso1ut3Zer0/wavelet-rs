use crate::Control;
use crate::runtime::{CycleOnce, Executor, Node, NodeBuilder, Notifier, TestRuntime};
use std::cell::{Ref, RefCell};

pub struct Push<T: 'static> {
    data: RefCell<T>,
    notifier: Option<Notifier>,
}

impl<T: 'static> Push<T> {
    const fn new(data: T) -> Self {
        Self {
            data: RefCell::new(data),
            notifier: None,
        }
    }

    pub fn data(&self) -> Ref<'_, T> {
        self.data.borrow()
    }

    pub fn push(&self, data: T) {
        *self.data.borrow_mut() = data;
        self.notifier.as_ref().map(|n| n.notify());
    }

    pub fn push_with_cycle(&self, runtime: &mut TestRuntime, data: T) {
        self.push(data);
        runtime.cycle_once();
    }

    pub fn push_with_cycle_advance(
        &self,
        runtime: &mut TestRuntime,
        data: T,
        duration: std::time::Duration,
    ) {
        self.push(data);
        runtime.advance_clock(duration);
        runtime.cycle_once();
    }
}

pub fn create_push_node<T: 'static>(executor: &mut Executor, data: T) -> Node<Push<T>> {
    NodeBuilder::new(Push::new(data))
        .on_init(|ex, this, idx| {
            let notifier = ex
                .io_driver()
                .register_notifier(idx)
                .expect("failed to register notifier");
            this.notifier = Some(notifier);
        })
        .build(executor, |_, _| Control::Broadcast)
}

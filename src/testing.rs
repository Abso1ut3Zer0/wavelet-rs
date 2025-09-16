use crate::Control;
use crate::runtime::{CycleOnce, Executor, Node, NodeBuilder, Notifier, TestRuntime};
use std::cell::{Ref, RefCell};
use std::rc::Rc;

pub struct Push<T: 'static> {
    inner: Rc<RefCell<PushInner<T>>>,
}

impl<T: 'static> Clone for Push<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T: 'static> Push<T> {
    pub fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(PushInner::new())),
        }
    }

    pub fn push(&self, data: T) {
        self.inner.borrow_mut().push(data);
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

    fn take(&self) -> T {
        self.inner.borrow_mut().take()
    }

    fn register_notifier(&self, notifier: Notifier) {
        self.inner.borrow_mut().register_notifier(notifier);
    }
}

pub struct PushInner<T: 'static> {
    data: T,
    notifier: Option<Notifier>,
}

impl<T: 'static> PushInner<T> {
    const fn new() -> Self {
        Self {
            data: unsafe { std::mem::zeroed() },
            notifier: None,
        }
    }

    pub fn push(&mut self, data: T) {
        self.data = data;
        self.notifier.as_ref().map(|n| n.notify());
    }

    fn take(&mut self) -> T {
        let mut val = unsafe { std::mem::zeroed() };
        std::mem::swap(&mut self.data, &mut val);
        val
    }

    fn register_notifier(&mut self, notifier: Notifier) {
        self.notifier = Some(notifier);
    }
}

pub fn push_node<T: 'static>(executor: &mut Executor, data: T) -> (Node<T>, Push<T>) {
    let push = Push::new();
    let push_on_init = push.clone();
    let push_on_cycle = push.clone();
    (
        NodeBuilder::new(data)
            .on_init(move |ex, this, idx| {
                let notifier = ex
                    .io_driver()
                    .register_notifier(idx)
                    .expect("failed to register notifier");
                push_on_init.register_notifier(notifier);
            })
            .build(executor, move |this, _| {
                *this = push_on_cycle.take();
                Control::Broadcast
            }),
        push,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_node() {
        let mut runtime = TestRuntime::new();
        let (parent, push) = push_node(runtime.executor(), 0);

        let parent = parent.clone();
        let child = NodeBuilder::new("0".to_string())
            .triggered_by(&parent)
            .build(runtime.executor(), move |data, _| {
                *data = parent.borrow().to_string();
                Control::Broadcast
            });

        push.push_with_cycle(&mut runtime, 2);
        println!("node data: {}", child.borrow());
        println!("child epoch: {:?}", child.mut_epoch());
        assert!(runtime.executor().has_mutated(&child));
        assert_eq!(child.borrow().as_str(), "2");
    }
}

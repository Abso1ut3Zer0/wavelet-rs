use crate::Control;
use crate::prelude::{CycleTime, EventDriver, ExecutionContext, Scheduler, SpawnFn};
use crate::runtime::{Executor, Node, NodeBuilder, Notifier, TestRuntime};
use petgraph::graph::NodeIndex;
use std::cell::{RefCell, UnsafeCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Instant;
use time::OffsetDateTime;

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

impl<T: Default + 'static> Push<T> {
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
        runtime.run_one_cycle();
    }

    pub fn push_with_cycle_advance(
        &self,
        runtime: &mut TestRuntime,
        data: T,
        duration: std::time::Duration,
    ) {
        self.push(data);
        runtime.advance_clock(duration);
        runtime.run_one_cycle();
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

impl<T: Default + 'static> PushInner<T> {
    fn new() -> Self {
        Self {
            data: T::default(),
            notifier: None,
        }
    }

    pub fn push(&mut self, data: T) {
        self.data = data;
        self.notifier.as_ref().map(|n| n.notify());
    }

    fn take(&mut self) -> T {
        std::mem::take(&mut self.data)
    }

    fn register_notifier(&mut self, notifier: Notifier) {
        self.notifier = Some(notifier);
    }
}

pub fn push_node<T: Default + 'static>(executor: &mut Executor, data: T) -> (Node<T>, Push<T>) {
    let push = Push::new();
    let push_on_init = push.clone();
    let push_on_cycle = push.clone();
    (
        NodeBuilder::new(data)
            .on_init(move |ex, _, idx| {
                let notifier = ex.register_notifier(idx);
                push_on_init.register_notifier(notifier);
            })
            .build(executor, move |this, _| {
                *this = push_on_cycle.take();
                Control::Broadcast
            }),
        push,
    )
}

pub struct MockContextGenerator {
    event_driver: EventDriver,
    scheduler: UnsafeCell<Scheduler>,
    deferred_spawns: VecDeque<SpawnFn>,
    current: NodeIndex,
    cycle_time: CycleTime,
    epoch: usize,
}

impl MockContextGenerator {
    pub fn new() -> Self {
        Self {
            event_driver: EventDriver::new(),
            scheduler: UnsafeCell::new(Scheduler::new()),
            deferred_spawns: VecDeque::new(),
            current: Default::default(),
            cycle_time: CycleTime::new(Instant::now(), OffsetDateTime::now_utc()),
            epoch: 0,
        }
    }

    pub fn context(&mut self) -> ExecutionContext<'_> {
        ExecutionContext::new(
            &mut self.event_driver,
            &self.scheduler,
            &mut self.deferred_spawns,
            self.cycle_time.clone(),
            self.epoch,
        )
    }

    pub fn set_current(&mut self, node_index: NodeIndex) {
        self.current = node_index;
    }

    pub fn set_cycle_time(&mut self, cycle_time: CycleTime) {
        self.cycle_time = cycle_time;
    }

    pub fn advance_epoch(&mut self) {
        self.epoch += 1;
    }
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

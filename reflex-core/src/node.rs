use crate::event_driver::Notifier;
use crate::executor::{ExecutionContext, Executor};
use crate::graph::NodeContext;
use crate::{Control, Relationship};
use petgraph::prelude::NodeIndex;
use std::cell::UnsafeCell;
use std::io;
use std::rc::Rc;

type OnDrop<T> = Box<dyn FnMut(&mut T) + 'static>;

pub struct Node<T: 'static>(Rc<UnsafeCell<NodeInner<T>>>);

impl<T: 'static> Node<T> {
    pub(crate) fn uninitialized(data: T, name: Option<String>) -> Self {
        Self {
            0: Rc::new(UnsafeCell::new(NodeInner {
                data,
                name,
                on_drop: None,
                index: NodeIndex::new(0),
                mut_epoch: 0,
                depth: 0,
            })),
        }
    }

    #[inline(always)]
    pub fn name(&self) -> Option<&str> {
        self.get().name.as_deref()
    }

    #[inline(always)]
    pub fn borrow(&self) -> &T {
        &self.get().data
    }

    #[inline(always)]
    pub(crate) fn borrow_mut(&self) -> &mut T {
        &mut self.get_mut().data
    }

    #[inline(always)]
    fn get(&self) -> &NodeInner<T> {
        unsafe { &*self.0.get() }
    }

    #[inline(always)]
    fn get_mut(&self) -> &mut NodeInner<T> {
        unsafe { &mut *self.0.get() }
    }

    #[inline(always)]
    pub(crate) fn index(&self) -> NodeIndex {
        self.get().index
    }

    #[inline(always)]
    pub(crate) fn depth(&self) -> u32 {
        self.get().depth
    }

    #[inline(always)]
    pub(crate) fn mut_epoch(&self) -> usize {
        self.get().mut_epoch
    }
}

impl<T: 'static> Clone for Node<T> {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

struct NodeInner<T: 'static> {
    data: T,
    name: Option<String>,
    on_drop: Option<OnDrop<T>>,
    index: NodeIndex,
    mut_epoch: usize,
    depth: u32,
}

impl<T: 'static> Drop for NodeInner<T> {
    fn drop(&mut self) {
        self.on_drop
            .take()
            .map(|mut on_drop| (on_drop)(&mut self.data));
    }
}

pub struct NodeBuilder<T: 'static> {
    data: T,
    name: Option<String>,
    parents: Vec<(NodeIndex, u32, Relationship)>,
    on_init: Option<Box<dyn FnMut(&mut Executor, &mut T, NodeIndex) + 'static>>,
    on_drop: Option<OnDrop<T>>,
}

impl<T: 'static> NodeBuilder<T> {
    pub fn new(data: T) -> Self {
        Self {
            data,
            name: None,
            parents: Vec::new(),
            on_init: None,
            on_drop: None,
        }
    }

    pub fn with_name(mut self, name: String) -> Self {
        self.name = Some(name);
        self
    }

    pub fn add_relationship<P>(mut self, parent: &Node<P>, relationship: Relationship) -> Self {
        self.parents
            .push((parent.index(), parent.depth(), relationship));
        self
    }

    pub fn on_init<F>(mut self, on_init: F) -> Self
    where
        F: FnMut(&mut Executor, &mut T, NodeIndex) + 'static,
    {
        assert!(self.on_init.is_none(), "cannot set on_init twice");
        self.on_init = Some(Box::new(on_init));
        self
    }

    pub fn on_drop<F>(mut self, on_drop: F) -> Self
    where
        F: FnMut(&mut T) + 'static,
    {
        self.on_drop = Some(Box::new(on_drop));
        self
    }

    pub fn build<F>(self, executor: &mut Executor, mut cycle_fn: F) -> Node<T>
    where
        F: FnMut(&mut T, &mut ExecutionContext) -> Control + 'static,
    {
        let node = Node::uninitialized(self.data, self.name);
        let depth = self
            .parents
            .iter()
            .map(|(_, depth, _)| depth)
            .max()
            .map(|d| d + 1)
            .unwrap_or(0);

        {
            let state = node.clone();
            let cycle_fn =
                Box::new(move |ctx: &mut ExecutionContext| cycle_fn(state.borrow_mut(), ctx));

            let idx = executor.graph().add_node(NodeContext::new(cycle_fn, depth));
            let inner = node.get_mut();
            inner.index = idx;
            inner.depth = depth;

            self.parents.iter().for_each(|(parent, _, relationship)| {
                executor.graph().add_edge(*parent, idx, *relationship);
            });

            executor.scheduler().enable_depth(depth);
            if let Some(mut on_init) = self.on_init {
                (on_init)(executor, &mut node.get_mut().data, idx)
            }

            inner.on_drop = self.on_drop;
        }

        node
    }

    pub fn build_with_notifier<F>(
        self,
        executor: &mut Executor,
        cycle_fn: F,
    ) -> io::Result<(Node<T>, Notifier)>
    where
        F: FnMut(&mut T, &mut ExecutionContext) -> Control + 'static,
    {
        let node = self.build(executor, cycle_fn);
        let notifier = executor.io_driver().register_notifier(node.index())?;
        Ok((node, notifier))
    }
}

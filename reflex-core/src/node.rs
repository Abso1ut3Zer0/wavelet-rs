use crate::executor::{ExecutionContext, Executor};
use crate::graph::Relationship;
use crate::graph::{Graph, NodeContext};
use petgraph::prelude::NodeIndex;
use std::cell::UnsafeCell;
use std::rc::Rc;

pub struct Node<T: 'static>(Rc<UnsafeCell<NodeInner<T>>>);

impl<T: 'static> Node<T> {
    pub(crate) fn uninitialized(data: T, name: Option<String>) -> Self {
        Self {
            0: Rc::new(UnsafeCell::new(NodeInner {
                data,
                name,
                index: NodeIndex::new(0),
                mut_epoch: 0,
                depth: 0,
            })),
        }
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
        unsafe { self.0.get().as_ref().unwrap_unchecked() }
    }

    #[inline(always)]
    fn get_mut(&self) -> &mut NodeInner<T> {
        unsafe { self.0.get().as_mut().unwrap_unchecked() }
    }

    #[inline(always)]
    pub(crate) fn name(&self) -> Option<&str> {
        self.get().name.as_deref()
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
    index: NodeIndex,
    mut_epoch: usize,
    depth: u32,
}

pub struct NodeBuilder<T: 'static> {
    data: T,
    name: Option<String>,
    parents: Vec<(NodeIndex, u32, Relationship)>,
    on_init: Option<Box<dyn FnMut(&mut Executor, &mut T, NodeIndex) + 'static>>,
}

impl<T: 'static> NodeBuilder<T> {
    pub fn new(data: T) -> Self {
        Self {
            data,
            name: None,
            parents: Vec::new(),
            on_init: None,
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
        self.on_init = Some(Box::new(on_init));
        self
    }

    pub fn build<F>(self, executor: &mut Executor, mut cycle_fn: F) -> Node<T>
    where
        F: FnMut(&mut T, &mut ExecutionContext) -> bool + 'static,
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
            let mut inner = node.get_mut();
            inner.index = idx;
            inner.depth = depth;

            self.parents.iter().for_each(|(parent, _, relationship)| {
                executor.graph().add_edge(*parent, idx, *relationship);
            });

            executor.scheduler().enable_depth(depth);
            if let Some(mut on_init) = self.on_init {
                (on_init)(executor, &mut node.get_mut().data, idx)
            }
        }

        node
    }
}

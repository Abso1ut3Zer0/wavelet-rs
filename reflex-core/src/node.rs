use crate::Relationship;
use crate::executor::Executor;
use crate::reactor::Reactor;
use petgraph::prelude::NodeIndex;
use std::cell::UnsafeCell;
use std::rc::{Rc, Weak};

pub struct Node<T: 'static>(Rc<UnsafeCell<NodeInner<T>>>);

impl<T: 'static> Node<T> {
    pub(crate) fn uninitialized(data: T, name: Option<String>) -> Self {
        Self {
            0: Rc::new(UnsafeCell::new(NodeInner {
                data,
                name,
                epoch: NodeEpoch {
                    mut_epoch: 0,
                    sched_epoch: 0,
                },
                index: NodeIndex::new(0),
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

    fn weak(&self) -> WeakNode<T> {
        WeakNode(Rc::downgrade(&self.0))
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
    pub(crate) fn is_scheduled(&self, current_epoch: usize) -> bool {
        self.get().epoch.sched_epoch == current_epoch
    }

    #[inline(always)]
    pub(crate) fn has_mutated(&self, current_epoch: usize) -> bool {
        self.get().epoch.mut_epoch == current_epoch
    }
}

impl<T: 'static> Clone for Node<T> {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct NodeEpoch {
    pub(crate) mut_epoch: usize,
    pub(crate) sched_epoch: usize,
}

struct NodeInner<T: 'static> {
    data: T,
    name: Option<String>,
    epoch: NodeEpoch,
    index: NodeIndex,
    depth: u32,
}

struct WeakNode<T: 'static>(Weak<UnsafeCell<NodeInner<T>>>);

impl<T: 'static> WeakNode<T> {
    #[inline(always)]
    pub fn upgrade(&self) -> Option<Node<T>> {
        self.0.upgrade().map(|rc| Node(rc))
    }
}

pub struct NodeBuilder<T: 'static> {
    data: T,
    name: Option<String>,
    parents: Vec<(NodeIndex, u32, Relationship)>,
}

impl<T: 'static> NodeBuilder<T> {
    pub fn new(data: T) -> Self {
        Self {
            data,
            name: None,
            parents: Vec::new(),
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

    pub fn build<F>(self, executor: &mut Executor, mut cycle_fn: F) -> Node<T>
    where
        F: FnMut(&mut T, &mut Reactor) -> bool + 'static,
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
            let idx = node.index();
            let weak = node.weak();
            let cycle_fn = Box::new(move |reactor: &mut Reactor| match weak.upgrade() {
                None => {
                    reactor.register_garbage(idx);
                    false
                }
                Some(state) => cycle_fn(state.borrow_mut(), reactor),
            });

            let idx = executor.add_node(cycle_fn);
            let mut inner = node.get_mut();
            inner.index = idx;
            inner.depth = depth;

            self.parents.iter().for_each(|(parent, _, relationship)| {
                executor.add_edge(*parent, idx, *relationship);
            });
        }

        node
    }
}

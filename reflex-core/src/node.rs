use crate::Relationship;
use crate::executor::Executor;
use crate::reactor::Reactor;
use petgraph::prelude::NodeIndex;
use std::cell::{Ref, RefCell, RefMut};
use std::rc::Rc;

pub struct Node<T: 'static>(Rc<RefCell<NodeInner<T>>>);

impl<T: 'static> Node<T> {
    pub(crate) fn uninitialized(data: T, name: Option<String>) -> Self {
        Self {
            0: Rc::new(RefCell::new(NodeInner {
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
    pub fn borrow(&self) -> Ref<'_, T> {
        Ref::map(self.0.borrow(), |inner| &inner.data)
    }

    #[inline(always)]
    pub(crate) fn borrow_mut(&self) -> RefMut<'_, T> {
        RefMut::map(self.0.borrow_mut(), |inner| &mut inner.data)
    }

    #[inline(always)]
    pub(crate) fn borrow_name(&self) -> Ref<'_, Option<String>> {
        Ref::map(self.0.borrow(), |inner| &inner.name)
    }

    #[inline(always)]
    pub(crate) fn index(&self) -> NodeIndex {
        self.0.borrow().index
    }

    #[inline(always)]
    pub(crate) fn depth(&self) -> u32 {
        self.0.borrow().depth
    }

    #[inline(always)]
    pub(crate) fn is_scheduled(&self, current_epoch: usize) -> bool {
        self.0.borrow().epoch.sched_epoch == current_epoch
    }

    #[inline(always)]
    pub(crate) fn has_mutated(&self, current_epoch: usize) -> bool {
        self.0.borrow().epoch.mut_epoch == current_epoch
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
            let cloned = node.clone();
            let cycle_fn = Box::new(move |reactor: &mut Reactor| {
                let state: &mut T = &mut *cloned.borrow_mut();
                cycle_fn(state, reactor)
            });

            let idx = executor.add_node(cycle_fn);
            let mut inner = node.0.borrow_mut();
            inner.index = idx;
            inner.depth = depth;

            self.parents.iter().for_each(|(parent, _, relationship)| {
                executor.add_edge(*parent, idx, *relationship);
            });
        }

        node
    }
}

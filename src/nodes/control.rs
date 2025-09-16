use crate::Control;
use crate::prelude::{Executor, Node, NodeBuilder};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::Hash;
use std::rc::Rc;

pub struct Router<K: Eq + Hash, T: 'static> {
    parent: Option<Node<Vec<T>>>,
    cache: RefCell<HashMap<K, Node<Vec<T>>>>,
    epoch: Rc<Cell<usize>>,
}

impl<K: Eq + Hash, T: 'static> Router<K, T> {
    fn new(parent: Option<Node<Vec<T>>>) -> Self {
        Self {
            parent,
            cache: RefCell::new(HashMap::new()),
            epoch: Rc::new(Cell::new(usize::MAX)),
        }
    }

    pub fn route(&self, executor: &mut Executor, key: K) -> Node<Vec<T>> {
        let epoch = self.epoch.clone();
        self.cache
            .borrow_mut()
            .entry(key)
            .or_insert(match &self.parent {
                None => NodeBuilder::new(Vec::new()).build(executor, move |this, ctx| {
                    if ctx.epoch() != epoch.get() {
                        this.clear();
                    }

                    epoch.set(ctx.epoch());
                    (!this.is_empty()).into()
                }),
                Some(parent) => NodeBuilder::new(Vec::new()).observer_of(parent).build(
                    executor,
                    move |this, ctx| {
                        if ctx.epoch() != epoch.get() {
                            this.clear();
                        }

                        epoch.set(ctx.epoch());
                        (!this.is_empty()).into()
                    },
                ),
            })
            .clone()
    }
}

pub fn take_router_node<K: Eq + Hash, T: 'static>(
    executor: &mut Executor,
    source: Node<Vec<T>>,
    route: impl Fn(&T) -> &K + 'static,
) -> Node<Router<K, T>> {
    let source = source.clone();
    NodeBuilder::new(Router::new(Some(source.clone())))
        .triggered_by(&source)
        .build(executor, move |this, ctx| {
            source.borrow_mut().drain(..).for_each(|item| {
                if let Some(node) = this.cache.borrow_mut().get_mut(route(&item)) {
                    node.borrow_mut().push(item);
                    ctx.schedule_node(node).expect("failed to schedule node");
                }
            });
            Control::Unchanged
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::*;
    use crate::testing::push_node;

    #[test]
    fn test_take_router() {
        let mut executor = Executor::new();
        let mut clock = TestClock::new();

        // let parent = push_node(&mut executor, Vec::new());
        // let router = take_router_node(&mut executor, parent.clone(), |item| item % 2);
    }
}

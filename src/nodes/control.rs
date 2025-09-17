use crate::Control;
use crate::channel::{Sender, TryReceiveError};
use crate::prelude::{Executor, Node, NodeBuilder, WeakNode};
use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::Hash;
use std::rc::Rc;

pub struct Router<K: Eq + Hash, T: 'static> {
    parent: Option<Node<Vec<T>>>,
    cache: Rc<RefCell<HashMap<K, (WeakNode<Vec<T>>, usize)>>>,
}

impl<K: Clone + Eq + Hash + 'static, T: 'static> Router<K, T> {
    fn new(parent: Option<Node<Vec<T>>>) -> Self {
        Self {
            parent,
            cache: Rc::new(RefCell::new(HashMap::new())),
        }
    }

    pub fn route(&self, executor: &mut Executor, key: K) -> Node<Vec<T>> {
        let mut cache = self.cache.borrow_mut();

        if let Some((weak, _)) = cache.get(&key)
            && let Some(node) = weak.upgrade()
        {
            return node;
        }

        let (node, epoch) = self.new_node(executor, key.clone());
        cache.insert(key, (node.downgrade(), epoch));
        node
    }

    fn new_node(&self, executor: &mut Executor, key: K) -> (Node<Vec<T>>, usize) {
        let cache = self.cache.clone();
        match &self.parent {
            None => (
                NodeBuilder::new(Vec::new())
                    .on_drop(move |_| {
                        cache.borrow_mut().remove(&key);
                    })
                    .build(executor, move |this, _| (!this.is_empty()).into()),
                0,
            ),
            Some(parent) => (
                NodeBuilder::new(Vec::new())
                    .on_drop(move |_| {
                        cache.borrow_mut().remove(&key);
                    })
                    .observer_of(parent)
                    .build(executor, move |this, _| (!this.is_empty()).into()),
                0,
            ),
        }
    }
}

pub fn take_router_node<K: Clone + Eq + Hash, T: 'static>(
    executor: &mut Executor,
    source: Node<Vec<T>>,
    route: impl Fn(&T) -> K + 'static,
) -> Node<Router<K, T>> {
    let source = source.clone();
    let gc = executor.garbage_collector();
    NodeBuilder::new(Router::new(Some(source.clone())))
        .triggered_by(&source)
        .on_drop(move |this| {
            this.cache.borrow().values().for_each(|(weak, _)| {
                weak.upgrade().map(|node| gc.mark_for_sweep(node.index()));
            });
            this.cache.borrow_mut().clear();
        })
        .build(executor, move |this, ctx| {
            source.borrow_mut().drain(..).for_each(|item| {
                if let Some((node, epoch)) = this.cache.borrow_mut().get_mut(&route(&item))
                    && let Some(node) = node.upgrade()
                {
                    if *epoch != ctx.epoch() {
                        node.borrow_mut().clear();
                        *epoch = ctx.epoch();
                        ctx.schedule_node(&node).expect("failed to schedule node");
                    }
                    node.borrow_mut().push(item);
                }
            });
            Control::Unchanged
        })
}

pub fn channel_router_node<K: Clone + Eq + Hash, T: 'static>(
    executor: &mut Executor,
    capacity: usize,
    poll_limit: usize,
    route: impl Fn(&T) -> K + 'static,
) -> std::io::Result<(Node<Router<K, T>>, Sender<T>)> {
    let gc = executor.garbage_collector();
    NodeBuilder::new(Router::new(None))
        .on_drop(move |this| {
            this.cache.borrow().values().for_each(|(weak, _)| {
                weak.upgrade().map(|node| gc.mark_for_sweep(node.index()));
            });
            this.cache.borrow_mut().clear();
        })
        .build_with_channel(executor, capacity, move |this, ctx, rx| {
            for _ in 0..poll_limit {
                match rx.try_receive() {
                    Ok(item) => {
                        if let Some((node, epoch)) = this.cache.borrow_mut().get_mut(&route(&item))
                            && let Some(node) = node.upgrade()
                        {
                            if *epoch != ctx.epoch() {
                                node.borrow_mut().clear();
                                *epoch = ctx.epoch();
                                ctx.schedule_node(&node).expect("failed to schedule node");
                            }
                            node.borrow_mut().push(item);
                        }
                    }
                    Err(TryReceiveError::Empty) => break,
                    Err(TryReceiveError::ChannelClosed) => return Control::Sweep,
                }
            }
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
        let mut runtime = TestRuntime::new();
        let (parent, push) = push_node(runtime.executor(), Vec::new());
        let router = take_router_node(runtime.executor(), parent.clone(), |item| item % 2);
        let node0 = router.borrow().route(runtime.executor(), 0);
        let node1 = router.borrow().route(runtime.executor(), 1);
        assert_eq!(router.borrow().cache.borrow().len(), 2);

        push.push_with_cycle(&mut runtime, vec![0]);
        println!("node0: {:?}", node0.borrow());
        println!("node1: {:?}", node1.borrow());

        assert!(runtime.executor().has_mutated(&node0));
        assert!(!runtime.executor().has_mutated(&node1));
        assert_eq!(node0.borrow().len(), 1);
        assert_eq!(node1.borrow().len(), 0);
        assert_eq!(node0.borrow()[0], 0);

        push.push_with_cycle(&mut runtime, vec![2, 3, 5]);
        println!("node0: {:?}", node0.borrow());
        println!("node1: {:?}", node1.borrow());

        assert!(runtime.executor().has_mutated(&node0));
        assert!(runtime.executor().has_mutated(&node1));
        assert_eq!(node0.borrow().len(), 1);
        assert_eq!(node1.borrow().len(), 2);
        assert_eq!(node0.borrow()[0], 2);
        assert_eq!(node1.borrow()[0], 3);
        assert_eq!(node1.borrow()[1], 5);

        drop(node0);
        drop(node1);
        assert_eq!(router.borrow().cache.borrow().len(), 0);
    }

    #[test]
    fn test_channel_router() {
        let mut runtime = TestRuntime::new();
        let (router, tx) = channel_router_node(runtime.executor(), 8, 8, |item| item % 2).unwrap();
        let node0 = router.borrow().route(runtime.executor(), 0);
        let node1 = router.borrow().route(runtime.executor(), 1);
        assert_eq!(router.borrow().cache.borrow().len(), 2);

        tx.blocking_send(0).unwrap();
        runtime.cycle_once();
        println!("node0: {:?}", node0.borrow());
        println!("node1: {:?}", node1.borrow());

        assert!(runtime.executor().has_mutated(&node0));
        assert!(!runtime.executor().has_mutated(&node1));
        assert_eq!(node0.borrow().len(), 1);
        assert_eq!(node1.borrow().len(), 0);
        assert_eq!(node0.borrow()[0], 0);

        tx.blocking_send(2).unwrap();
        tx.blocking_send(3).unwrap();
        tx.blocking_send(5).unwrap();
        runtime.cycle_once();
        println!("node0: {:?}", node0.borrow());
        println!("node1: {:?}", node1.borrow());

        assert!(runtime.executor().has_mutated(&node0));
        assert!(runtime.executor().has_mutated(&node1));
        assert_eq!(node0.borrow().len(), 1);
        assert_eq!(node1.borrow().len(), 2);
        assert_eq!(node0.borrow()[0], 2);
        assert_eq!(node1.borrow()[0], 3);
        assert_eq!(node1.borrow()[1], 5);

        drop(node0);
        drop(node1);
        assert_eq!(router.borrow().cache.borrow().len(), 0);
    }

    #[test]
    fn test_router_drop() {
        let mut runtime = TestRuntime::new();
        let (parent, push) = push_node(runtime.executor(), Vec::new());
        let router = take_router_node(runtime.executor(), parent.clone(), |item| item % 2);
        let node0 = router.borrow().route(runtime.executor(), 0);
        let node1 = router.borrow().route(runtime.executor(), 1);
        assert_eq!(router.borrow().cache.borrow().len(), 2);

        drop(router);
        runtime.cycle_once();
        assert_eq!(runtime.executor().graph().node_count(), 1); // only push node remaining
    }
}

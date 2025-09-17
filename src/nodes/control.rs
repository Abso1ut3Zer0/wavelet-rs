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
                    .build(executor, move |this, _| Control::from(!this.is_empty())),
                0,
            ),
            Some(parent) => (
                NodeBuilder::new(Vec::new())
                    .on_drop(move |_| {
                        cache.borrow_mut().remove(&key);
                    })
                    .observer_of(parent)
                    .build(executor, move |this, _| Control::from(!this.is_empty())),
                0,
            ),
        }
    }
}

pub fn take_route_stream_node<K: Clone + Eq + Hash, T>(
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

pub fn channel_route_stream_node<K: Clone + Eq + Hash, T>(
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

pub fn switch_stream_node<T: Clone>(
    executor: &mut Executor,
    primary: Node<Vec<T>>,
    secondary: Node<Vec<T>>,
    switch: Node<bool>,
) -> Node<Vec<T>> {
    let mut use_primary = *switch.borrow();
    NodeBuilder::new(Vec::new())
        .triggered_by(&primary)
        .triggered_by(&secondary)
        .triggered_by(&switch)
        .build(executor, move |this, ctx| {
            if ctx.has_mutated(&switch) {
                this.clear();
                use_primary = *switch.borrow();
                if use_primary {
                    this.extend(primary.borrow().iter().cloned());
                } else {
                    this.extend(secondary.borrow().iter().cloned());
                }

                return Control::from(!this.is_empty());
            }

            if use_primary && ctx.has_mutated(&primary) {
                this.clear();
                this.extend(primary.borrow().iter().cloned());
                return Control::from(!this.is_empty());
            } else if !use_primary && ctx.has_mutated(&secondary) {
                this.clear();
                this.extend(secondary.borrow().iter().cloned());
                return Control::from(!this.is_empty());
            }

            Control::Unchanged
        })
}

pub fn take_switch_stream_node<T>(
    executor: &mut Executor,
    primary: Node<Vec<T>>,
    secondary: Node<Vec<T>>,
    switch: Node<bool>,
) -> Node<Vec<T>> {
    let mut use_primary = *switch.borrow();
    NodeBuilder::new(Vec::new())
        .triggered_by(&primary)
        .triggered_by(&secondary)
        .triggered_by(&switch)
        .build(executor, move |this, ctx| {
            if ctx.has_mutated(&switch) {
                this.clear();
                use_primary = *switch.borrow();
                if use_primary {
                    this.extend(primary.borrow_mut().drain(..));
                } else {
                    this.extend(secondary.borrow_mut().drain(..));
                }

                return Control::from(!this.is_empty());
            }

            if use_primary && ctx.has_mutated(&primary) {
                this.clear();
                this.extend(primary.borrow_mut().drain(..));
                return Control::from(!this.is_empty());
            } else if !use_primary && ctx.has_mutated(&secondary) {
                this.clear();
                this.extend(secondary.borrow_mut().drain(..));
                return Control::from(!this.is_empty());
            }

            Control::Unchanged
        })
}

pub fn switch_node<T: Clone>(
    executor: &mut Executor,
    primary: Node<T>,
    secondary: Node<T>,
    switch: Node<bool>,
    initial: T,
) -> Node<T> {
    let mut use_primary = *switch.borrow();
    NodeBuilder::new(initial)
        .triggered_by(&primary)
        .triggered_by(&secondary)
        .triggered_by(&switch)
        .build(executor, move |this, ctx| {
            if ctx.has_mutated(&switch) {
                use_primary = *switch.borrow();
                if use_primary {
                    *this = primary.borrow().clone();
                } else {
                    *this = secondary.borrow().clone();
                }

                return Control::Broadcast;
            }

            if use_primary && ctx.has_mutated(&primary) {
                *this = primary.borrow().clone();
                return Control::Broadcast;
            } else if !use_primary && ctx.has_mutated(&secondary) {
                *this = secondary.borrow().clone();
                return Control::Broadcast;
            }

            Control::Unchanged
        })
}

pub fn take_switch_node<T: Default>(
    executor: &mut Executor,
    primary: Node<T>,
    secondary: Node<T>,
    switch: Node<bool>,
) -> Node<T> {
    let mut use_primary = *switch.borrow();
    NodeBuilder::new(T::default())
        .triggered_by(&primary)
        .triggered_by(&secondary)
        .triggered_by(&switch)
        .build(executor, move |this, ctx| {
            if ctx.has_mutated(&switch) {
                use_primary = *switch.borrow();
                if use_primary {
                    *this = std::mem::take(primary.borrow_mut());
                } else {
                    *this = std::mem::take(secondary.borrow_mut());
                }

                return Control::Broadcast;
            }

            if use_primary && ctx.has_mutated(&primary) {
                *this = std::mem::take(primary.borrow_mut());
                return Control::Broadcast;
            } else if !use_primary && ctx.has_mutated(&secondary) {
                *this = std::mem::take(secondary.borrow_mut());
                return Control::Broadcast;
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
        let router = take_route_stream_node(runtime.executor(), parent.clone(), |item| item % 2);
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
        let (router, tx) =
            channel_route_stream_node(runtime.executor(), 8, 8, |item| item % 2).unwrap();
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
        let (parent, _push) = push_node(runtime.executor(), Vec::new());
        let router = take_route_stream_node(runtime.executor(), parent.clone(), |item| item % 2);
        let _node0 = router.borrow().route(runtime.executor(), 0);
        let _node1 = router.borrow().route(runtime.executor(), 1);
        assert_eq!(router.borrow().cache.borrow().len(), 2);

        drop(router);
        runtime.cycle_once();
        assert_eq!(runtime.executor().graph().node_count(), 1); // only push node remaining
    }

    #[test]
    fn test_switch_stream_node_basic() {
        let mut runtime = TestRuntime::new();
        let (left_parent, left_push) = push_node(runtime.executor(), Vec::new());
        let (right_parent, right_push) = push_node(runtime.executor(), Vec::new());
        let (switch_parent, switch_push) = push_node(runtime.executor(), true); // Start with left

        let switch_node = switch_stream_node(
            runtime.executor(),
            left_parent.clone(),
            right_parent.clone(),
            switch_parent.clone(),
        );

        // Initially uses left (true)
        left_push.push_with_cycle(&mut runtime, vec![1, 2, 3]);
        assert_eq!(*switch_node.borrow(), vec![1, 2, 3]);
        assert!(runtime.executor().has_mutated(&switch_node));

        // Right should be ignored while switch is true
        right_push.push_with_cycle(&mut runtime, vec![10, 20]);
        assert_eq!(*switch_node.borrow(), vec![1, 2, 3]); // unchanged, right was ignored
        assert!(!runtime.executor().has_mutated(&switch_node));

        // Switch to right (false)
        switch_push.push_with_cycle(&mut runtime, false);
        assert_eq!(*switch_node.borrow(), vec![10, 20]); // Switch moves data to right

        // Now right should work
        right_push.push_with_cycle(&mut runtime, vec![30, 40]);
        assert_eq!(*switch_node.borrow(), vec![30, 40]);
        assert!(runtime.executor().has_mutated(&switch_node));

        // Left should be ignored while switch is false
        left_push.push_with_cycle(&mut runtime, vec![100, 200]);
        assert_eq!(*switch_node.borrow(), vec![30, 40]); // unchanged, left was ignored
        assert!(!runtime.executor().has_mutated(&switch_node));
    }

    #[test]
    fn test_take_switch_stream_node_basic() {
        let mut runtime = TestRuntime::new();
        let (left_parent, left_push) = push_node(runtime.executor(), Vec::new());
        let (right_parent, right_push) = push_node(runtime.executor(), Vec::new());
        let (switch_parent, switch_push) = push_node(runtime.executor(), true); // Start with left

        let switch_node = take_switch_stream_node(
            runtime.executor(),
            left_parent.clone(),
            right_parent.clone(),
            switch_parent.clone(),
        );

        // Initially uses left (true) and drains it
        left_push.push_with_cycle(&mut runtime, vec![1, 2, 3]);
        assert_eq!(*switch_node.borrow(), vec![1, 2, 3]);
        assert_eq!(*left_parent.borrow(), Vec::<i32>::new()); // Should be drained
        assert!(runtime.executor().has_mutated(&switch_node));

        // Right should be ignored while switch is true
        right_push.push_with_cycle(&mut runtime, vec![10, 20]);
        assert_eq!(*switch_node.borrow(), vec![1, 2, 3]); // unchanged, right was ignored
        assert_eq!(*right_parent.borrow(), vec![10, 20]); // Right should still have data
        assert!(!runtime.executor().has_mutated(&switch_node));

        // Switch to right (false)
        switch_push.push_with_cycle(&mut runtime, false);
        assert_eq!(*switch_node.borrow(), vec![10, 20]); // Switch to right and pull data
        assert!(right_parent.borrow().is_empty());

        // Now right should work and be drained
        right_push.push_with_cycle(&mut runtime, vec![30, 40]);
        assert_eq!(*switch_node.borrow(), vec![30, 40]); // Gets data
        assert_eq!(*right_parent.borrow(), Vec::<i32>::new()); // Should be drained
        assert!(runtime.executor().has_mutated(&switch_node));
    }

    #[test]
    fn test_switch_stream_node_clones_data() {
        let mut runtime = TestRuntime::new();
        let (left_parent, left_push) = push_node(runtime.executor(), Vec::new());
        let (right_parent, right_push) = push_node(runtime.executor(), Vec::new());
        let (switch_parent, switch_push) = push_node(runtime.executor(), true);

        let switch_node = switch_stream_node(
            runtime.executor(),
            left_parent.clone(),
            right_parent.clone(),
            switch_parent.clone(),
        );

        // Data should be cloned, not moved
        left_push.push_with_cycle(&mut runtime, vec![1, 2, 3]);
        assert_eq!(*switch_node.borrow(), vec![1, 2, 3]);
        assert_eq!(*left_parent.borrow(), vec![1, 2, 3]); // Original should remain

        switch_push.push_with_cycle(&mut runtime, false);
        right_push.push_with_cycle(&mut runtime, vec![4, 5]);
        assert_eq!(*switch_node.borrow(), vec![4, 5]);
        assert_eq!(*right_parent.borrow(), vec![4, 5]); // Original should remain
    }

    #[test]
    fn test_switch_during_same_cycle() {
        let mut runtime = TestRuntime::new();
        let (left_parent, left_push) = push_node(runtime.executor(), Vec::new());
        let (right_parent, right_push) = push_node(runtime.executor(), Vec::new());
        let (switch_parent, switch_push) = push_node(runtime.executor(), true);

        let switch_node = switch_stream_node(
            runtime.executor(),
            left_parent.clone(),
            right_parent.clone(),
            switch_parent.clone(),
        );

        // Push data to both sides and change switch in same cycle
        left_push.push(vec![1, 2]);
        right_push.push(vec![10, 20]);
        switch_push.push(false); // Switch to right
        runtime.cycle_once();

        // Should use the new switch state (right)
        assert_eq!(*switch_node.borrow(), vec![10, 20]);
    }

    #[test]
    fn test_both_inputs_mutate_same_cycle() {
        let mut runtime = TestRuntime::new();
        let (left_parent, left_push) = push_node(runtime.executor(), Vec::new());
        let (right_parent, right_push) = push_node(runtime.executor(), Vec::new());
        let (switch_parent, _) = push_node(runtime.executor(), true); // Use left

        let switch_node = switch_stream_node(
            runtime.executor(),
            left_parent.clone(),
            right_parent.clone(),
            switch_parent.clone(),
        );

        // Both inputs get data in same cycle
        left_push.push(vec![1, 2]);
        right_push.push(vec![10, 20]);
        runtime.cycle_once();

        // Should only process the selected input (left)
        assert_eq!(*switch_node.borrow(), vec![1, 2]);
    }

    #[test]
    fn test_empty_inputs() {
        let mut runtime = TestRuntime::new();
        let (left_parent, left_push) = push_node(runtime.executor(), Vec::<i32>::new());
        let (right_parent, right_push) = push_node(runtime.executor(), Vec::new());
        let (switch_parent, _) = push_node(runtime.executor(), true);

        let switch_node = switch_stream_node(
            runtime.executor(),
            left_parent.clone(),
            right_parent.clone(),
            switch_parent.clone(),
        );

        // Empty input should not trigger downstream
        left_push.push_with_cycle(&mut runtime, vec![]);
        assert_eq!(*switch_node.borrow(), Vec::<i32>::new());
        assert!(!runtime.executor().has_mutated(&switch_node));

        right_push.push_with_cycle(&mut runtime, vec![]);
        assert_eq!(*switch_node.borrow(), Vec::<i32>::new());
        assert!(!runtime.executor().has_mutated(&switch_node));
    }

    #[test]
    fn test_switch_multiple_times() {
        let mut runtime = TestRuntime::new();
        let (left_parent, left_push) = push_node(runtime.executor(), Vec::new());
        let (right_parent, right_push) = push_node(runtime.executor(), Vec::new());
        let (switch_parent, switch_push) = push_node(runtime.executor(), true);

        let switch_node = switch_stream_node(
            runtime.executor(),
            left_parent.clone(),
            right_parent.clone(),
            switch_parent.clone(),
        );

        // Start with left
        left_push.push_with_cycle(&mut runtime, vec![1]);
        assert_eq!(*switch_node.borrow(), vec![1]);

        // Switch to right
        switch_push.push_with_cycle(&mut runtime, false);
        right_push.push_with_cycle(&mut runtime, vec![2]);
        assert_eq!(*switch_node.borrow(), vec![2]);

        // Switch back to left
        switch_push.push_with_cycle(&mut runtime, true);
        left_push.push_with_cycle(&mut runtime, vec![3]);
        assert_eq!(*switch_node.borrow(), vec![3]);
    }

    #[test]
    fn test_switch_with_no_initial_value() {
        let mut runtime = TestRuntime::new();
        let (left_parent, left_push) = push_node(runtime.executor(), Vec::new());
        let (right_parent, right_push) = push_node(runtime.executor(), Vec::new());

        // Create switch node without initial value (defaults to false)
        let switch_node_data =
            NodeBuilder::new(true).build(runtime.executor(), |_, _| Control::Unchanged);

        let switch_node = switch_stream_node(
            runtime.executor(),
            left_parent.clone(),
            right_parent.clone(),
            switch_node_data.clone(),
        );

        // Should start with the left (true) since that's the default
        left_push.push_with_cycle(&mut runtime, vec![1, 2]);
        assert_eq!(*switch_node.borrow(), vec![1, 2]);

        right_push.push_with_cycle(&mut runtime, vec![10, 20]);
        assert_eq!(*switch_node.borrow(), vec![1, 2]); // unchanged
    }

    #[test]
    fn test_switch_node_basic() {
        let mut runtime = TestRuntime::new();
        let (primary_parent, primary_push) = push_node(runtime.executor(), 10);
        let (secondary_parent, secondary_push) = push_node(runtime.executor(), 20);
        let (switch_parent, switch_push) = push_node(runtime.executor(), true); // Start with primary

        let switch_node = switch_node(
            runtime.executor(),
            primary_parent.clone(),
            secondary_parent.clone(),
            switch_parent.clone(),
            0, // Initial value
        );

        // Initially should have initial value
        assert_eq!(*switch_node.borrow(), 0);

        // Primary updates should propagate
        primary_push.push_with_cycle(&mut runtime, 100);
        assert_eq!(*switch_node.borrow(), 100);
        assert!(runtime.executor().has_mutated(&switch_node));

        // Secondary updates should be ignored while switch is true
        secondary_push.push_with_cycle(&mut runtime, 200);
        assert_eq!(*switch_node.borrow(), 100); // Unchanged
        assert!(!runtime.executor().has_mutated(&switch_node));

        // Switch to secondary
        switch_push.push_with_cycle(&mut runtime, false);
        assert_eq!(*switch_node.borrow(), 200); // Should pick up secondary's current value
        assert!(runtime.executor().has_mutated(&switch_node));

        // Now secondary updates should propagate
        secondary_push.push_with_cycle(&mut runtime, 300);
        assert_eq!(*switch_node.borrow(), 300);
        assert!(runtime.executor().has_mutated(&switch_node));

        // Primary updates should be ignored while switch is false
        primary_push.push_with_cycle(&mut runtime, 400);
        assert_eq!(*switch_node.borrow(), 300); // Unchanged
        assert!(!runtime.executor().has_mutated(&switch_node));
    }

    #[test]
    fn test_take_switch_node_basic() {
        let mut runtime = TestRuntime::new();
        let (primary_parent, primary_push) = push_node(runtime.executor(), 10);
        let (secondary_parent, secondary_push) = push_node(runtime.executor(), 20);
        let (switch_parent, switch_push) = push_node(runtime.executor(), true); // Start with primary

        let switch_node = take_switch_node(
            runtime.executor(),
            primary_parent.clone(),
            secondary_parent.clone(),
            switch_parent.clone(),
        );

        // Initially should have default value
        assert_eq!(*switch_node.borrow(), 0); // Default for i32

        // Primary updates should propagate and consume
        primary_push.push_with_cycle(&mut runtime, 100);
        assert_eq!(*switch_node.borrow(), 100);
        assert_eq!(*primary_parent.borrow(), 0); // Should be taken (reset to default)
        assert!(runtime.executor().has_mutated(&switch_node));

        // Secondary updates should be ignored while switch is true
        secondary_push.push_with_cycle(&mut runtime, 200);
        assert_eq!(*switch_node.borrow(), 100); // Unchanged
        assert_eq!(*secondary_parent.borrow(), 200); // Should still have data
        assert!(!runtime.executor().has_mutated(&switch_node));

        // Switch to secondary
        switch_push.push_with_cycle(&mut runtime, false);
        assert_eq!(*switch_node.borrow(), 200); // Should take secondary's value
        assert_eq!(*secondary_parent.borrow(), 0); // Should be taken
        assert!(runtime.executor().has_mutated(&switch_node));

        // Now secondary updates should propagate and consume
        secondary_push.push_with_cycle(&mut runtime, 300);
        assert_eq!(*switch_node.borrow(), 300);
        assert_eq!(*secondary_parent.borrow(), 0); // Should be taken
        assert!(runtime.executor().has_mutated(&switch_node));
    }

    #[test]
    fn test_switch_node_clones_data() {
        let mut runtime = TestRuntime::new();
        let (primary_parent, primary_push) = push_node(runtime.executor(), 10);
        let (secondary_parent, _secondary_push) = push_node(runtime.executor(), 20);
        let (switch_parent, _) = push_node(runtime.executor(), true);

        let switch_node = switch_node(
            runtime.executor(),
            primary_parent.clone(),
            secondary_parent.clone(),
            switch_parent.clone(),
            0,
        );

        // Data should be cloned, not moved
        primary_push.push_with_cycle(&mut runtime, 100);
        assert_eq!(*switch_node.borrow(), 100);
        assert_eq!(*primary_parent.borrow(), 100); // Original should remain
    }

    #[test]
    fn test_normal_switch_during_same_cycle() {
        let mut runtime = TestRuntime::new();
        let (primary_parent, primary_push) = push_node(runtime.executor(), 10);
        let (secondary_parent, secondary_push) = push_node(runtime.executor(), 20);
        let (switch_parent, switch_push) = push_node(runtime.executor(), true);

        let switch_node = switch_node(
            runtime.executor(),
            primary_parent.clone(),
            secondary_parent.clone(),
            switch_parent.clone(),
            0,
        );

        // Update both inputs and switch in same cycle
        primary_push.push(100);
        secondary_push.push(200);
        switch_push.push(false); // Switch to secondary
        runtime.cycle_once();

        // Should use the new switch state (secondary)
        assert_eq!(*switch_node.borrow(), 200);
    }

    #[test]
    fn test_both_switch_inputs_mutate_same_cycle() {
        let mut runtime = TestRuntime::new();
        let (primary_parent, primary_push) = push_node(runtime.executor(), 10);
        let (secondary_parent, secondary_push) = push_node(runtime.executor(), 20);
        let (switch_parent, _) = push_node(runtime.executor(), true); // Use primary

        let switch_node = switch_node(
            runtime.executor(),
            primary_parent.clone(),
            secondary_parent.clone(),
            switch_parent.clone(),
            0,
        );

        // Both inputs get data in same cycle
        primary_push.push(100);
        secondary_push.push(200);
        runtime.cycle_once();

        // Should only process the selected input (primary)
        assert_eq!(*switch_node.borrow(), 100);
    }

    #[test]
    fn test_normal_switch_multiple_times() {
        let mut runtime = TestRuntime::new();
        let (primary_parent, primary_push) = push_node(runtime.executor(), 10);
        let (secondary_parent, secondary_push) = push_node(runtime.executor(), 20);
        let (switch_parent, switch_push) = push_node(runtime.executor(), true);

        let switch_node = switch_node(
            runtime.executor(),
            primary_parent.clone(),
            secondary_parent.clone(),
            switch_parent.clone(),
            0,
        );

        // Start with primary
        primary_push.push_with_cycle(&mut runtime, 100);
        assert_eq!(*switch_node.borrow(), 100);

        // Switch to secondary
        switch_push.push_with_cycle(&mut runtime, false);
        assert_eq!(*switch_node.borrow(), 20); // Secondary's current value

        secondary_push.push_with_cycle(&mut runtime, 200);
        assert_eq!(*switch_node.borrow(), 200);

        // Switch back to primary
        switch_push.push_with_cycle(&mut runtime, true);
        assert_eq!(*switch_node.borrow(), 100); // Primary's current value

        primary_push.push_with_cycle(&mut runtime, 300);
        assert_eq!(*switch_node.borrow(), 300);
    }

    #[test]
    fn test_switch_always_broadcasts() {
        let mut runtime = TestRuntime::new();
        let (primary_parent, primary_push) = push_node(runtime.executor(), 10);
        let (secondary_parent, _) = push_node(runtime.executor(), 20);
        let (switch_parent, _) = push_node(runtime.executor(), true);

        let switch_node = switch_node(
            runtime.executor(),
            primary_parent.clone(),
            secondary_parent.clone(),
            switch_parent.clone(),
            0,
        );

        // Any input change should broadcast (even if value doesn't change)
        primary_push.push_with_cycle(&mut runtime, 10); // Same as source's current value
        assert!(runtime.executor().has_mutated(&switch_node)); // Should still broadcast
    }

    #[test]
    fn test_take_switch_node_with_structs() {
        #[derive(Debug, Default, PartialEq, Clone)]
        struct TestData {
            id: u32,
            value: String,
        }

        let mut runtime = TestRuntime::new();
        let (primary_parent, primary_push) = push_node(
            runtime.executor(),
            TestData {
                id: 1,
                value: "primary".to_string(),
            },
        );
        let (secondary_parent, _secondary_push) = push_node(
            runtime.executor(),
            TestData {
                id: 2,
                value: "secondary".to_string(),
            },
        );
        let (switch_parent, switch_push) = push_node(runtime.executor(), true);

        let switch_node = take_switch_node(
            runtime.executor(),
            primary_parent.clone(),
            secondary_parent.clone(),
            switch_parent.clone(),
        );

        // Should start with default
        assert_eq!(*switch_node.borrow(), TestData::default());

        // Primary update should be taken
        let new_primary = TestData {
            id: 10,
            value: "updated_primary".to_string(),
        };
        primary_push.push_with_cycle(&mut runtime, new_primary.clone());
        assert_eq!(*switch_node.borrow(), new_primary);
        assert_eq!(*primary_parent.borrow(), TestData::default()); // Should be taken

        // Switch to secondary
        switch_push.push_with_cycle(&mut runtime, false);
        let expected_secondary = TestData {
            id: 2,
            value: "secondary".to_string(),
        };
        assert_eq!(*switch_node.borrow(), expected_secondary);
        assert_eq!(*secondary_parent.borrow(), TestData::default()); // Should be taken
    }

    #[test]
    fn test_switch_node_with_custom_initial() {
        let mut runtime = TestRuntime::new();
        let (primary_parent, _) = push_node(runtime.executor(), 10);
        let (secondary_parent, _) = push_node(runtime.executor(), 20);
        let (switch_parent, _) = push_node(runtime.executor(), true);

        let switch_node = switch_node(
            runtime.executor(),
            primary_parent.clone(),
            secondary_parent.clone(),
            switch_parent.clone(),
            999, // Custom initial value
        );

        // Should start with custom initial value
        assert_eq!(*switch_node.borrow(), 999);
    }
}

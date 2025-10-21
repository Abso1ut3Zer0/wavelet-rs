use crate::Control;
use crate::prelude::{Executor, Node, NodeBuilder};
pub fn and_node(executor: &mut Executor, left: Node<bool>, right: Node<bool>) -> Node<bool> {
    and_node_with_init(executor, left, right, false)
}
pub fn and_node_with_init(
    executor: &mut Executor,
    left: Node<bool>,
    right: Node<bool>,
    init: bool,
) -> Node<bool> {
    NodeBuilder::new(init)
        .triggered_by(&left)
        .triggered_by(&right)
        .build(executor, move |this, _| {
            let new_value = *left.borrow() && *right.borrow();
            let is_different = new_value ^ *this;
            *this = new_value;
            Control::from(is_different)
        })
}
pub fn and_node_with_bootstrap(
    executor: &mut Executor,
    left: Node<bool>,
    right: Node<bool>,
) -> Node<bool> {
    NodeBuilder::new(false)
        .triggered_by(&left)
        .triggered_by(&right)
        .on_init(|ex, _, current| {
            ex.yield_driver().yield_now(current);
        })
        .build(executor, move |this, _| {
            let new_value = *left.borrow() && *right.borrow();
            let is_different = new_value ^ *this;
            *this = new_value;
            Control::from(is_different)
        })
}
pub fn or_node(executor: &mut Executor, left: Node<bool>, right: Node<bool>) -> Node<bool> {
    or_node_with_init(executor, left, right, false)
}
pub fn or_node_with_init(
    executor: &mut Executor,
    left: Node<bool>,
    right: Node<bool>,
    init: bool,
) -> Node<bool> {
    NodeBuilder::new(init)
        .triggered_by(&left)
        .triggered_by(&right)
        .build(executor, move |this, _| {
            let new_value = *left.borrow() || *right.borrow();
            let is_different = new_value ^ *this;
            *this = new_value;
            Control::from(is_different)
        })
}
pub fn or_node_with_bootstrap(
    executor: &mut Executor,
    left: Node<bool>,
    right: Node<bool>,
) -> Node<bool> {
    NodeBuilder::new(false)
        .triggered_by(&left)
        .triggered_by(&right)
        .on_init(|ex, _, current| {
            ex.yield_driver().yield_now(current);
        })
        .build(executor, move |this, _| {
            let new_value = *left.borrow() || *right.borrow();
            let is_different = new_value ^ *this;
            *this = new_value;
            Control::from(is_different)
        })
}
pub fn xor_node(executor: &mut Executor, left: Node<bool>, right: Node<bool>) -> Node<bool> {
    xor_node_with_init(executor, left, right, false)
}
pub fn xor_node_with_init(
    executor: &mut Executor,
    left: Node<bool>,
    right: Node<bool>,
    init: bool,
) -> Node<bool> {
    NodeBuilder::new(init)
        .triggered_by(&left)
        .triggered_by(&right)
        .build(executor, move |this, _| {
            let new_value = *left.borrow() ^ *right.borrow();
            let is_different = new_value ^ *this;
            *this = new_value;
            Control::from(is_different)
        })
}
pub fn xor_node_with_bootstrap(
    executor: &mut Executor,
    left: Node<bool>,
    right: Node<bool>,
) -> Node<bool> {
    NodeBuilder::new(false)
        .triggered_by(&left)
        .triggered_by(&right)
        .on_init(|ex, _, current| {
            ex.yield_driver().yield_now(current);
        })
        .build(executor, move |this, _| {
            let new_value = *left.borrow() ^ *right.borrow();
            let is_different = new_value ^ *this;
            *this = new_value;
            Control::from(is_different)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::TestRuntime;
    use crate::prelude::*;
    use crate::testing::push_node;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn test_and_node_basic() {
        let mut runtime = TestRuntime::new();

        // Create push nodes for inputs - returns (Node<T>, Push<T>)
        let (left_node, left_push) = push_node(runtime.executor(), false);
        let (right_node, right_push) = push_node(runtime.executor(), false);

        // Create the AND node
        let and = and_node(runtime.executor(), left_node, right_node);

        // Test all combinations
        // false AND false = false
        assert_eq!(*and.borrow(), false);

        // true AND false = false
        left_push.push(true);
        runtime.run_one_cycle();
        assert_eq!(*and.borrow(), false);

        // true AND true = true
        right_push.push(true);
        runtime.run_one_cycle();
        assert_eq!(*and.borrow(), true);

        // false AND true = false
        left_push.push(false);
        runtime.run_one_cycle();
        assert_eq!(*and.borrow(), false);

        // false AND false = false
        right_push.push(false);
        runtime.run_one_cycle();
        assert_eq!(*and.borrow(), false);
    }

    #[test]
    fn test_and_node_with_init() {
        let mut runtime = TestRuntime::new();

        let (left_node, left_push) = push_node(runtime.executor(), true);
        let (right_node, _right_push) = push_node(runtime.executor(), true);

        // Test with init = true
        let and = and_node_with_init(
            runtime.executor(),
            left_node.clone(),
            right_node.clone(),
            true,
        );

        // Should start with true regardless of inputs initially not being processed
        assert_eq!(*and.borrow(), true);

        // After cycle, should reflect actual AND logic
        left_push.push(true);
        runtime.run_one_cycle();
        assert_eq!(*and.borrow(), true); // true AND true = true

        // Test with init = false but true inputs
        let and2 = and_node_with_init(runtime.executor(), left_node, right_node, false);
        assert_eq!(*and2.borrow(), false); // Starts with false

        left_push.push(true);
        runtime.run_one_cycle();
        assert_eq!(*and2.borrow(), true); // Updates to true AND true = true
    }

    #[test]
    fn test_and_node_with_bootstrap() {
        let mut runtime = TestRuntime::new();

        let (left_node, _left_push) = push_node(runtime.executor(), true);
        let (right_node, _right_push) = push_node(runtime.executor(), true);

        let and = and_node_with_bootstrap(runtime.executor(), left_node, right_node);

        // Initially false
        assert_eq!(*and.borrow(), false);

        // Bootstrap should schedule it for evaluation
        runtime.run_one_cycle();
        assert_eq!(*and.borrow(), true); // true AND true = true
    }

    #[test]
    fn test_or_node_basic() {
        let mut runtime = TestRuntime::new();

        let (left_node, left_push) = push_node(runtime.executor(), false);
        let (right_node, right_push) = push_node(runtime.executor(), false);

        let or = or_node(runtime.executor(), left_node, right_node);

        // false OR false = false
        assert_eq!(*or.borrow(), false);

        // true OR false = true
        left_push.push(true);
        runtime.run_one_cycle();
        assert_eq!(*or.borrow(), true);

        // true OR true = true
        right_push.push(true);
        runtime.run_one_cycle();
        assert_eq!(*or.borrow(), true);

        // false OR true = true
        left_push.push(false);
        runtime.run_one_cycle();
        assert_eq!(*or.borrow(), true);

        // false OR false = false
        right_push.push(false);
        runtime.run_one_cycle();
        assert_eq!(*or.borrow(), false);
    }

    #[test]
    fn test_or_node_with_init() {
        let mut runtime = TestRuntime::new();

        let (left_node, left_push) = push_node(runtime.executor(), false);
        let (right_node, _right_push) = push_node(runtime.executor(), false);

        // Test with init = true
        let or = or_node_with_init(
            runtime.executor(),
            left_node.clone(),
            right_node.clone(),
            true,
        );
        assert_eq!(*or.borrow(), true); // Starts with true

        left_push.push(false);
        runtime.run_one_cycle();
        assert_eq!(*or.borrow(), false); // false OR false = false

        // Test with init = false
        let or2 = or_node_with_init(runtime.executor(), left_node, right_node, false);
        assert_eq!(*or2.borrow(), false); // Starts with false
    }

    #[test]
    fn test_or_node_with_bootstrap() {
        let mut runtime = TestRuntime::new();

        let (left_node, _left_push) = push_node(runtime.executor(), false);
        let (right_node, _right_push) = push_node(runtime.executor(), true);

        let or = or_node_with_bootstrap(runtime.executor(), left_node, right_node);

        // Initially false
        assert_eq!(*or.borrow(), false);

        // Bootstrap triggers evaluation
        runtime.run_one_cycle();
        assert_eq!(*or.borrow(), true); // false OR true = true
    }

    #[test]
    fn test_xor_node_basic() {
        let mut runtime = TestRuntime::new();

        let (left_node, left_push) = push_node(runtime.executor(), false);
        let (right_node, right_push) = push_node(runtime.executor(), false);

        let xor = xor_node(runtime.executor(), left_node, right_node);

        // false XOR false = false
        assert_eq!(*xor.borrow(), false);

        // true XOR false = true
        left_push.push(true);
        runtime.run_one_cycle();
        assert_eq!(*xor.borrow(), true);

        // true XOR true = false
        right_push.push(true);
        runtime.run_one_cycle();
        assert_eq!(*xor.borrow(), false);

        // false XOR true = true
        left_push.push(false);
        runtime.run_one_cycle();
        assert_eq!(*xor.borrow(), true);

        // false XOR false = false
        right_push.push(false);
        runtime.run_one_cycle();
        assert_eq!(*xor.borrow(), false);
    }

    #[test]
    fn test_xor_node_with_init() {
        let mut runtime = TestRuntime::new();

        let (left_node, _left_push) = push_node(runtime.executor(), true);
        let (right_node, right_push) = push_node(runtime.executor(), false);

        // Test with init = true
        let xor = xor_node_with_init(
            runtime.executor(),
            left_node.clone(),
            right_node.clone(),
            true,
        );
        assert_eq!(*xor.borrow(), true); // Starts with true

        runtime.run_one_cycle();
        assert_eq!(*xor.borrow(), true); // true XOR false = true

        // Test with init = false
        let xor2 = xor_node_with_init(runtime.executor(), left_node, right_node, false);
        assert_eq!(*xor2.borrow(), false); // Starts with false

        right_push.push(false);
        runtime.run_one_cycle();
        assert_eq!(*xor2.borrow(), true); // true XOR false = true
    }

    #[test]
    fn test_xor_node_with_bootstrap() {
        let mut runtime = TestRuntime::new();

        let (left_node, _left_push) = push_node(runtime.executor(), true);
        let (right_node, _right_push) = push_node(runtime.executor(), true);

        let xor = xor_node_with_bootstrap(runtime.executor(), left_node, right_node);

        // Initially false
        assert_eq!(*xor.borrow(), false);

        // Bootstrap triggers evaluation
        runtime.run_one_cycle();
        assert_eq!(*xor.borrow(), false); // true XOR true = false
    }

    #[test]
    fn test_propagation_only_on_change() {
        let mut runtime = TestRuntime::new();

        let (left_node, left_push) = push_node(runtime.executor(), true);
        let (right_node, right_push) = push_node(runtime.executor(), false);

        let and = and_node(runtime.executor(), left_node, right_node);

        // Create a child node that counts how many times it's triggered
        let trigger_count = Rc::new(RefCell::new(0));
        let cloned = trigger_count.clone();
        let counter =
            NodeBuilder::new(0)
                .triggered_by(&and)
                .build(runtime.executor(), move |this, _| {
                    *cloned.borrow_mut() += 1;
                    *this += 1;
                    Control::Broadcast
                });

        // Initial state
        assert_eq!(*and.borrow(), false);
        assert_eq!(*counter.borrow(), 0);
        assert_eq!(*trigger_count.borrow(), 0);

        // Change that affects output (false -> false, no change)
        right_push.push(false);
        runtime.run_one_cycle();
        assert_eq!(*and.borrow(), false);
        assert_eq!(*trigger_count.borrow(), 0); // Should not trigger

        // Change that affects output (false -> true)
        right_push.push(true);
        runtime.run_one_cycle();
        assert_eq!(*and.borrow(), true);
        assert_eq!(*trigger_count.borrow(), 1); // Should trigger

        // Change that doesn't affect output (true -> true)
        left_push.push(true);
        right_push.push(true);
        runtime.run_one_cycle();
        assert_eq!(*and.borrow(), true);
        assert_eq!(*trigger_count.borrow(), 1); // Should not trigger again
    }

    #[test]
    fn test_complex_logic_chain() {
        let mut runtime = TestRuntime::new();

        // Create a complex logic chain: (A AND B) OR (C XOR D)
        let (a_node, a_push) = push_node(runtime.executor(), true);
        let (b_node, b_push) = push_node(runtime.executor(), false);
        let (c_node, c_push) = push_node(runtime.executor(), true);
        let (d_node, d_push) = push_node(runtime.executor(), true);

        let and_ab = and_node(runtime.executor(), a_node, b_node);
        let xor_cd = xor_node(runtime.executor(), c_node, d_node);
        let final_or = or_node(runtime.executor(), and_ab, xor_cd);

        // Initial: (true AND false) OR (true XOR true) = false OR false = false
        runtime.run_one_cycle();
        assert_eq!(*final_or.borrow(), false);

        // Change B to true: (true AND true) OR (true XOR true) = true OR false = true
        b_push.push(true);
        runtime.run_one_cycle();
        assert_eq!(*final_or.borrow(), true);

        // Change D to false: (true AND true) OR (true XOR false) = true OR true = true
        d_push.push(false);
        runtime.run_one_cycle();
        assert_eq!(*final_or.borrow(), true);

        // Change A to false and B to false: (false AND false) OR (true XOR false) = false OR true = true
        a_push.push(false);
        b_push.push(false);
        runtime.run_one_cycle();
        assert_eq!(*final_or.borrow(), true);

        // Change C to false: (false AND false) OR (false XOR false) = false OR false = false
        c_push.push(false);
        runtime.run_one_cycle();
        assert_eq!(*final_or.borrow(), false);
    }

    #[test]
    fn test_bootstrap_scheduling() {
        let mut runtime = TestRuntime::new();

        let (left_node, _left_push) = push_node(runtime.executor(), true);
        let (right_node, _right_push) = push_node(runtime.executor(), true);

        // Create multiple bootstrapped nodes
        let and =
            and_node_with_bootstrap(runtime.executor(), left_node.clone(), right_node.clone());
        let or = or_node_with_bootstrap(runtime.executor(), left_node.clone(), right_node.clone());
        let xor = xor_node_with_bootstrap(runtime.executor(), left_node, right_node);

        // All should start at false
        assert_eq!(*and.borrow(), false);
        assert_eq!(*or.borrow(), false);
        assert_eq!(*xor.borrow(), false);

        // After one cycle, all should be evaluated
        runtime.run_one_cycle();
        assert_eq!(*and.borrow(), true); // true AND true = true
        assert_eq!(*or.borrow(), true); // true OR true = true
        assert_eq!(*xor.borrow(), false); // true XOR true = false
    }
}

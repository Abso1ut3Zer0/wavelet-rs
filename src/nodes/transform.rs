use crate::Control;
use crate::prelude::Node;
use crate::runtime::{Executor, NodeBuilder};
use ahash::{HashSet, HashSetExt};
use std::hash::Hash;

pub fn accumulate_stream_node<T, A>(
    executor: &mut Executor,
    parent: Node<Vec<T>>,
    initial: A,
    fold_fn: impl Fn(&mut A, &T) + 'static,
) -> Node<A> {
    NodeBuilder::new(initial)
        .triggered_by(&parent)
        .build(executor, move |this, _| {
            parent.borrow().iter().for_each(|item| fold_fn(this, item));
            Control::Broadcast
        })
}

pub fn accumulate_stream_with_reset_node<T, A: Default>(
    executor: &mut Executor,
    parent: Node<Vec<T>>,
    fold_fn: impl Fn(&mut A, &T) + 'static,
) -> Node<A> {
    NodeBuilder::new(A::default())
        .triggered_by(&parent)
        .build(executor, move |this, _| {
            *this = A::default();
            parent.borrow().iter().for_each(|item| fold_fn(this, item));
            Control::Broadcast
        })
}

pub fn deduplicate_stream_node<K: Eq + Hash + 'static, T: Clone>(
    executor: &mut Executor,
    parent: Node<Vec<T>>,
    key_fn: impl Fn(&T) -> K + 'static,
) -> Node<Vec<T>> {
    let mut seen = HashSet::new();
    NodeBuilder::new(Vec::new())
        .triggered_by(&parent)
        .build(executor, move |this, _| {
            seen.clear();
            this.clear();
            for item in parent.borrow().as_slice() {
                let key = key_fn(item);
                if !seen.contains(&key) {
                    seen.insert(key);
                    this.push(item.clone());
                }
            }

            Control::from(!this.is_empty())
        })
}

pub fn filter_stream_node<T: Clone>(
    executor: &mut Executor,
    parent: Node<Vec<T>>,
    predicate: impl Fn(&T) -> bool + 'static,
) -> Node<Vec<T>> {
    NodeBuilder::new(Vec::new())
        .triggered_by(&parent)
        .build(executor, move |this, _| {
            this.clear();
            this.extend(
                parent
                    .borrow()
                    .iter()
                    .filter(|item| predicate(item))
                    .cloned(),
            );

            Control::from(!this.is_empty())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::*;
    use crate::testing::push_node;

    #[test]
    fn test_accumulator_node() {
        let mut runtime = TestRuntime::new();
        let (parent, push) = push_node(runtime.executor(), Vec::new());

        // Running sum accumulator
        let sum_node =
            accumulate_stream_node(runtime.executor(), parent.clone(), 0, |acc, &item| {
                *acc += item
            });

        // First cycle: sum 1 + 2 + 3 = 6
        push.push_with_cycle(&mut runtime, vec![1, 2, 3]);
        assert_eq!(*sum_node.borrow(), 6);

        // Second cycle: add 4 + 5 to existing sum = 15
        push.push_with_cycle(&mut runtime, vec![4, 5]);
        assert_eq!(*sum_node.borrow(), 15);

        // Third cycle: add nothing, sum stays the same
        push.push_with_cycle(&mut runtime, vec![]);
        assert_eq!(*sum_node.borrow(), 15);
    }

    #[test]
    fn test_accumulator_with_reset_node() {
        let mut runtime = TestRuntime::new();
        let (parent, push) = push_node(runtime.executor(), Vec::new());

        // Per-cycle sum accumulator
        let sum_node = accumulate_stream_with_reset_node(
            runtime.executor(),
            parent.clone(),
            |acc: &mut i32, &item| *acc += item,
        );

        // First cycle: sum 1 + 2 + 3 = 6
        push.push_with_cycle(&mut runtime, vec![1, 2, 3]);
        assert_eq!(*sum_node.borrow(), 6);

        // Second cycle: reset and sum 4 + 5 = 9
        push.push_with_cycle(&mut runtime, vec![4, 5]);
        assert_eq!(*sum_node.borrow(), 9);

        // Third cycle: reset and sum nothing = 0
        push.push_with_cycle(&mut runtime, vec![]);
        assert_eq!(*sum_node.borrow(), 0);
    }

    #[test]
    fn test_accumulator_complex_state() {
        let mut runtime = TestRuntime::new();
        let (parent, push) = push_node(runtime.executor(), Vec::new());

        // Count and sum accumulator
        let stats_node = accumulate_stream_node(
            runtime.executor(),
            parent.clone(),
            (0, 0), // (count, sum)
            |acc: &mut (i32, i32), &item| {
                acc.0 += 1; // increment count
                acc.1 += item; // add to sum
            },
        );

        push.push_with_cycle(&mut runtime, vec![10, 20]);
        assert_eq!(*stats_node.borrow(), (2, 30));

        push.push_with_cycle(&mut runtime, vec![5]);
        assert_eq!(*stats_node.borrow(), (3, 35));
    }

    #[test]
    fn test_deduplicate_node() {
        let mut runtime = TestRuntime::new();
        let (parent, push) = push_node(runtime.executor(), Vec::new());

        // Deduplicate by value (identity function)
        let dedup_node = deduplicate_stream_node(runtime.executor(), parent.clone(), |&item| item);

        // First cycle: [1, 2, 2, 3, 1] -> [1, 2, 3]
        push.push_with_cycle(&mut runtime, vec![1, 2, 2, 3, 1]);
        assert_eq!(*dedup_node.borrow(), vec![1, 2, 3]);

        // Second cycle: [3, 4, 4, 5] -> [3, 4, 5] (fresh dedup each cycle)
        push.push_with_cycle(&mut runtime, vec![3, 4, 4, 5]);
        assert_eq!(*dedup_node.borrow(), vec![3, 4, 5]);

        // Third cycle: empty input -> empty output
        push.push_with_cycle(&mut runtime, vec![]);
        assert_eq!(*dedup_node.borrow(), Vec::<i32>::new());
    }

    #[test]
    fn test_deduplicate_by_key() {
        #[derive(Clone, Debug, PartialEq)]
        struct Trade {
            id: u32,
            symbol: String,
            price: f64,
        }

        let mut runtime = TestRuntime::new();
        let (parent, push) = push_node(runtime.executor(), Vec::new());

        // Deduplicate by symbol (first trade per symbol wins)
        let dedup_node =
            deduplicate_stream_node(runtime.executor(), parent.clone(), |trade: &Trade| {
                trade.symbol.clone()
            });

        let trades = vec![
            Trade {
                id: 1,
                symbol: "AAPL".to_string(),
                price: 150.0,
            },
            Trade {
                id: 2,
                symbol: "GOOGL".to_string(),
                price: 2800.0,
            },
            Trade {
                id: 3,
                symbol: "AAPL".to_string(),
                price: 151.0,
            }, // Duplicate symbol
            Trade {
                id: 4,
                symbol: "MSFT".to_string(),
                price: 300.0,
            },
        ];

        push.push_with_cycle(&mut runtime, trades);

        let result = dedup_node.borrow();
        assert_eq!(result.len(), 3); // Only 3 unique symbols
        assert_eq!(result[0].symbol, "AAPL");
        assert_eq!(result[0].price, 150.0); // First AAPL trade wins
        assert_eq!(result[1].symbol, "GOOGL");
        assert_eq!(result[2].symbol, "MSFT");
    }

    #[test]
    fn test_no_mutations_on_empty() {
        let mut runtime = TestRuntime::new();
        let (parent, push) = push_node(runtime.executor(), Vec::new());

        let dedup_node = deduplicate_stream_node(runtime.executor(), parent.clone(), |&item| item);

        // Empty input should not trigger downstream (Control::Unchanged)
        push.push_with_cycle(&mut runtime, vec![]);
        assert!(!runtime.executor().has_mutated(&dedup_node));

        // Non-empty input should trigger downstream
        push.push_with_cycle(&mut runtime, vec![1]);
        assert!(runtime.executor().has_mutated(&dedup_node));
    }

    #[test]
    fn test_accumulator_always_broadcasts() {
        let mut runtime = TestRuntime::new();
        let (parent, push) = push_node(runtime.executor(), Vec::new());

        let sum_node =
            accumulate_stream_node(runtime.executor(), parent.clone(), 0, |acc, &item| {
                *acc += item
            });

        // Even empty input should trigger downstream (Control::Broadcast)
        push.push_with_cycle(&mut runtime, vec![]);
        assert!(runtime.executor().has_mutated(&sum_node));

        push.push_with_cycle(&mut runtime, vec![1, 2]);
        assert!(runtime.executor().has_mutated(&sum_node));
    }

    #[test]
    fn test_filter_node() {
        let mut runtime = TestRuntime::new();
        let (parent, push) = push_node(runtime.executor(), Vec::new());

        // Filter for even numbers
        let even_node = filter_stream_node(runtime.executor(), parent.clone(), |&x| x % 2 == 0);

        // First cycle: [1, 2, 3, 4, 5] -> [2, 4]
        push.push_with_cycle(&mut runtime, vec![1, 2, 3, 4, 5]);
        assert_eq!(*even_node.borrow(), vec![2, 4]);
        assert!(runtime.executor().has_mutated(&even_node));

        // Second cycle: [6, 7, 8] -> [6, 8]
        push.push_with_cycle(&mut runtime, vec![6, 7, 8]);
        assert_eq!(*even_node.borrow(), vec![6, 8]);
        assert!(runtime.executor().has_mutated(&even_node));

        // Third cycle: [1, 3, 5] -> [] (no evens, no mutation)
        push.push_with_cycle(&mut runtime, vec![1, 3, 5]);
        assert_eq!(*even_node.borrow(), Vec::<i32>::new());
        assert!(!runtime.executor().has_mutated(&even_node));

        // Fourth cycle: empty input -> [] (no mutation)
        push.push_with_cycle(&mut runtime, vec![]);
        assert_eq!(*even_node.borrow(), Vec::<i32>::new());
        assert!(!runtime.executor().has_mutated(&even_node));
    }

    #[test]
    fn test_filter_node_complex_predicate() {
        #[derive(Clone, Debug, PartialEq)]
        struct Order {
            id: u32,
            quantity: u32,
            price: f64,
        }

        let mut runtime = TestRuntime::new();
        let (parent, push) = push_node(runtime.executor(), Vec::new());

        // Filter for large orders (quantity > 100)
        let large_orders =
            filter_stream_node(runtime.executor(), parent.clone(), |order: &Order| {
                order.quantity > 100
            });

        let orders = vec![
            Order {
                id: 1,
                quantity: 50,
                price: 100.0,
            }, // Small
            Order {
                id: 2,
                quantity: 150,
                price: 101.0,
            }, // Large
            Order {
                id: 3,
                quantity: 200,
                price: 99.0,
            }, // Large
            Order {
                id: 4,
                quantity: 75,
                price: 102.0,
            }, // Small
        ];

        push.push_with_cycle(&mut runtime, orders);

        let result = large_orders.borrow();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].id, 2);
        assert_eq!(result[0].quantity, 150);
        assert_eq!(result[1].id, 3);
        assert_eq!(result[1].quantity, 200);
    }

    #[test]
    fn test_filter_preserves_order() {
        let mut runtime = TestRuntime::new();
        let (parent, push) = push_node(runtime.executor(), Vec::new());

        let filtered = filter_stream_node(runtime.executor(), parent.clone(), |&x| x > 5);

        // Order should be preserved in filtered output
        push.push_with_cycle(&mut runtime, vec![10, 3, 8, 1, 6, 2, 9]);
        assert_eq!(*filtered.borrow(), vec![10, 8, 6, 9]);
    }
}

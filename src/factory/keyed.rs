use crate::runtime::{Executor, Node};
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::hash::Hash;
use std::rc::Rc;

/// A dependency injection factory that creates and caches nodes based on keys.
///
/// `KeyedFactory<K, T>` provides a build-time configuration mechanism for creating
/// nodes with different implementations while maintaining the same graph topology.
/// Each unique key gets its own cached node instance, enabling efficient reuse
/// of expensive subgraph construction.
///
/// # Dependency Injection Pattern
///
/// The factory separates "what to create" (the factory function) from "when to create"
/// (the graph execution). This enables:
/// - **Environment-specific implementations**: Different factories for dev/staging/prod
/// - **Testing flexibility**: Mock implementations using identical graph structure
/// - **Configuration-driven graphs**: Build different node types based on runtime config
/// - **Resource sharing**: Expensive nodes (database connections, etc.) created once per key
///
/// # Example: Market Data Factory
///
/// ```rust, ignore
/// // Build-time: Configure different data sources
/// let data_factory = match config.environment {
///     Environment::Live => KeyedFactory::default()
///         .attach(|executor, symbol: &String| {
///             create_live_market_feed(executor, symbol)
///         }),
///     Environment::Replay => KeyedFactory::default()
///         .attach(|executor, symbol: &String| {
///             create_historical_replay(executor, symbol, config.date_range)
///         }),
///     Environment::Test => KeyedFactory::default()
///         .attach(|executor, symbol: &String| {
///             create_mock_data_generator(executor, symbol)
///         }),
/// };
///
/// // Runtime: Same graph construction, different implementations
/// let eurusd_feed = data_factory.get(&mut executor, "EURUSD".to_string());
/// let gbpusd_feed = data_factory.get(&mut executor, "GBPUSD".to_string());
///
/// let processor = NodeBuilder::new(PriceProcessor::new())
///     .triggered_by(&eurusd_feed)  // Uses configured implementation
///     .build(&mut executor, process_logic);
/// ```
///
/// # Lifecycle
///
/// 1. **Attach**: Configure the factory function at build time
/// 2. **Get**: Request nodes by key during graph construction
/// 3. **Cache**: First request creates and caches the node
/// 4. **Reuse**: Subsequent requests for the same key return the cached node
///
/// # Thread Safety
///
/// Uses `Rc<RefCell<>>` for shared ownership within the single-threaded runtime.
/// The factory is designed for build-time use and should not be accessed during
/// node execution cycles.
#[derive(Clone)]
pub struct KeyedFactory<'a, K: Hash + Eq, T: 'static>(Rc<RefCell<KeyedFactoryInner<'a, K, T>>>);

impl<'a, K: Hash + Eq, T: 'static> Default for KeyedFactory<'a, K, T> {
    fn default() -> Self {
        Self(Rc::new(RefCell::new(KeyedFactoryInner::default())))
    }
}

impl<'a, K: Hash + Eq, T: 'static> KeyedFactory<'a, K, T> {
    /// Configures the factory function that creates nodes for each key.
    ///
    /// The factory function receives:
    /// - `executor`: Full access to the runtime for node creation
    /// - `key`: Reference to the key for customizing the created node
    ///
    /// # Panics
    /// Panics if a factory function has already been attached.
    ///
    /// # Example
    /// ```rust, ignore
    /// use wavelet::prelude::*;
    /// let factory = KeyedFactory::default()
    ///     .attach(|executor, db_name: &String| {
    ///         NodeBuilder::new(DatabaseConnection::new(db_name))
    ///             .with_name(format!("db_{}", db_name))
    ///             .build(executor, handle_queries)
    ///     });
    /// ```
    pub fn attach(self, f: impl FnMut(&mut Executor, &K) -> Node<T> + 'a) -> Self {
        {
            let mut inner = self.0.borrow_mut();
            inner.attach(f);
        }
        self
    }

    /// Creates or retrieves a cached node for the specified key.
    ///
    /// If this is the first request for the key, the factory function will be
    /// called to create a new node. Otherwise, the cached node is returned.
    ///
    /// # Panics
    /// Panics if no factory function has been attached.
    ///
    /// # Example
    /// ```rust, ignore
    /// let conn1 = db_factory.get(&mut executor, "users".to_string());
    /// let conn2 = db_factory.get(&mut executor, "users".to_string());
    /// // conn1 and conn2 are the same node instance
    /// ```
    pub fn get(&self, executor: &mut Executor, key: K) -> Node<T> {
        self.0.borrow_mut().get(executor, key)
    }
}

struct KeyedFactoryInner<'a, K: Hash + Eq, T: 'static> {
    cache: HashMap<K, Node<T>>,
    factory: Option<Box<dyn FnMut(&mut Executor, &K) -> Node<T> + 'a>>,
}

impl<'a, K: Hash + Eq, T: 'static> Default for KeyedFactoryInner<'a, K, T> {
    fn default() -> Self {
        Self {
            cache: HashMap::new(),
            factory: None,
        }
    }
}

impl<'a, K: Hash + Eq, T: 'static> KeyedFactoryInner<'a, K, T> {
    fn attach(&mut self, f: impl FnMut(&mut Executor, &K) -> Node<T> + 'a) {
        assert!(self.factory.is_none(), "factory already attached");
        self.factory = Some(Box::new(f));
    }

    fn get(&mut self, executor: &mut Executor, key: K) -> Node<T> {
        let mut entry = self.cache.entry(key);
        match entry {
            Entry::Occupied(node) => node.get().to_owned(),
            Entry::Vacant(entry) => {
                let factory = self.factory.as_mut().expect("factory not attached");
                let node = factory(executor, entry.key());
                entry.insert(node).to_owned()
            }
        }
    }
}

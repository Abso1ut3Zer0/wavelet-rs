use crate::runtime::{Executor, Node};
use std::cell::{OnceCell, RefCell};
use std::rc::Rc;

/// A dependency injection factory for creating single-instance wsnl with lazy initialization.
///
/// `SimpleFactory<T>` provides build-time configuration for wsnl that should only
/// be created once per factory instance. Uses `OnceCell` for thread-safe lazy
/// initialization - the factory function is called exactly once on the first
/// `get()` request, and subsequent calls return the cached node.
///
/// # Use Cases
///
/// - **Singleton services**: Database connections, configuration managers, loggers
/// - **Expensive initialization**: Nodes that load large datasets or establish connections
/// - **Shared resources**: Nodes that multiple parts of the graph need to reference
/// - **Environment abstraction**: Different implementations based on build-time configuration
///
/// # Example: Configuration Service
///
/// ```rust, ignore
/// // Build-time: Configure different config sources
/// let config_factory = match environment {
///     Environment::Production => SimpleFactory::default()
///         .attach(|executor| {
///             NodeBuilder::new(ProductionConfig::load())
///                 .with_name("prod_config".to_string())
///                 .build(executor, |config, ctx| {
///                     // Handle config reload requests
///                     Control::Broadcast
///                 })
///         }),
///     Environment::Test => SimpleFactory::default()
///         .attach(|executor| {
///             NodeBuilder::new(MockConfig::default())
///                 .with_name("test_config".to_string())
///                 .build(executor, |config, ctx| Control::Unchanged)
///         }),
/// };
///
/// // Runtime: Multiple wsnl can depend on the same config instance
/// let config_node = config_factory.get(&mut executor);
///
/// let processor1 = NodeBuilder::new(DataProcessor::new())
///     .observer_of(&config_node)  // Read config but don't react to changes
///     .build(&mut executor, process_data);
///
/// let processor2 = NodeBuilder::new(AnotherProcessor::new())
///     .observer_of(&config_node)  // Same config instance
///     .build(&mut executor, process_other_data);
/// ```
///
/// # Lifecycle
///
/// 1. **Attach**: Configure the factory function at build time
/// 2. **Get**: Request the node during graph construction
/// 3. **Initialize**: First `get()` call executes the factory function
/// 4. **Reuse**: All subsequent `get()` calls return the same node instance
///
/// # Comparison with KeyedFactory
///
/// - `SimpleFactory`: One node per factory instance (singleton pattern)
/// - `KeyedFactory`: One node per unique key (multi-instance with caching)
///
/// Choose `SimpleFactory` when you need exactly one instance of a particular
/// node type in your graph.
#[derive(Clone)]
pub struct SimpleFactory<'a, T: 'static>(Rc<RefCell<SimpleFactoryInner<'a, T>>>);

impl<'a, T: 'static> Default for SimpleFactory<'a, T> {
    fn default() -> Self {
        Self(Rc::new(RefCell::new(SimpleFactoryInner::default())))
    }
}

impl<'a, T: 'static> SimpleFactory<'a, T> {
    /// Configures the factory function that creates the node.
    ///
    /// The factory function receives full access to the executor for node creation.
    /// This function will be called exactly once on the first `get()` request.
    ///
    /// # Panics
    /// Panics if a factory function has already been attached.
    ///
    /// # Example
    /// ```rust
    /// use wavelet::prelude::*;
    /// let factory = SimpleFactory::default()
    ///     .attach(|executor| {
    ///         NodeBuilder::new(0)
    ///             .on_init(|executor, timer, idx| {
    ///                 // Set up a recurring heartbeat
    ///                 executor.yield_driver().yield_now(idx);
    ///             })
    ///             .build(executor, |_, ctx| {
    ///                 Control::Broadcast
    ///             })
    ///     });
    /// ```
    pub fn attach(self, f: impl FnOnce(&mut Executor) -> Node<T> + 'a) -> Self {
        {
            let mut inner = self.0.borrow_mut();
            inner.attach(f);
        }
        self
    }

    /// Creates or retrieves the cached node instance.
    ///
    /// On the first call, executes the factory function to create the node.
    /// All subsequent calls return the same node instance.
    ///
    /// # Panics
    /// Panics if no factory function has been attached.
    ///
    /// # Example
    /// ```rust, ignore
    /// let logger1 = logger_factory.get(&mut executor);
    /// let logger2 = logger_factory.get(&mut executor);
    /// // logger1 and logger2 are the same node instance
    /// ```
    pub fn get(&self, executor: &mut Executor) -> Node<T> {
        self.0.borrow_mut().get(executor)
    }
}

struct SimpleFactoryInner<'a, T: 'static> {
    node: OnceCell<Node<T>>,
    factory: Option<Box<dyn FnOnce(&mut Executor) -> Node<T> + 'a>>,
}

impl<'a, T: 'static> Default for SimpleFactoryInner<'a, T> {
    fn default() -> Self {
        Self {
            node: OnceCell::new(),
            factory: None,
        }
    }
}

impl<'a, T: 'static> SimpleFactoryInner<'a, T> {
    fn attach(&mut self, f: impl FnOnce(&mut Executor) -> Node<T> + 'a) {
        assert!(self.factory.is_none(), "factory already attached");
        self.factory = Some(Box::new(f));
    }

    fn get(&mut self, executor: &mut Executor) -> Node<T> {
        self.node
            .get_or_init(|| {
                let factory = self.factory.take().expect("no factory attached");
                factory(executor)
            })
            .to_owned()
    }
}

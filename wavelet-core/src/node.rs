use crate::event_driver::Notifier;
use crate::executor::{ExecutionContext, Executor};
use crate::garbage_collector::GarbageCollector;
use crate::graph::NodeContext;
use crate::{Control, Relationship};
use petgraph::prelude::NodeIndex;
use std::cell::UnsafeCell;
use std::io;
use std::rc::{Rc, Weak};

type OnDrop<T> = Box<dyn FnMut(&mut T) + 'static>;

/// A smart pointer to a node in the computation graph.
///
/// `Node<T>` provides shared ownership of node data while maintaining efficient
/// access patterns for the graph runtime. The key design principle is **separation
/// of data and behavior** - the node holds stateful data of type `T`, while the
/// computation logic is provided as a closure during node creation.
///
/// This data-oriented approach enables:
/// - **Zero-cost abstractions**: Direct access to typed data without vtable overhead
/// - **Flexible behavior**: The same data type can have different processing logic
/// - **Controlled mutations**: The runtime coordinates when and how data changes
/// - **Clear ownership**: Data lifetime is tied to the node
///
/// # Mutation Contract
///
/// Node data mutation follows a strict contract:
/// - **Immutable handle**: `Node<T>` provides only read-access to data via `borrow()`
/// - **Mutable access**: Mutations happen exclusively within the cycle function where
///   the runtime provides `&mut T` access
/// - **Runtime awareness**: Mutations during cycle execution inform the scheduler
///   about dependency updates
/// - **Interior mutability discouraged**: Using `Cell`/`RefCell` bypasses the runtime's
///   change tracking and can break incremental computation guarantees
///
/// # Example
/// ```rust
/// use wavelet_core::prelude::{Node, NodeBuilder, Control, Executor};
///
/// let mut executor = Executor::new();
/// let counter_node = NodeBuilder::new(0u64)  // Data: u64 counter
///     .build(&mut executor, |count, ctx| {   // Behavior: increment logic
///         *count += 1;  // Controlled mutation within cycle fn
///         if *count % 100 == 0 {
///             Control::Broadcast  // Runtime tracks this change
///         } else {
///             Control::Unchanged
///         }
///     });
///
/// // Outside cycle: only immutable access
/// let current_value = *counter_node.borrow();  // Read-only
/// ```
///
/// # Safety
/// Uses `Rc<UnsafeCell<T>>` for interior mutability within the single-threaded
/// runtime. The runtime ensures exclusive access during node execution.
pub struct Node<T: 'static>(Rc<UnsafeCell<NodeInner<T>>>);

impl<T: 'static> Node<T> {
    /// Creates an uninitialized node with default graph metadata.
    ///
    /// This is the first phase of the two-phase node initialization process:
    /// 1. **Uninitialized creation**: Node is created with placeholder graph metadata
    /// 2. **Graph registration**: Node is inserted into the graph and gets real metadata
    ///
    /// The node starts with placeholder values:
    /// - `index`: Temporary `NodeIndex::new(0)` (updated after graph insertion)
    /// - `depth`: `0` (calculated from dependency relationships)
    /// - `mut_epoch`: `0` (managed by the runtime scheduler)
    ///
    /// This phased approach allows the node to exist before knowing its final
    /// position in the graph, enabling circular reference handling and builder
    /// pattern flexibility.
    ///
    /// # Parameters
    /// - `data`: The node's state data of type `T`
    /// - `name`: Optional debug name for the node
    /// - `gc`: Garbage collector handle for cleanup coordination
    pub(crate) fn uninitialized(data: T, name: Option<String>, gc: GarbageCollector) -> Self {
        Self {
            0: Rc::new(UnsafeCell::new(NodeInner {
                data,
                name,
                on_drop: None,
                gc,
                index: NodeIndex::new(0),
                mut_epoch: 0,
                depth: 0,
            })),
        }
    }

    /// Returns the optional debug name of this node.
    #[inline(always)]
    pub fn name(&self) -> Option<&str> {
        self.get().name.as_deref()
    }

    /// Creates a weak reference to this node that doesn't affect its lifecycle.
    ///
    /// Useful when you need a handle to the node but don't want to prevent
    /// garbage collection. The weak reference can be upgraded back to a
    /// strong `Node<T>` if the node is still alive.
    #[inline(always)]
    pub fn downgrade(&self) -> WeakNode<T> {
        WeakNode(Rc::downgrade(&self.0))
    }

    /// Returns the node's unique identifier within the graph.
    #[inline(always)]
    pub fn index(&self) -> NodeIndex {
        self.get().index
    }

    /// Provides immutable access to the node's data.
    ///
    /// This is the primary way to read node data from other nodes.
    /// For mutations, use the mutable access provided within cycle functions.
    #[inline(always)]
    pub fn borrow(&self) -> &T {
        &self.get().data
    }

    /// Provides mutable access to the node's data (runtime internal only).
    ///
    /// Only used by the runtime during node execution to provide controlled
    /// mutation access within cycle functions.
    #[inline(always)]
    pub(crate) fn borrow_mut(&self) -> &mut T {
        &mut self.get_mut().data
    }

    /// Internal accessor for node metadata (immutable).
    #[inline(always)]
    fn get(&self) -> &NodeInner<T> {
        unsafe { &*self.0.get() }
    }

    /// Internal accessor for node metadata (mutable).
    #[inline(always)]
    fn get_mut(&self) -> &mut NodeInner<T> {
        unsafe { &mut *self.0.get() }
    }

    /// Returns the node's depth level in the dependency graph (runtime internal).
    #[inline(always)]
    pub(crate) fn depth(&self) -> u32 {
        self.get().depth
    }

    /// Returns the epoch when this node last mutated (runtime internal).
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

/// A weak reference to a node that doesn't affect its lifecycle.
///
/// `WeakNode<T>` holds a weak reference to a node without preventing
/// garbage collection. This is useful for avoiding reference cycles
/// or when you need a handle that can detect if a node has been cleaned up.
///
/// # Example
/// ```rust
/// use wavelet_core::prelude::{Node, NodeBuilder, WeakNode, Executor, Control};
///
/// let mut executor = Executor::new();
/// let node = NodeBuilder::new(0).build(&mut executor, |_| Control::Unchanged);
/// let weak_ref = node.downgrade();
///
/// // Later, check if the node is still alive
/// if let Some(strong_node) = weak_ref.upgrade() {
///     // Node is still alive, can access data
///     let value = *strong_node.borrow();
/// } else {
///     // Node has been garbage collected
/// }
/// ```
pub struct WeakNode<T: 'static>(Weak<UnsafeCell<NodeInner<T>>>);

impl<T: 'static> WeakNode<T> {
    /// Attempts to upgrade the weak reference to a strong `Node<T>`.
    ///
    /// Returns `Some(Node<T>)` if the node is still alive, or `None`
    /// if it has been garbage collected.
    #[inline(always)]
    pub fn upgrade(&self) -> Option<Node<T>> {
        self.0.upgrade().map(|rc| Node(rc))
    }
}

struct NodeInner<T: 'static> {
    data: T,
    name: Option<String>,
    on_drop: Option<OnDrop<T>>,
    gc: GarbageCollector,
    index: NodeIndex,
    mut_epoch: usize,
    depth: u32,
}

impl<T: 'static> Drop for NodeInner<T> {
    fn drop(&mut self) {
        self.gc.mark_for_sweep(self.index);
        self.on_drop
            .take()
            .map(|mut on_drop| (on_drop)(&mut self.data));
    }
}

/// Builder for creating nodes in the computation graph.
///
/// `NodeBuilder<T>` is the primary interface users interact with when creating
/// nodes. It provides a fluent API for configuring node relationships, lifecycle
/// callbacks, and metadata before inserting the node into the graph.
///
/// The builder follows a clear pattern:
/// 1. **Create**: Start with node data using `NodeBuilder::new(data)`
/// 2. **Configure**: Add relationships, names, and callbacks
/// 3. **Build**: Insert into the graph with `build()` or `spawn()`
///
/// # Example
/// ```rust, ignore
/// let market_data = NodeBuilder::new(PriceData::default())
///     .with_name("price_processor".to_string())
///     .triggered_by(&data_feed)
///     .observer_of(&config_node)
///     .on_init(|executor, data, idx| {
///         // Setup I/O or timers here
///         executor.yield_driver().yield_now(idx);
///     })
///     .build(&mut executor, |data, ctx| {
///         // Process price updates
///         data.update_prices(ctx);
///         Control::Broadcast
///     });
/// ```
///
/// # Fields
/// - `data`: The initial state data for the node
/// - `name`: Optional debug name for logging/debugging
/// - `parents`: Dependency relationships (parent index, depth, relationship type)
/// - `on_init`: Called once when the node is first added to graph
/// - `on_drop`: Called when the node is being dropped
pub struct NodeBuilder<T: 'static> {
    data: T,
    name: Option<String>,
    parents: Vec<(NodeIndex, u32, Relationship)>,
    on_init: Option<Box<dyn FnMut(&mut Executor, &mut T, NodeIndex) + 'static>>,
    on_drop: Option<OnDrop<T>>,
}

impl<T: 'static> NodeBuilder<T> {
    pub fn new(data: T) -> Self {
        Self {
            data,
            name: None,
            parents: Vec::new(),
            on_init: None,
            on_drop: None,
        }
    }

    pub fn with_name(mut self, name: String) -> Self {
        self.name = Some(name);
        self
    }

    /// Adds a dependency relationship to another node in the graph.
    ///
    /// This is the foundational method for building graph topology. It records
    /// the parent node's index, depth, and relationship type for use during
    /// graph construction. The child node's depth will be calculated as the
    /// maximum parent depth + 1.
    ///
    /// # Parameters
    /// - `parent`: The node this new node depends on
    /// - `relationship`: How this node should react to parent changes
    #[inline]
    pub fn add_relationship<P>(mut self, parent: &Node<P>, relationship: Relationship) -> Self {
        self.parents
            .push((parent.index(), parent.depth(), relationship));
        self
    }

    /// Creates a `Trigger` relationship with the parent node.
    ///
    /// The new node will be automatically scheduled whenever the parent
    /// node mutates (returns `Control::Broadcast`). Use this when the
    /// child should immediately react to a parent's changes.
    #[inline]
    pub fn triggered_by<P>(self, parent: &Node<P>) -> Self {
        self.add_relationship(parent, Relationship::Trigger)
    }

    /// Creates an `Observe` relationship with the parent node.
    ///
    /// The new node can read from the parent but won't be automatically
    /// scheduled when the parent changes. Use this when you need access
    /// to parent data but want to control when to react to changes using
    /// `ExecutionContext::has_mutated()`.
    #[inline]
    pub fn observer_of<P>(self, parent: &Node<P>) -> Self {
        self.add_relationship(parent, Relationship::Observe)
    }

    /// Sets a callback to run when the node is first added to the graph.
    ///
    /// The `on_init` callback is called exactly once after the node has been
    /// inserted into the graph and assigned its final `NodeIndex`. This is
    /// the ideal place to:
    /// - Register I/O interests or timers
    /// - Schedule the node for initial execution
    /// - Set up any resources that need the node's graph position
    ///
    /// # Parameters
    /// - `executor`: Full access to the runtime for registering events
    /// - `data`: Mutable access to the node's data for initialization
    /// - `idx`: The node's assigned index in the graph
    ///
    /// # Example
    /// ```rust, ignore
    /// .on_init(|executor, data, idx| {
    ///     // Start processing immediately
    ///     executor.yield_driver().yield_now(idx);
    ///
    ///     // Set up a recurring timer
    ///     data.timer_handle = Some(executor.timer_driver()
    ///         .register_timer(idx, Instant::now() + Duration::from_secs(1)));
    /// })
    /// ```
    pub fn on_init<F>(mut self, on_init: F) -> Self
    where
        F: FnMut(&mut Executor, &mut T, NodeIndex) + 'static,
    {
        assert!(self.on_init.is_none(), "cannot set on_init twice");
        self.on_init = Some(Box::new(on_init));
        self
    }

    /// Sets a callback to run when the node is dropped.
    ///
    /// The `on_drop` callback is called when the node is about to be removed
    /// from the graph. Use this to clean up resources like file handles,
    /// network connections, or custom allocations.
    ///
    /// # Example
    /// ```rust, ignore
    /// .on_drop(|data| {
    ///     // Clean up resources
    ///     if let Some(file) = data.log_file.take() {
    ///         let _ = file.sync_all();
    ///     }
    /// })
    /// ```
    pub fn on_drop<F>(mut self, on_drop: F) -> Self
    where
        F: FnMut(&mut T) + 'static,
    {
        assert!(self.on_drop.is_none(), "cannot set on_drop twice");
        self.on_drop = Some(Box::new(on_drop));
        self
    }

    /// Builds the node and inserts it into the graph, returning a handle.
    ///
    /// This completes the node creation process by:
    /// 1. Creating an uninitialized node with the builder's data
    /// 2. Calculating the node's depth based on parent relationships
    /// 3. Wrapping the cycle function with weak reference safety
    /// 4. Inserting the node into the graph and establishing edges
    /// 5. Running the `on_init` callback if provided
    ///
    /// The returned `Node<T>` handle keeps the node alive - if all handles
    /// are dropped, the node will be garbage collected.
    ///
    /// # Parameters
    /// - `executor`: The runtime to insert the node into
    /// - `cycle_fn`: The computation logic for this node
    ///
    /// # Returns
    /// A strong reference to the created node
    ///
    /// # Note
    /// This method only creates one strong reference to the node, which
    /// is what is returned. The closure registered with the graph uses
    /// a weak reference, which allows us to handle dynamic graph cleanup
    /// via the garbage collection mechanism.
    pub fn build<F>(self, executor: &mut Executor, mut cycle_fn: F) -> Node<T>
    where
        F: FnMut(&mut T, &mut ExecutionContext) -> Control + 'static,
    {
        let node = Node::uninitialized(self.data, self.name, executor.garbage_collector());
        let depth = self
            .parents
            .iter()
            .map(|(_, depth, _)| depth)
            .max()
            .map(|d| d + 1)
            .unwrap_or(0);

        {
            let state = node.downgrade();
            let cycle_fn = Box::new(move |ctx: &mut ExecutionContext| match state.upgrade() {
                Some(state) => cycle_fn(state.borrow_mut(), ctx),
                None => Control::Sweep,
            });

            let idx = executor.graph().add_node(NodeContext::new(cycle_fn, depth));
            let inner = node.get_mut();
            inner.index = idx;
            inner.depth = depth;

            self.parents.iter().for_each(|(parent, _, relationship)| {
                executor.graph().add_edge(*parent, idx, *relationship);
            });

            executor.scheduler().enable_depth(depth);
            if let Some(mut on_init) = self.on_init {
                (on_init)(executor, &mut node.get_mut().data, idx)
            }

            inner.on_drop = self.on_drop;
        }

        node
    }

    /// Builds the node and creates an associated notifier for external events.
    ///
    /// This is a convenience method that combines `build()` with notifier
    /// registration. Use this when the node needs to respond to external
    /// events (like informing updates from a background thread) in addition
    /// to graph-based dependencies.
    ///
    /// # Returns
    /// A tuple of `(Node<T>, Notifier)` where the notifier can wake the node
    ///
    /// # Note
    /// This method only creates one strong reference to the node, which
    /// is what is returned. The closure registered with the graph uses
    /// a weak reference, which allows us to handle dynamic graph cleanup
    /// via the garbage collection mechanism.
    pub fn build_with_notifier<F>(
        self,
        executor: &mut Executor,
        cycle_fn: F,
    ) -> io::Result<(Node<T>, Notifier)>
    where
        F: FnMut(&mut T, &mut ExecutionContext) -> Control + 'static,
    {
        let node = self.build(executor, cycle_fn);
        let notifier = executor.io_driver().register_notifier(node.index())?;
        Ok((node, notifier))
    }

    /// Builds the node without returning a handle (fire-and-forget).
    ///
    /// This creates the node in the graph but doesn't return a handle to it.
    /// The node will stay alive as long as it has incoming edges from other
    /// nodes or until it returns `Control::Sweep`. Use this pattern for:
    /// - Leaf nodes that don't need external references
    /// - Temporary processing nodes
    /// - Nodes that manage their own lifecycle
    ///
    /// # Safety Note
    /// Without a handle, you can't directly access the node's data from
    /// outside the graph. The node can only be controlled through its
    /// relationships and cycle function.
    ///
    /// # Note
    /// This method only creates one strong reference to the node, which
    /// is registered with the graph closure. The node created from `spawn`
    /// needs to own its own lifecycle to ensure we do not leak resources.
    pub fn spawn<F>(self, executor: &mut Executor, mut cycle_fn: F)
    where
        F: FnMut(&mut T, &mut ExecutionContext) -> Control + 'static,
    {
        let node = Node::uninitialized(self.data, self.name, executor.garbage_collector());
        let depth = self
            .parents
            .iter()
            .map(|(_, depth, _)| depth)
            .max()
            .map(|d| d + 1)
            .unwrap_or(0);

        let state = node.clone();
        let cycle_fn =
            Box::new(move |ctx: &mut ExecutionContext| cycle_fn(state.borrow_mut(), ctx));

        let idx = executor.graph().add_node(NodeContext::new(cycle_fn, depth));
        let inner = node.get_mut();
        inner.index = idx;
        inner.depth = depth;

        self.parents.iter().for_each(|(parent, _, relationship)| {
            executor.graph().add_edge(*parent, idx, *relationship);
        });

        executor.scheduler().enable_depth(depth);
        if let Some(mut on_init) = self.on_init {
            (on_init)(executor, &mut node.get_mut().data, idx)
        }

        inner.on_drop = self.on_drop;
    }
}

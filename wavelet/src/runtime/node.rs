use crate::channel::Channel;
#[cfg(feature = "channel")]
use crate::channel::{Receiver, Sender};
use crate::runtime::executor::{ExecutionContext, Executor};
use crate::runtime::garbage_collector::GarbageCollector;
use crate::runtime::graph::NodeContext;
use crate::runtime::{CycleFn, Notifier};
use crate::{Control, Relationship};
use petgraph::prelude::NodeIndex;
use std::cell::UnsafeCell;
use std::collections::HashSet;
use std::io;
use std::rc::{Rc, Weak};
use std::sync::Arc;

type OnDrop<T> = Box<dyn FnMut(&mut T) + 'static>;

/// Common interface for accessing node metadata regardless of ownership model.
///
/// NodeHandle provides a unified way to access core node properties (index, depth, mut_epoch)
/// across different node types including `Node<T>`, `ExclusiveNode<T>`, and `RawHandle`.
/// This abstraction allows code to work generically with node metadata without caring
/// about the specific ownership or access patterns.
///
/// # Example
///
/// ```rust,ignore
/// fn schedule_node<H: NodeHandle>(scheduler: &mut Scheduler, handle: &H) {
///     scheduler.schedule(handle.index(), handle.depth());
/// }
///
/// // Works with any node type
/// schedule_node(&mut scheduler, &regular_node);
/// schedule_node(&mut scheduler, &exclusive_node);
/// schedule_node(&mut scheduler, &raw_handle);
/// ```
pub trait NodeHandle {
    fn index(&self) -> NodeIndex;
    fn depth(&self) -> u32;
    fn mut_epoch(&self) -> usize;
}

/// A lightweight handle containing only essential node metadata.
///
/// RawHandle stores the minimum information needed to identify and work with a node
/// in the execution graph without holding references to the actual node data or
/// requiring borrowing semantics. This is particularly useful when:
///
/// - A node is moved exclusively and you need to retain metadata
/// - Passing node information across ownership boundaries
/// - Caching node metadata for later scheduling decisions
/// - Working with node references in contexts where borrowing is problematic
///
/// # Use Case: Exclusive Node Moves
///
/// The primary motivation is handling cases where a node gets moved for exclusive
/// access, but you need to preserve metadata for scheduling or graph operations:
///
/// ```rust,ignore
/// // Extract handle before exclusive move
/// let handle = RawHandle::from(&my_node);
///
/// // Node gets moved exclusively somewhere else
/// exclusive_processor.consume_node(my_node);
///
/// // Can still use metadata for scheduling
/// scheduler.schedule(handle.index(), handle.depth());
/// ```
///
/// # Characteristics
///
/// - **Lightweight**: Only stores `NodeIndex` and depth, no heap allocations
/// - **Copy-friendly**: Implements `Clone` and `Debug` for easy manipulation
/// - **No Epochs**: Returns 0 for `mut_epoch()` since raw handles don't track mutations
/// - **Immutable**: Represents a snapshot of node metadata at creation time
#[derive(Debug, Clone)]
pub struct RawHandle {
    index: NodeIndex,
    depth: u32,
}

impl RawHandle {
    const fn new(index: NodeIndex, depth: u32) -> Self {
        Self { index, depth }
    }

    pub const fn index(&self) -> NodeIndex {
        self.index
    }

    pub const fn depth(&self) -> u32 {
        self.depth
    }
}

impl NodeHandle for RawHandle {
    fn index(&self) -> NodeIndex {
        self.index
    }

    fn depth(&self) -> u32 {
        self.depth
    }

    fn mut_epoch(&self) -> usize {
        unimplemented!("RawHandle doesn't track mutations")
    }
}

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
/// ```rust, ignore
/// use wavelet::prelude::{Node, NodeBuilder, Control, Executor};
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

impl<T: 'static> NodeHandle for Node<T> {
    fn index(&self) -> NodeIndex {
        self.index()
    }

    fn depth(&self) -> u32 {
        self.depth()
    }

    fn mut_epoch(&self) -> usize {
        self.mut_epoch()
    }
}

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

    #[inline(always)]
    pub fn raw_handle(&self) -> RawHandle {
        RawHandle::new(self.index(), self.depth())
    }

    /// Returns the optional debug name of this node.
    #[inline(always)]
    pub fn name(&self) -> Option<&str> {
        unsafe { self.get_inner().name.as_deref() }
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

    #[inline(always)]
    pub fn upgrade(self) -> Option<ExclusiveNode<T>> {
        if Rc::strong_count(&self.0) == 1 && Rc::weak_count(&self.0) == 1 {
            Some(ExclusiveNode(self.0))
        } else {
            None
        }
    }

    /// Returns the node's unique identifier within the graph.
    #[inline(always)]
    pub fn index(&self) -> NodeIndex {
        unsafe { self.get_inner().index }
    }

    /// Provides immutable access to the node's data.
    ///
    /// This is the primary way to read node data from other wsnl.
    /// For mutations, use the mutable access provided within cycle functions.
    #[inline(always)]
    pub fn borrow(&self) -> &T {
        unsafe { &self.get_inner().data }
    }

    /// Provides mutable access to the node's data (runtime internal only).
    ///
    /// Only used by the runtime during node execution to provide controlled
    /// mutation access within cycle functions.
    #[inline(always)]
    fn borrow_mut(&self) -> &mut T {
        unsafe { &mut self.get_inner_mut().data }
    }

    /// Provides unsafe immutable accessor to
    /// node's wrapped data
    pub unsafe fn get(&self) -> &T {
        unsafe { &self.get_inner().data }
    }

    /// Provides unsafe mutable accessor to
    /// node's wrapped data
    pub unsafe fn get_mut(&mut self) -> &mut T {
        unsafe { &mut self.get_inner_mut().data }
    }

    /// Internal accessor for node metadata (immutable).
    ///
    /// Public, but labeled as unsafe.
    #[inline(always)]
    unsafe fn get_inner(&self) -> &NodeInner<T> {
        unsafe { &*self.0.get() }
    }

    /// Internal accessor for node metadata (mutable).
    ///
    /// Public, but labeled as unsafe.
    #[inline(always)]
    unsafe fn get_inner_mut(&self) -> &mut NodeInner<T> {
        unsafe { &mut *self.0.get() }
    }

    /// Returns the node's depth level in the dependency graph (runtime internal).
    #[inline(always)]
    pub fn depth(&self) -> u32 {
        unsafe { self.get_inner().depth }
    }

    /// Returns the epoch when this node last mutated (runtime internal).
    #[inline(always)]
    pub fn mut_epoch(&self) -> usize {
        unsafe { self.get_inner().mut_epoch }
    }

    /// Internal method to set the epoch if the node
    /// has mutated.
    #[inline(always)]
    fn set_mut_epoch(&mut self, epoch: usize) {
        unsafe {
            self.get_inner_mut().mut_epoch = epoch;
        }
    }
}

impl<T: 'static> Clone for Node<T> {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// ExclusiveNode: Exclusive Consumption Pattern
///
/// ExclusiveNode enables a child node to take exclusive ownership of data from its parent,
/// supporting single-consumer patterns where data should be consumed by exactly one
/// downstream processor.
///
/// # Example Use Case: Routing and Dispatching
///
/// An example motivation is routing scenarios where:
/// 1. A generic parent collects data (e.g., from a socket buffer)
/// 2. A specialized child consumes all data exclusively to route/dispatch it
/// 3. The parent remains generic and reusable across different routing strategies
///
/// ```rust,ignore
/// // Generic data collector - reusable across different routing strategies
/// let socket_buffer = NodeBuilder::new(SocketBuffer::new())
///     .build(&mut executor, |buffer, ctx| {
///         // Read from socket into generic buffer
///         buffer.read_from_socket()?;
///         Control::Broadcast
///     });
///
/// // Exclusive consumer that routes based on message headers
/// let message_router = ExclusiveNodeBuilder::new(MessageRouter::new())
///     .exclusively_consumes(&socket_buffer)
///     .build(&mut executor, |router, ctx, parent_data| {
///         // Take exclusive ownership of all buffered data
///         let messages = parent_data.take_all_messages();
///
///         for msg in messages {
///             match msg.routing_key() {
///                 "orders" => router.dispatch_to_order_processor(msg),
///                 "quotes" => router.dispatch_to_quote_processor(msg),
///                 "admin" => router.dispatch_to_admin_handler(msg),
///                 _ => router.send_to_dead_letter(msg),
///             }
///         }
///
///         Control::Broadcast
///     });
/// ```
///
/// This pattern enables clean separation between generic data collection and specialized
/// routing/dispatching logic while maintaining efficient data flow.
pub struct ExclusiveNode<T: 'static>(Rc<UnsafeCell<NodeInner<T>>>);

impl<T: 'static> NodeHandle for ExclusiveNode<T> {
    fn index(&self) -> NodeIndex {
        self.index()
    }

    fn depth(&self) -> u32 {
        self.depth()
    }

    fn mut_epoch(&self) -> usize {
        self.mut_epoch()
    }
}

impl<T: 'static> ExclusiveNode<T> {
    #[inline(always)]
    pub fn downgrade(self) -> Node<T> {
        Node(self.0)
    }

    #[inline(always)]
    pub fn raw_handle(&self) -> RawHandle {
        RawHandle::new(self.index(), self.depth())
    }

    /// Returns the optional debug name of this node.
    #[inline(always)]
    pub fn name(&self) -> Option<&str> {
        unsafe { self.get_inner().name.as_deref() }
    }

    /// Returns the node's unique identifier within the graph.
    #[inline(always)]
    pub fn index(&self) -> NodeIndex {
        unsafe { self.get_inner().index }
    }

    #[inline(always)]
    pub fn depth(&self) -> u32 {
        unsafe { self.get_inner().depth }
    }

    #[inline(always)]
    pub fn mut_epoch(&self) -> usize {
        unsafe { self.get_inner().mut_epoch }
    }

    /// Provides immutable access to the node's data.
    ///
    /// This is the primary way to read node data from other wsnl.
    /// For mutations, use the mutable access provided within cycle functions.
    #[inline(always)]
    pub fn borrow(&self) -> &T {
        unsafe { &self.get_inner().data }
    }

    /// Provides mutable access to the node's data.
    ///
    /// This is safe, since we enforce that at this
    /// point there is exclusive access to the node.
    #[inline(always)]
    pub fn borrow_mut(&mut self) -> &mut T {
        unsafe { &mut self.get_inner_mut().data }
    }

    /// Provides unsafe immutable accessor to
    /// node's wrapped data
    pub unsafe fn get(&self) -> &T {
        unsafe { &self.get_inner().data }
    }

    /// Provides unsafe mutable accessor to
    /// node's wrapped data
    pub unsafe fn get_mut(&mut self) -> &mut T {
        unsafe { &mut self.get_inner_mut().data }
    }

    /// Internal accessor for node metadata (immutable).
    #[inline(always)]
    unsafe fn get_inner(&self) -> &NodeInner<T> {
        unsafe { &*self.0.get() }
    }

    /// Internal accessor for node metadata (mutable).
    #[inline(always)]
    unsafe fn get_inner_mut(&self) -> &mut NodeInner<T> {
        unsafe { &mut *self.0.get() }
    }
}

/// A weak reference to a node that doesn't affect its lifecycle.
///
/// `WeakNode<T>` holds a weak reference to a node without preventing
/// garbage collection. This is useful for avoiding reference cycles
/// or when you need a handle that can detect if a node has been cleaned up.
///
/// # Example
/// ```rust, ignore
/// use wavelet::prelude::{Node, NodeBuilder, WeakNode, Executor, Control};
///
/// let mut executor = Executor::new();
/// let node = NodeBuilder::new(0).build(&mut executor, |_, _| Control::Unchanged);
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

/// Builder for creating wsnl in the computation graph.
///
/// `NodeBuilder<T>` is the primary interface users interact with when creating
/// wsnl. It provides a fluent API for configuring node relationships, lifecycle
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
///
/// # Panic Handling
///
/// By default, node cycle functions are wrapped in
/// [`std::panic::catch_unwind`]. If a panic occurs during execution:
/// - The panic is isolated to the node
/// - The node is marked for garbage collection (`Control::Sweep`)
/// - The rest of the graph continues uninterrupted
///
/// This provides fault tolerance with negligible overhead on the
/// non-panic path. If you prefer fail-fast semantics instead, use
/// [`NodeBuilder::allow_panic`], which disables isolation and lets
/// panics propagate normally.
///
/// # Author Responsibility
///
/// While the runtime isolates panics at the node boundary, authors are
/// responsible for ensuring their cycle logic does not leave behind
/// inconsistent states if unwound mid-execution. To avoid logic errors:
/// - Perform work in locals and commit results to the node state at the end
/// - Avoid external side effects (I/O, scheduling) before committing
/// - Do not leak partially-mutated references across cycles
///
/// Following these rules makes it safe for the runtime to isolate panics
/// without corrupting downstream computation.
pub struct NodeBuilder<T: 'static> {
    data: T,
    name: Option<String>,
    parents: HashSet<(NodeIndex, u32, Relationship)>,
    on_init: Option<Box<dyn FnMut(&mut Executor, &mut T, NodeIndex) + 'static>>,
    on_drop: Option<OnDrop<T>>,
    allow_panic: bool,
}

impl<T: 'static> NodeBuilder<T> {
    pub fn new(data: T) -> Self {
        Self {
            data,
            name: None,
            parents: HashSet::new(),
            on_init: None,
            on_drop: None,
            allow_panic: false,
        }
    }

    /// Sets the `name` field of the struct to the provided `name` value and returns the updated instance.
    pub fn named(mut self, name: String) -> Self {
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
    pub fn add_relationship<N: NodeHandle>(
        mut self,
        parent: &N,
        relationship: Relationship,
    ) -> Self {
        assert!(
            !self
                .parents
                .contains(&(parent.index(), parent.depth(), relationship)),
            "cannot add duplicate relationship"
        );
        self.parents
            .insert((parent.index(), parent.depth(), relationship));
        self
    }

    /// Adds multiple dependency relationships to other wsnl with
    /// the same relationship type.
    #[inline]
    pub fn add_many_relationships<'a, N: NodeHandle + 'a>(
        self,
        parents: impl IntoIterator<Item = &'a N>,
        relationship: Relationship,
    ) -> Self {
        parents
            .into_iter()
            .fold(self, |s, parent| s.add_relationship(parent, relationship))
    }

    /// Creates a `Trigger` relationship with the parent node.
    ///
    /// The new node will be automatically scheduled whenever the parent
    /// node mutates (returns `Control::Broadcast`). Use this when the
    /// child should immediately react to a parent's changes.
    #[inline]
    pub fn triggered_by<N: NodeHandle>(self, parent: &N) -> Self {
        self.add_relationship(parent, Relationship::Trigger)
    }

    /// Creates multiple `Trigger` relationships.
    #[inline]
    pub fn triggered_by_many<'a, N: NodeHandle + 'a>(
        self,
        parents: impl IntoIterator<Item = &'a N>,
    ) -> Self {
        self.add_many_relationships(parents, Relationship::Trigger)
    }

    /// Creates an `Observe` relationship with the parent node.
    ///
    /// The new node can read from the parent but won't be automatically
    /// scheduled when the parent changes. Use this when you need access
    /// to parent data but want to control when to react to changes using
    /// `ExecutionContext::has_mutated()`.
    #[inline]
    pub fn observer_of<N: NodeHandle>(self, parent: &N) -> Self {
        self.add_relationship(parent, Relationship::Observe)
    }

    /// Creates multiple `Observe` relationships.
    #[inline]
    pub fn observer_of_many<'a, N: NodeHandle + 'a>(
        self,
        parents: impl IntoIterator<Item = &'a N>,
    ) -> Self {
        self.add_many_relationships(parents, Relationship::Observe)
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

    /// Sets whether the node is allowed to panic during execution,
    /// i.e., is not caught by a panic unwind.
    pub fn allow_panic(mut self, allow_panic: bool) -> Self {
        self.allow_panic = allow_panic;
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
            let cycle_fn: CycleFn = if self.allow_panic {
                Box::new(move |ctx: &mut ExecutionContext| match state.upgrade() {
                    Some(mut state) => {
                        let ctrl = cycle_fn(state.borrow_mut(), ctx);
                        if ctrl.is_broadcast() {
                            state.set_mut_epoch(ctx.epoch());
                        }
                        ctrl
                    }
                    None => Control::Sweep,
                })
            } else {
                Box::new(move |ctx: &mut ExecutionContext| match state.upgrade() {
                    Some(mut state) => {
                        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            cycle_fn(state.borrow_mut(), ctx)
                        })) {
                            Ok(ctrl) => {
                                if ctrl.is_broadcast() {
                                    state.set_mut_epoch(ctx.epoch());
                                }
                                ctrl
                            }
                            Err(_) => Control::Sweep,
                        }
                    }
                    None => Control::Sweep,
                })
            };

            let idx = executor.graph().add_node(NodeContext::new(cycle_fn, depth));
            let inner = unsafe { node.get_inner_mut() };
            inner.index = idx;
            inner.depth = depth;

            self.parents.iter().for_each(|(parent, _, relationship)| {
                executor.graph().add_edge(*parent, idx, *relationship);
            });

            executor.scheduler().enable_depth(depth);
            if let Some(mut on_init) = self.on_init {
                (on_init)(executor, &mut unsafe { node.get_inner_mut() }.data, idx)
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
        let notifier = executor.register_notifier(node.index());
        Ok((node, notifier))
    }

    #[cfg(feature = "channel")]
    /// Builds a node with an associated MPSC channel for external message passing.
    ///
    /// This method creates a node that can receive messages from outside the graph through
    /// a bounded MPSC channel. The channel is automatically integrated with the executor's
    /// I/O driver, so messages trigger node execution without polling.
    ///
    /// # Type Parameters
    ///
    /// * `U` - The type of messages that can be sent through the channel
    /// * `F` - The cycle function that processes both node state and channel messages
    ///
    /// # Arguments
    ///
    /// * `executor` - The executor managing the graph
    /// * `capacity` - The channel's buffer capacity (must be > 0)
    /// * `cycle_fn` - Function called on each node execution, receiving:
    ///   - `&mut T`: Mutable reference to the node's state
    ///   - `&mut ExecutionContext`: The execution context for scheduling and I/O
    ///   - `&Receiver<U>`: Channel receiver for incoming messages
    ///
    /// # Returns
    ///
    /// Returns a tuple containing:
    /// - The constructed `Node<T>` handle
    /// - A `Sender<U>` for sending messages to the node
    ///
    /// Returns an `io::Error` if the notifier registration fails.
    ///
    /// # Channel Behavior
    ///
    /// - Messages sent through the `Sender` automatically wake the node for processing
    /// - The channel is bounded with the specified capacity
    /// - When all senders are dropped, the receiver will report the channel as closed
    /// - The channel receiver is accessible only within the cycle function
    ///
    /// # Example
    ///
    /// ```rust, ignore
    /// let (node, tx) = NodeBuilder::new(MyState::default())
    ///     .triggered_by(&parent_node)
    ///     .build_with_channel(executor, 100, |state, ctx, rx| {
    ///         // Process any pending messages
    ///         while let Ok(msg) = rx.try_receive() {
    ///             state.process_message(msg);
    ///         }
    ///
    ///         // Return control flow based on state changes
    ///         if state.has_changes() {
    ///             Control::Broadcast
    ///         } else {
    ///             Control::Unchanged
    ///         }
    ///     })?;
    ///
    /// // External code can now send messages to the node
    /// tx.send(MyMessage::new()).unwrap();
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is 0.
    ///
    /// # Panic Safety
    ///
    /// The node's panic behavior depends on the builder's `allow_panic` setting:
    /// - If `allow_panic` is false (default), panics in the cycle function are caught
    ///   and converted to `Control::Sweep`, marking the node for removal
    /// - If `allow_panic` is true, panics propagate normally
    pub fn build_with_channel<U, F>(
        self,
        executor: &mut Executor,
        capacity: usize,
        mut cycle_fn: F,
    ) -> io::Result<(Node<T>, Sender<U>)>
    where
        U: 'static,
        F: FnMut(&mut T, &mut ExecutionContext, &Receiver<U>) -> Control + 'static,
    {
        assert!(capacity > 0, "capacity must be greater than 0");
        let node = Node::uninitialized(self.data, self.name, executor.garbage_collector());
        let depth = self
            .parents
            .iter()
            .map(|(_, depth, _)| depth)
            .max()
            .map(|d| d + 1)
            .unwrap_or(0);

        let chan = Arc::new(Channel::new(capacity));
        let rx = Receiver::new(chan.clone());
        {
            let state = node.downgrade();
            let receiver = rx;
            let cycle_fn: CycleFn = if self.allow_panic {
                Box::new(move |ctx: &mut ExecutionContext| match state.upgrade() {
                    Some(mut state) => {
                        let ctrl = cycle_fn(state.borrow_mut(), ctx, &receiver);
                        if ctrl.is_broadcast() {
                            state.set_mut_epoch(ctx.epoch());
                        }
                        ctrl
                    }
                    None => Control::Sweep,
                })
            } else {
                Box::new(move |ctx: &mut ExecutionContext| match state.upgrade() {
                    Some(mut state) => {
                        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            cycle_fn(state.borrow_mut(), ctx, &receiver)
                        })) {
                            Ok(ctrl) => {
                                if ctrl.is_broadcast() {
                                    state.set_mut_epoch(ctx.epoch());
                                }
                                ctrl
                            }
                            Err(_) => Control::Sweep,
                        }
                    }
                    None => Control::Sweep,
                })
            };

            let idx = executor.graph().add_node(NodeContext::new(cycle_fn, depth));
            let inner = unsafe { node.get_inner_mut() };
            inner.index = idx;
            inner.depth = depth;

            self.parents.iter().for_each(|(parent, _, relationship)| {
                executor.graph().add_edge(*parent, idx, *relationship);
            });

            executor.scheduler().enable_depth(depth);
            if let Some(mut on_init) = self.on_init {
                (on_init)(executor, &mut unsafe { node.get_inner_mut() }.data, idx)
            }

            inner.on_drop = self.on_drop;
        }

        let notifier = executor.register_notifier(node.index());
        let tx = Sender::new(chan, notifier);
        Ok((node, tx))
    }

    /// Builds the node without returning a handle (fire-and-forget).
    ///
    /// This creates the node in the graph but doesn't return a handle to it.
    /// The node will stay alive as long as it has incoming edges from other
    /// wsnl or until it returns `Control::Sweep`. Use this pattern for:
    /// - Leaf wsnl that don't need external references
    /// - Temporary processing wsnl
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

        let mut state = node.clone();
        let cycle_fn: CycleFn = if self.allow_panic {
            Box::new(move |ctx: &mut ExecutionContext| {
                let ctrl = cycle_fn(state.borrow_mut(), ctx);
                if ctrl.is_broadcast() {
                    state.set_mut_epoch(ctx.epoch());
                }
                ctrl
            })
        } else {
            Box::new(move |ctx: &mut ExecutionContext| {
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    cycle_fn(state.borrow_mut(), ctx)
                })) {
                    Ok(ctrl) => {
                        if ctrl.is_broadcast() {
                            state.set_mut_epoch(ctx.epoch());
                        }
                        ctrl
                    }
                    Err(_) => Control::Sweep,
                }
            })
        };

        let idx = executor.graph().add_node(NodeContext::new(cycle_fn, depth));
        let inner = unsafe { node.get_inner_mut() };
        inner.index = idx;
        inner.depth = depth;

        self.parents.iter().for_each(|(parent, _, relationship)| {
            executor.graph().add_edge(*parent, idx, *relationship);
        });

        executor.scheduler().enable_depth(depth);
        if let Some(mut on_init) = self.on_init {
            (on_init)(executor, &mut unsafe { node.get_inner_mut() }.data, idx)
        }

        inner.on_drop = self.on_drop;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exclusive_node() {
        let mut executor = Executor::new();
        let node = NodeBuilder::new(23).build(&mut executor, |_, _| Control::Unchanged);

        let exclusive = node.upgrade();
        assert!(exclusive.is_some());
        let node = exclusive.unwrap().downgrade();

        let _cloned = std::hint::black_box(node.clone());
        let exclusive = node.upgrade();
        assert!(exclusive.is_none());
    }
}

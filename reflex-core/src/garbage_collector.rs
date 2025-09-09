use petgraph::graph::NodeIndex;
use std::cell::UnsafeCell;
use std::collections::VecDeque;
use std::rc::Rc;

/// Manages deferred removal of nodes from the graph after cycle completion.
///
/// # Safety Invariants
///
/// The GarbageCollector maintains several critical invariants that must be upheld
/// for correct graph behavior:
///
/// ## Leaf-First Removal Invariant
///
/// **CRITICAL**: Nodes should only be marked for removal if they are leaf nodes
/// (nodes with no outgoing edges) or if the entire subgraph rooted at that node
/// is being removed in the same cycle.
///
/// ### Why This Matters
///
/// When a node is dropped and marked for garbage collection, only that specific
/// node is registered for removal. If the node has children that are still
/// referenced elsewhere in the system, those children become "orphaned" but
/// remain in the graph, leading to:
///
/// - **Memory leaks**: Child nodes remain allocated but unreachable
/// - **Broken graph topology**: Dangling references to the removed parent
/// - **Undefined behavior**: Other nodes may hold stale parent references
///
/// ### Example: Correct Usage (Leaf Removal)
///
/// ```text
/// Before removal:
///     A
///    / \
///   B   C (leaf)
///  /
/// D (leaf)
///
/// ✅ Safe to remove: C or D (they are leaves)
/// ❌ Unsafe to remove: A or B (they have children)
/// ```
///
/// ### Example: Incorrect Usage (Non-Leaf Removal)
///
/// ```rust, ignore
/// // ❌ DANGEROUS: Removing a parent node while children exist
/// let parent = create_parent_node();
/// let child1 = create_child_node(&parent);
/// let child2 = create_child_node(&parent);
///
/// // This creates a memory leak - child1 and child2 remain in graph
/// // but parent is removed, creating orphaned nodes
/// drop(parent); // Marks parent for removal, but children remain!
/// ```
///
/// ### Example: Correct Subgraph Removal
///
/// ```rust, ignore
/// // ✅ CORRECT: Remove entire subgraph in dependency order
/// let parent = create_parent_node();
/// let child1 = create_child_node(&parent);
/// let child2 = create_child_node(&parent);
///
/// // Remove children first (leaf-to-root order)
/// drop(child1);
/// drop(child2);
/// // Now safe - no orphaned children and parent will automatically be removed
/// ```
///
/// ## Implementation Responsibility
///
/// The GarbageCollector **cannot enforce** this invariant automatically because:
/// - It only receives `NodeIndex` values, not full graph topology information
/// - Checking parent-child relationships would require expensive graph traversals
///
/// Therefore, **users must ensure** they follow proper removal patterns:
///
/// 1. **Identify leaf nodes** before marking for removal
/// 2. **Remove children before parents** when removing subgraphs
/// 3. **Coordinate removal** across related nodes in the same cycle
///
/// ## Detection and Debugging
///
/// Signs that this invariant has been violated:
/// - Nodes that should have been removed remain in the graph
/// - Unexpected memory growth over time
/// - Broken parent-child relationships in node traversals
/// - Graph algorithms producing unexpected results
/// ```
#[derive(Debug, Clone)]
pub struct GarbageCollector {
    /// Queue of node indices marked for removal
    ///
    /// SAFETY: UnsafeCell is safe here due to temporal separation:
    /// - Writes occur during cycle execution (node drops)
    /// - Reads/drains occur after cycle completion
    removal_queue: Rc<UnsafeCell<VecDeque<NodeIndex>>>,
}

impl GarbageCollector {
    pub(crate) fn new() -> Self {
        Self {
            removal_queue: Rc::new(UnsafeCell::new(VecDeque::new())),
        }
    }

    /// Mark a node for deferred removal after the current cycle.
    ///
    /// # Safety Invariant
    ///
    /// The caller must ensure that the node being marked is either:
    /// 1. A leaf node (no children), OR
    /// 2. Part of a coordinated subgraph removal where all children
    ///    will also be marked for removal in the same cycle
    ///
    /// Violating this invariant may result in orphaned nodes and memory leaks.
    ///
    /// # Arguments
    ///
    /// * `node_index` - The index of the node to remove after cycle completion
    #[inline(always)]
    pub fn mark_for_sweep(&self, node: NodeIndex) {
        unsafe { &mut *self.removal_queue.get() }.push_back(node);
    }

    #[inline(always)]
    pub(crate) fn next_to_sweep(&self) -> Option<NodeIndex> {
        unsafe { &mut *self.removal_queue.get() }.pop_front()
    }
}

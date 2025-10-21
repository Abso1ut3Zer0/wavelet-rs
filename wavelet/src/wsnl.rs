//! # Wavelet Standard Node Library (wsnl)
//!
//! A collection of commonly-used node implementations for stream processing,
//! control flow, timing, and utility operations. These nodes provide the
//! building blocks for constructing complex processing pipelines.
//!
//! ## Organization
//!
//! The library is organized into focused modules:
//!
//! ### [`transform`] - Stream Transformation
//! Nodes that process collections of data:
//! - `accumulate_stream_node` - Fold operations with persistent state
//! - `accumulate_stream_with_reset_node` - Per-cycle aggregations
//! - `filter_stream_node` - Predicate-based filtering
//! - `deduplicate_stream_node` - Remove duplicates by key
//!
//! ### [`control`] - Flow Control
//! Nodes that manage data routing and conditional flow:
//! - `Router` - Dynamic routing by key with on-demand output creation
//! - `switch_stream_node` / `switch_node` - Conditional source selection
//! - `take_*` variants - Consuming versions of routing/switching nodes
//!
//! ### [`time`] - Temporal Operations
//! Nodes that work with time and scheduling:
//! - `periodic_trigger_node` - Fixed-interval event generation
//!
//! ### [`util`] - Utilities
//! Helper nodes for common patterns:
//! - `constant_node` - Static value sources
//! - `sweep_node` - Controlled node cleanup
//!
//! ## Usage Patterns
//!
//! Standard nodes are designed to compose naturally:
//!
//! ```rust, ignore
//! use wavelet::wsnl::*;
//!
//! // Build a processing pipeline
//! let raw_data = /* source */;
//! let filtered = filter_stream_node(executor, raw_data, |x| x.is_valid());
//! let router = take_route_stream_node(executor, filtered, |x| x.category());
//! let category_a = router.borrow().route(executor, "A");
//! let stats = accumulate_stream_node(executor, category_a, Stats::new(), |s, item| s.update(item));
//! ```

pub mod control;
pub mod logic;
pub mod time;
pub mod transform;
pub mod util;

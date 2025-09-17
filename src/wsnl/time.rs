//! # Time-Based Nodes
//!
//! Nodes that create periodic events and manage timing-based operations within
//! the execution graph. These wsnl integrate with the runtime's timer system
//! to provide scheduled execution capabilities.

use crate::Control;
use crate::prelude::{Executor, Node, NodeBuilder};
use crate::runtime::TimerSource;

/// Creates a node that triggers periodically at fixed intervals.
///
/// This node acts as a periodic event source, broadcasting at regular time intervals.
/// It automatically registers itself with the timer system and re-schedules on each
/// trigger to maintain the periodic behavior.
///
/// # Arguments
/// * `executor` - The executor to create the node in
/// * `duration` - The time interval between triggers
///
/// # Returns
/// A node that broadcasts every `duration` period
///
/// # Behavior
/// - Starts immediately on first cycle (via yield_now in on_init)
/// - Always broadcasts when triggered (provides timing signal)
/// - Automatically re-registers timer for next period
/// - Provides consistent periodic timing regardless of processing delays
///
/// # Timer Management
/// The node manages its own timer lifecycle:
/// 1. Initial trigger via yield_now() to start immediately
/// 2. Each execution registers next timer at `current_time + duration`
/// 3. Runtime timer system handles the actual scheduling
///
/// # Examples
/// ```rust, ignore
/// // Heartbeat every second
/// let heartbeat = periodic_trigger_node(executor, Duration::from_secs(1));
///
/// // High-frequency sampling at 100Hz
/// let sampler = periodic_trigger_node(executor, Duration::from_millis(10));
///
/// // Connect to processing pipeline
/// let processor = NodeBuilder::new(Vec::new())
///     .triggered_by(&heartbeat)
///     .build(executor, |data, ctx| {
///         // Process on each heartbeat
///         Control::Broadcast
///     });
/// ```
///
/// # Notes
/// - Timer precision depends on the runtime's timer driver implementation
/// - Processing delays do not accumulate - each timer is scheduled from current time
/// - The node carries no data payload, serving purely as a timing signal
pub fn periodic_trigger_node(
    executor: &mut Executor,
    duration: std::time::Duration,
) -> Node<Option<TimerSource>> {
    NodeBuilder::new(None)
        .on_init(|ex, _, idx| {
            ex.yield_driver().yield_now(idx);
        })
        .build(executor, move |this, ctx| {
            let source = ctx.register_timer(ctx.current(), ctx.now() + duration);
            if this.is_some() {
                this.insert(source);
                Control::Broadcast
            } else {
                Control::Unchanged
            }
        })
}

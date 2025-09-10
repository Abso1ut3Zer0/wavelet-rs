//! High-performance graph-based stream processing runtime.
//!
//! The `runtime` module provides the core execution engine for wavelet's cooperative
//! stream processing model. Built around a computation graph where nodes represent
//! stream processors and edges define data dependencies, the runtime delivers
//! deterministic, low-latency execution without the overhead of async runtimes
//! or actor systems.
//!
//! # Architecture Overview
//!
//! ## Computation Model
//! - **Nodes**: Stateful stream processors that transform data
//! - **Relationships**: Define when nodes should execute (`Trigger` vs `Observe`)
//! - **Cooperative scheduling**: Nodes voluntarily yield control after processing
//! - **Dependency ordering**: Execution follows graph topology (depth-first, scheduled first)
//! - **Incremental computation**: Only recompute when dependencies actually change
//!
//! ## Core Components
//!
//! ### [`Executor`]
//! The central execution engine that orchestrates:
//! - Graph topology management and node lifecycle
//! - Event-driven scheduling (I/O, timers, yields)
//! - Dependency-ordered execution cycles
//! - Garbage collection and resource cleanup
//!
//! ### [`Node<T>`] and [`NodeBuilder<T>`]
//! Type-safe containers for node state with controlled mutation:
//! - **Data-oriented design**: Separate data (`T`) from behavior (cycle functions)
//! - **Controlled mutation**: Data changes only within cycle functions
//! - **Builder pattern**: Fluent API for configuring relationships and lifecycle
//!
//! ### [`Runtime<C, M>`]
//! Complete runtime orchestration combining:
//! - **Clock abstraction**: Consistent time across execution cycles
//! - **Execution modes**: Different CPU/latency trade-offs (`Spin`, `Sleep`, `Block`)
//! - **Runtime loops**: Automated execution patterns for different use cases
//!
//! ### Event System
//! Unified event handling for external stimulus:
//! - **I/O Events**: Network sockets, file handles, external notifications
//! - **Timer Events**: Time-based scheduling with precise expiration
//! - **Yield Events**: Immediate re-scheduling for continued processing
//!
//! # Design Principles
//!
//! ## Single-Threaded Cooperative Model
//! - **Predictable performance**: No hidden thread spawning or context switching
//! - **Deterministic execution**: Same inputs always produce the same execution order
//! - **Zero-cost abstractions**: Direct function calls without async overhead
//! - **Resource control**: Explicit management of CPU, memory, and I/O
//!
//! ## Data-Oriented Design
//! - **Type safety**: Compile-time guarantees about node data types
//! - **Memory efficiency**: Minimal indirection and cache-friendly layouts
//! - **Controlled mutation**: Runtime coordinates when and how data changes
//! - **Clear ownership**: Data lifecycle tied to node lifecycle
//!
//! ## Event-Driven Execution
//! - **External integration**: Clean interfaces to operating system events
//! - **Backpressure handling**: Natural flow control through graph topology
//! - **Resource efficiency**: Sleep when no work is available
//! - **Low latency**: Direct event dispatch without queueing overhead
//!
//! # Usage Patterns
//!
//! ## Basic Stream Processing
//! ```rust, ignore
//! use wavelet::runtime::*;
//!
//! let mut executor = Executor::new();
//!
//! // Create data source
//! let data_source = NodeBuilder::new(DataSource::new())
//!     .on_init(|executor, _, idx| {
//!         executor.yield_driver().yield_now(idx); // Start processing
//!     })
//!     .build(&mut executor, |source, ctx| {
//!         if let Some(data) = source.poll_data() {
//!             source.latest = data;
//!             Control::Broadcast // Notify downstream
//!         } else {
//!             Control::Unchanged
//!         }
//!     });
//!
//! // Create processor that reacts to data
//! let processor = NodeBuilder::new(Processor::new())
//!     .triggered_by(&data_source)
//!     .build(&mut executor, |proc, ctx| {
//!         proc.process_data();
//!         Control::Unchanged
//!     });
//!
//! // Run the graph
//! let runtime = Runtime::builder()
//!     .with_clock(PrecisionClock::new())
//!     .with_mode(Sleep::new(Duration::from_millis(1)))
//!     .build()?;
//!
//! runtime.run_forever();
//! ```
//!
//! ## I/O Integration
//! ```rust, ignore
//! let (network_node, notifier) = NodeBuilder::new(NetworkHandler::new())
//!     .build_with_notifier(&mut executor, |handler, ctx| {
//!         match handler.socket.try_read(&mut handler.buffer) {
//!             Ok(0) => Control::Sweep, // Connection closed
//!             Ok(n) => {
//!                 handler.process_bytes(n);
//!                 Control::Broadcast
//!             }
//!             Err(e) if e.kind() == ErrorKind::WouldBlock => {
//!                 // Re-register for readiness
//!                 handler.reregister_interest(ctx);
//!                 Control::Unchanged
//!             }
//!             Err(_) => Control::Sweep, // Connection error
//!         }
//!     })?;
//!
//! // External thread can wake the network node
//! notifier.notify()?;
//! ```
//!
//! ## Dynamic Graph Construction
//! ```rust, ignore
//! let spawner = NodeBuilder::new(DynamicSpawner::new())
//!     .build(&mut executor, |spawner, ctx| {
//!         if spawner.should_create_worker() {
//!             ctx.spawn_subgraph(|executor| {
//!                 let worker = NodeBuilder::new(Worker::new())
//!                     .triggered_by(&spawner.work_queue)
//!                     .build(executor, process_work);
//!             });
//!         }
//!         Control::Unchanged
//!     });
//! ```
//!
//! # Performance Characteristics
//!
//! - **Latency**: Sub-microsecond node execution overhead
//! - **Throughput**: Millions of events per second on modern hardware
//! - **Memory**: Predictable allocation patterns, minimal runtime overhead
//! - **CPU**: Efficient utilization with configurable sleep/spin strategies
//! - **Determinism**: Consistent performance across runs with same inputs
//!
//! # Target Applications
//!
//! The runtime excels in domains requiring:
//! - **Financial systems**: Low-latency trading, risk management, market data
//! - **Real-time analytics**: Live dashboards, alerting, stream aggregation
//! - **IoT processing**: Sensor data, device management, edge computing
//! - **Protocol handling**: Stateful network protocols, message parsing
//! - **Media processing**: Audio/video pipelines, real-time effects
//!
//! For request/response workloads or applications requiring automatic parallelism,
//! consider using async runtimes like Tokio alongside wavelet for the appropriate
//! components of your system.

pub mod clock;
pub mod event_driver;
pub mod executor;
mod garbage_collector;
pub mod graph;
pub mod node;
mod scheduler;

pub use clock::*;
pub use event_driver::*;
pub use executor::*;
pub use graph::*;
pub use node::*;

const MINIMUM_TIMER_PRECISION: std::time::Duration = std::time::Duration::from_millis(1);

/// Marker trait for runtime execution strategies.
///
/// Defines how the runtime should behave when no nodes are ready for execution.
/// Different execution modes provide different trade-offs between CPU usage,
/// latency, and power consumption.
pub trait ExecutionMode {}

/// Busy-wait execution mode that continuously polls without yielding CPU.
///
/// Provides the lowest possible latency at the cost of high CPU usage.
/// Best for latency-critical applications where CPU resources are dedicated.
pub struct Spin;

/// Sleep-based execution mode that yields CPU for a maximum duration.
///
/// Balances latency and CPU usage by sleeping for the shorter of:
/// - The configured maximum sleep duration
/// - Time until the next timer expires
///
/// Good for most applications that need reasonable latency without burning CPU.
pub struct Sleep(std::time::Duration);

/// Blocking execution mode that waits indefinitely for events.
///
/// Provides the most CPU-efficient operation by blocking until events occur
/// or timers expire. Higher latency but minimal CPU usage when idle.
/// Best for background processing or low-frequency event handling.
pub struct Block;
impl ExecutionMode for Spin {}
impl ExecutionMode for Sleep {}
impl ExecutionMode for Block {}

impl Sleep {
    /// Creates a new Sleep execution mode with the specified maximum duration.
    ///
    /// The duration must be at least 1ms due to normal OS sleep limitations.
    pub fn new(duration: std::time::Duration) -> Self {
        assert!(duration >= std::time::Duration::from_millis(1));
        Self(duration)
    }
}

/// Helper trait that implements single-cycle execution for different runtime configurations.
///
/// `CycleOnce` abstracts over the different timeout calculation strategies needed
/// for various clock and execution mode combinations. Each implementation handles:
/// - **Time snapshot**: Getting current time from the clock
/// - **Timeout calculation**: Determining how long to wait for I/O events
/// - **Error handling**: Converting I/O errors to executor state
///
/// This trait enables generic execution patterns like `run_forever()` while
/// allowing each runtime configuration to optimize its polling behavior.
///
/// # Implementation Strategies
///
/// - **`Spin`**: Always uses `Duration::ZERO` timeout for immediate polling
/// - **`Sleep(max_duration)`**: Uses the minimum of max duration and next timer
/// - **`Block`**: Waits indefinitely or until next timer (whichever comes first)
/// - **`TestClock`**: Always uses immediate polling regardless of execution mode
pub trait CycleOnce {
    /// Executes one complete cycle and returns the executor state.
    fn cycle_once(&mut self) -> ExecutorState;
}

/// Errors that can occur during runtime construction.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeBuilderError {
    #[error("no clock provided")]
    NoClock,
    #[error("no execution mode provided")]
    NoExecutionMode,
}

/// Builder for constructing a runtime with specific clock and execution mode.
///
/// Uses the type system to ensure both clock and execution mode are provided
/// before building the runtime.
pub struct RuntimeBuilder<C: Clock, M: ExecutionMode> {
    clock: Option<C>,
    mode: Option<M>,
}

impl<C: Clock, M: ExecutionMode> RuntimeBuilder<C, M> {
    pub const fn new() -> Self {
        Self {
            clock: None,
            mode: None,
        }
    }

    pub fn with_clock(mut self, clock: C) -> Self {
        self.clock = Some(clock);
        self
    }

    pub fn with_mode(mut self, mode: M) -> Self {
        self.mode = Some(mode);
        self
    }

    pub fn build(self) -> Result<Runtime<C, M>, RuntimeBuilderError> {
        let clock = self.clock.ok_or(RuntimeBuilderError::NoClock)?;
        let mode = self.mode.ok_or(RuntimeBuilderError::NoExecutionMode)?;
        Ok(Runtime {
            executor: Executor::new(),
            clock,
            mode,
        })
    }
}

#[cfg(feature = "testing")]
pub type TestRuntime = Runtime<TestClock, Spin>;
pub type RealtimeRuntime<M: ExecutionMode> = Runtime<PrecisionClock, M>;
pub type HistoricalRuntime = Runtime<HistoricalClock, Spin>;

/// A complete runtime instance that combines executor, clock, and execution mode.
///
/// The `Runtime` orchestrates the execution loop by:
/// - Using the clock to provide consistent time snapshots
/// - Running the executor with the configured execution mode
/// - Handling different clock types (Precision, Historical, Test) appropriately
///
/// The type parameters ensure compile-time guarantees about clock and mode
/// compatibility.
pub struct Runtime<C: Clock, M: ExecutionMode> {
    /// The core computation engine
    executor: Executor,

    /// Time source for execution cycles
    clock: C,

    /// Execution strategy for the main loop
    mode: M,
}

impl<C: Clock, M: ExecutionMode> Runtime<C, M> {
    pub const fn builder() -> RuntimeBuilder<C, M> {
        RuntimeBuilder::new()
    }

    pub fn executor(&mut self) -> &mut Executor {
        &mut self.executor
    }
}

impl<M: ExecutionMode> Runtime<PrecisionClock, M>
where
    Self: CycleOnce,
{
    pub fn run_forever(mut self) {
        while self.cycle_once().is_running() {
            // continue
        }
    }
}

impl Runtime<HistoricalClock, Spin> {
    pub fn run_until_completion(mut self) {
        while !self.clock.is_exhausted() {
            let state = self
                .executor
                .cycle(self.clock.trigger_time(), Some(std::time::Duration::ZERO))
                .unwrap_or(ExecutorState::Running);

            if state.is_terminated() {
                return;
            }
        }
    }
}

impl CycleOnce for Runtime<PrecisionClock, Spin> {
    #[inline(always)]
    fn cycle_once(&mut self) -> ExecutorState {
        self.executor
            .cycle(self.clock.trigger_time(), Some(std::time::Duration::ZERO))
            .unwrap_or(ExecutorState::Running)
    }
}

impl CycleOnce for Runtime<PrecisionClock, Sleep> {
    #[inline(always)]
    fn cycle_once(&mut self) -> ExecutorState {
        let now = self.clock.trigger_time();
        let duration = self
            .executor
            .next_timer()
            .map(|when| (when.saturating_duration_since(now.instant)).min(self.mode.0));
        self.executor
            .cycle(now, duration)
            .unwrap_or(ExecutorState::Running)
    }
}

impl CycleOnce for Runtime<PrecisionClock, Block> {
    #[inline(always)]
    fn cycle_once(&mut self) -> ExecutorState {
        let now = self.clock.trigger_time();
        let duration = self.executor.next_timer().map(|when| {
            when.saturating_duration_since(now.instant)
                .max(MINIMUM_TIMER_PRECISION)
        });
        self.executor
            .cycle(now, duration)
            .unwrap_or(ExecutorState::Running)
    }
}

#[cfg(feature = "testing")]
impl Runtime<TestClock, Spin> {
    pub fn run_one_cycle(&mut self) -> ExecutorState {
        self.cycle_once()
    }

    pub fn advance_clock(&mut self, duration: std::time::Duration) {
        self.clock.advance(duration);
    }
}

#[cfg(feature = "testing")]
impl CycleOnce for Runtime<TestClock, Spin> {
    #[inline(always)]
    fn cycle_once(&mut self) -> ExecutorState {
        self.executor
            .cycle(self.clock.trigger_time(), Some(std::time::Duration::ZERO))
            .unwrap_or(ExecutorState::Running)
    }
}

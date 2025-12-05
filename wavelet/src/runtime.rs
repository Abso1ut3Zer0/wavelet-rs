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
//! ### [`Runtime<C>`]
//! Complete runtime orchestration combining:
//! - **Clock abstraction**: Consistent time across execution cycles
//! - **Execution modes**: Different CPU/latency trade-offs (`Spin`, `Park`)
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
//! let runtime = RealtimeRuntime::new(ExecutionMode::Park);
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

#[cfg(feature = "signals")]
use std::sync::{Arc, atomic::AtomicBool};

pub use clock::*;
use enum_as_inner::EnumAsInner;
pub use event_driver::*;
pub use executor::*;
pub use graph::*;
pub use node::*;
pub use scheduler::*;

/// Execution mode for the real-time runtime.
///
/// Spin mode never parks the thread. Trades off
/// high cpu usage for the lowest possible latency.
///
/// Park mode will park the thread if no event
/// is available at the time of the poll. This
/// trades off latency for energy usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumAsInner)]
pub enum ExecutionMode {
    Spin,
    Park,
}

#[cfg(feature = "testing")]
pub type TestRuntime = Runtime<TestClock>;
#[allow(type_alias_bounds)]
pub type RealtimeRuntime = Runtime<PrecisionClock>;
pub type HistoricalRuntime = Runtime<HistoricalClock>;

/// A complete runtime instance that combines executor, clock, and execution mode.
///
/// The `Runtime` orchestrates the execution loop by:
/// - Using the clock to provide consistent time snapshots
/// - Running the executor with the configured execution mode
/// - Handling different clock types (Precision, Historical, Test) appropriately
///
/// The type parameters ensure compile-time guarantees about clock and mode
/// compatibility.
pub struct Runtime<C: Clock> {
    /// The core computation engine
    executor: Executor,

    /// Time source for execution cycles
    clock: C,

    /// Execution strategy for the main loop
    mode: ExecutionMode,

    /// Shutdown flag
    #[cfg(feature = "signals")]
    shutdown: Arc<AtomicBool>,
}

impl<C: Clock> Runtime<C> {
    pub const fn executor(&mut self) -> &mut Executor {
        &mut self.executor
    }
}

#[cfg(feature = "testing")]
impl Runtime<TestClock> {
    pub fn new() -> Self {
        Self {
            executor: Executor::new(ExecutionMode::Spin),
            clock: TestClock::new(),
            mode: ExecutionMode::Spin,
            #[cfg(feature = "signals")]
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn run_one_cycle(&mut self) -> ExecutorState {
        self.executor
            .cycle(&mut self.clock, Some(std::time::Duration::ZERO))
            .unwrap_or(ExecutorState::Running)
    }

    pub fn advance_clock(&mut self, duration: std::time::Duration) {
        self.clock.advance(duration);
    }
}

impl Runtime<HistoricalClock> {
    pub fn new(interval: Interval) -> Self {
        Self {
            executor: Executor::new(ExecutionMode::Spin),
            clock: HistoricalClock::new(interval),
            mode: ExecutionMode::Spin,
            #[cfg(feature = "signals")]
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn run_until_completion(mut self) {
        while !self.clock.is_exhausted() {
            let state = self
                .executor
                .cycle(&mut self.clock, Some(std::time::Duration::ZERO))
                .unwrap_or(ExecutorState::Running);

            if state.is_terminated() {
                return;
            }
        }
    }
}

impl Runtime<PrecisionClock> {
    pub fn new(mode: ExecutionMode) -> Self {
        Self {
            executor: Executor::new(mode),
            clock: PrecisionClock::new(),
            mode,
            #[cfg(feature = "signals")]
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_config(cfg: EventDriverConfig, mode: ExecutionMode) -> Self {
        Self {
            executor: Executor::with_config(cfg, mode),
            clock: PrecisionClock::new(),
            mode,
            #[cfg(feature = "signals")]
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }

    #[cfg(feature = "signals")]
    pub fn enable_graceful_shutdown(&self) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            use signal_hook::consts::{SIGINT, SIGTERM};
            use signal_hook::flag;
            flag::register(SIGINT, self.shutdown.clone())?;
            flag::register(SIGTERM, self.shutdown.clone())?;
        }

        #[cfg(not(unix))]
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "signal handling is only supported on unix platforms",
            ));
        }

        Ok(())
    }

    #[cfg(feature = "signals")]
    pub fn run_forever(&mut self) {
        use std::sync::atomic::Ordering;
        while !self.shutdown.load(Ordering::Relaxed)
            && let ExecutorState::Running = self
                .executor
                .cycle(
                    &mut self.clock,
                    match self.mode {
                        ExecutionMode::Park => None,
                        ExecutionMode::Spin => Some(std::time::Duration::ZERO),
                    },
                )
                .unwrap_or(ExecutorState::Running)
        {
            // continue
        }
    }

    #[cfg(not(feature = "signals"))]
    pub fn run_forever(&mut self) {
        while let ExecutorState::Running = self
            .executor
            .cycle(
                &mut self.clock,
                match self.mode {
                    ExecutionMode::Park => None,
                    ExecutionMode::Spin => Some(std::time::Duration::ZERO),
                },
            )
            .unwrap_or(ExecutorState::Running)
        {
            // continue
        }
    }
}

//! Dependency injection factories for configurable graph construction.
//!
//! The `factories` module provides build-time dependency injection for stream processing
//! graphs, enabling the same graph topology to use different node implementations based
//! on configuration, environment, or testing requirements.
//!
//! # The Problem
//!
//! Stream processing applications often need to:
//! - Use different data sources in dev/staging/production environments
//! - Swap real implementations with mocks during testing
//! - Share expensive resources (database connections, file handles) across nodes
//! - Configure behavior based on runtime parameters without changing graph structure
//!
//! Traditional approaches either hardcode dependencies (inflexible) or use runtime
//! service location (performance overhead, late binding errors).
//!
//! # The Solution: Factory Pattern
//!
//! Factories separate **what to create** from **when to create**, providing:
//!
//! ## Build-Time Configuration
//! ```rust, ignore
//! let data_source = match config.environment {
//!     Environment::Production => KeyedFactory::default()
//!         .attach(|executor, symbol| create_live_feed(executor, symbol)),
//!     Environment::Test => KeyedFactory::default()
//!         .attach(|executor, symbol| create_mock_feed(executor, symbol)),
//! };
//! ```
//!
//! ## Consistent Graph Construction
//! ```rust, ignore
//! // Same code works with any configured factory
//! let feed = data_source.get(&mut executor, "EURUSD".to_string());
//! let processor = NodeBuilder::new(PriceProcessor::new())
//!     .triggered_by(&feed)  // Type-safe, regardless of implementation
//!     .build(&mut executor, process_prices);
//! ```
//!
//! ## Performance Benefits
//! - **Zero runtime overhead**: All binding happens at graph construction time
//! - **Efficient caching**: Expensive nodes created once and reused
//! - **Type safety**: Compile-time guarantees about node compatibility
//! - **Predictable execution**: No hidden service location during node cycles
//!
//! # Factory Types
//!
//! ## [`SimpleFactory<T>`]
//! Creates exactly one instance per factory (singleton pattern):
//! - Configuration services
//! - Shared loggers or monitors
//! - Global timers or heartbeats
//! - Expensive initialization (loading datasets, establishing connections)
//!
//! ## [`KeyedFactory<K, T>`]
//! Creates one instance per unique key (multi-instance with caching):
//! - Per-symbol market data feeds
//! - Per-database connection pools
//! - Per-user session handlers
//! - Any resource that varies by parameter
//!
//! # Architecture Integration
//!
//! Factories integrate with the wavelet runtime's cooperative scheduling model:
//!
//! 1. **Build Phase**: Factories configure different implementations
//! 2. **Graph Construction**: Nodes are created through factory calls
//! 3. **Runtime Phase**: Graph topology is fixed, execution is deterministic
//! 4. **Dynamic Spawning**: Runtime nodes can still create subgraphs as needed
//!
//! This provides the flexibility of dependency injection without compromising
//! the performance and predictability of the core execution model.
//!
//! # Example: Multi-Environment Trading System
//!
//! ```rust, ignore
//! use wavelet::factories::*;
//!
//! fn build_trading_graph(config: &Config, executor: &mut Executor) {
//!     // Configure data source based on environment
//!     let market_data = match config.environment {
//!         Environment::Live => KeyedFactory::default()
//!             .attach(|executor, symbol| {
//!                 create_live_market_feed(executor, symbol, &config.api_key)
//!             }),
//!         Environment::Backtest => KeyedFactory::default()
//!             .attach(|executor, symbol| {
//!                 create_historical_replay(executor, symbol, config.date_range)
//!             }),
//!         Environment::Test => KeyedFactory::default()
//!             .attach(|executor, symbol| {
//!                 create_deterministic_mock(executor, symbol)
//!             }),
//!     };
//!
//!     // Configure risk management
//!     let risk_service = SimpleFactory::default()
//!         .attach(|executor| {
//!             create_risk_manager(executor, config.max_position_size)
//!         });
//!
//!     // Build graph using configured factories
//!     let eurusd_feed = market_data.get(executor, "EURUSD".to_string());
//!     let gbpusd_feed = market_data.get(executor, "GBPUSD".to_string());
//!     let risk_manager = risk_service.get(executor);
//!
//!     let strategy = NodeBuilder::new(TradingStrategy::new())
//!         .triggered_by(&eurusd_feed)
//!         .triggered_by(&gbpusd_feed)
//!         .observer_of(&risk_manager)
//!         .build(executor, execute_strategy);
//!
//!     // Same graph structure, different behavior based on config
//! }
//! ```
//!
//! This pattern enables sophisticated applications that can adapt to different
//! environments while maintaining the performance and determinism of the
//! underlying stream processing runtime.

pub use keyed::*;
pub use simple::*;
mod keyed;
mod simple;

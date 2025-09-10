# Wavelet

A high-performance, graph-based stream processing runtime for Rust that delivers deterministic execution with zero-cost
abstractions.

![Wavelet](wavelet.png)

## Overview

Wavelet is designed for applications that need predictable, low-latency stream processing without the overhead of async
runtimes or actor systems. Built around a computation graph where nodes represent stream processors and edges define
data dependencies, wavelet provides:

- **Deterministic execution** - Same inputs always produce the same execution order
- **Cooperative scheduling** - Dependency-ordered execution without thread overhead
- **Event integration** - Unified I/O, timer, and yield event handling
- **Dependency injection** - Build-time configuration for different environments
- **Zero-cost abstractions** - Direct function calls with no hidden allocations

## Quick Start

Add wavelet to your `Cargo.toml`:

```toml
[dependencies]
wavelet = "0.1"
```

Create a simple stream processing graph:

```rust
use wavelet::prelude::*;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut executor = Executor::new();

    // Create a data source
    let source = NodeBuilder::new(0u64)
        .on_init(|executor, _, idx| {
            executor.yield_driver().yield_now(idx);
        })
        .build(&mut executor, |counter, _ctx| {
            *counter += 1;
            println!("Source: {}", counter);
            Control::Broadcast
        });

    // Create a processor that reacts to the source
    let _processor = NodeBuilder::new(String::new())
        .triggered_by(&source)
        .build(&mut executor, |state, _ctx| {
            *state = format!("Processed: {}", source.borrow());
            println!("{}", state);
            Control::Unchanged
        });

    // Run the graph
    let runtime = Runtime::builder()
        .with_clock(PrecisionClock::new())
        .with_mode(Sleep::new(Duration::from_millis(1)))
        .build()?;

    runtime.run_forever();
}
```

## Architecture

### Core Concepts

- **Nodes**: Stateful stream processors that transform data
- **Relationships**: Define execution dependencies (`Trigger` vs `Observe`)
- **Cooperative Scheduling**: Nodes yield control after processing
- **Event System**: I/O, timer, and yield events drive execution

### Key Components

```rust
// Build a computation graph
let processor = NodeBuilder::new(ProcessorState::new())
    .triggered_by(&data_source)     // Execute when source changes
    .observer_of(&config_node)      // Read config but don't auto-execute
    .with_name("processor".into())
    .build(&mut executor, |state, ctx| {
        // Process data with mutable access to state
        state.process(data_source.borrow().latest_data());
        Control::Broadcast  // Notify downstream nodes
    });
```

## Features

Wavelet uses Cargo features to enable different functionality:

- `runtime` (default) - Core execution engine
- `factories` - Dependency injection for build-time configuration
- `full` - All features enabled

```toml
[dependencies]
wavelet = { version = "0.1", features = ["full"] }
```

### Dependency Injection with Factories

Configure different implementations for different environments:

```rust
use wavelet::factories::*;

let data_source = match environment {
    Environment::Production => KeyedFactory::default()
        .attach(|executor, symbol| create_live_feed(executor, symbol)),
    Environment::Test => KeyedFactory::default()
        .attach(|executor, symbol| create_mock_feed(executor, symbol)),
};

// Same graph construction code works with any configured factory
let feed = data_source.get(&mut executor, "EURUSD".to_string());
```

## Performance

- **Latency**: Sub-microsecond node execution overhead
- **Throughput**: Millions of events per second
- **Memory**: Predictable allocation patterns
- **CPU**: Configurable sleep/spin strategies for different workloads

## Use Cases

Wavelet excels in domains requiring deterministic, low-latency processing:

- **Financial Systems** - Trading engines, risk management, market data processing
- **Real-time Analytics** - Live dashboards, alerting, stream aggregation
- **IoT Processing** - Sensor data, device management, edge computing
- **Protocol Handling** - Stateful network protocols, message parsing
- **Media Processing** - Audio/video pipelines, real-time effects

## Examples

See the `examples/` directory for complete applications:

- `basic_stream` - Simple data processing pipeline
- `market_data` - Financial data processing with multiple symbols
- `network_server` - TCP server with per-connection processing
- `dependency_injection` - Using factories for environment configuration

## Documentation

- [API Documentation](https://docs.rs/wavelet)
- [Architecture Guide](docs/architecture.md)
- [Performance Guide](docs/performance.md)
- [Migration Guide](docs/migration.md)

## Comparison with Other Frameworks

| Framework | Model             | Determinism | Latency | Complexity |
|-----------|-------------------|-------------|---------|------------|
| Wavelet   | Cooperative Graph | ✅           | < 1μs   | Low        |
| Tokio     | Async Tasks       | ❌           | ~10μs   | Medium     |
| Actix     | Actor System      | ❌           | ~5μs    | High       |
| Timely    | Dataflow          | ✅           | ~1μs    | High       |

## Contributing

We welcome contributions! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.

## Acknowledgments

Wavelet draws inspiration from:

- Timely Dataflow's deterministic execution model
- Reactive Extensions' event composition patterns
- Traditional signal processing graph architectures
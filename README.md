# Wavelet

A high-performance, graph-based stream processing runtime for Rust that delivers deterministic execution with zero-cost
abstractions.

![Wavelet](wavelet.png)

## Overview

Wavelet is designed for applications that need predictable, low-latency stream processing without the overhead of async
runtimes or actor systems. Built around a computation graph where nodes represent stream processors and edges define
data dependencies, wavelet provides:

- **Deterministic execution** - Same inputs always produce the same execution order
- **Dependency-ordered processing** - Guaranteed that parent nodes are always processed before their children
- **Event integration** - Unified I/O, timer, and yield event handling
- **Dependency injection** - Build-time configuration for different environments

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
    let runtime = Runtime::builder()
        .with_clock(PrecisionClock::new())
        .with_mode(Sleep::new(Duration::from_millis(1)))
        .build()?;

    // Create a data source
    let source = NodeBuilder::new(0u64)
        .on_init(|executor, _, idx| {
            executor.yield_driver().yield_now(idx);
        })
        .build(runtime.executor(), |counter, _ctx| {
            *counter += 1;
            println!("Source: {}", counter);
            Control::Broadcast
        });

    // Create a processor that reacts to the source
    let _processor = NodeBuilder::new(String::new())
        .triggered_by(&source)
        .build(runtime.executor(), |state, _ctx| {
            *state = format!("Processed: {}", source.borrow());
            println!("{}", state);
            Control::Unchanged
        });

    // Run the graph
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

```rust, ignore
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

```rust, ignore
use wavelet::factories::*;

let data_source = match environment {
Environment::Production => KeyedFactory::default ()
.attach( | executor, symbol| create_live_feed(executor, symbol)),
Environment::Test => KeyedFactory::default ()
.attach( | executor, symbol| create_mock_feed(executor, symbol)),
};

// Same graph construction code works with any configured factory
let feed = data_source.get( & mut executor, "EURUSD".to_string());
```

## Use Cases

Wavelet excels in domains requiring deterministic, low-latency processing:

- **Financial Systems** - Trading engines, risk management, market data processing
- **Real-time Analytics** - Live dashboards, alerting, stream aggregation
- **IoT Processing** - Sensor data, device management, edge computing
- **Protocol Handling** - Stateful network protocols, message parsing

## Contributing

We welcome contributions! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## License

MIT License ([LICENSE](LICENSE))
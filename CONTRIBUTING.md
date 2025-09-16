# Contributing to Wavelet

Thank you for your interest in contributing to Wavelet! This document provides guidelines and information for
contributors.

## Getting Started

### Prerequisites

- Rust 1.70+ (we use modern Rust features)
- Basic understanding of graph algorithms and event-driven programming
- Familiarity with `mio`, `petgraph`, and async concepts (helpful but not required)

### Setting Up Development Environment

```bash
git clone https://github.com/yourusername/wavelet
cd wavelet
cargo test --all-features
cargo doc --all-features --open
```

## Project Structure

```
wavelet/
├── src/
│   ├── lib.rs                    # Main library exports
│   ├── runtime.rs                # Core runtime and builder
│   ├── executor.rs               # Execution engine and context
│   ├── scheduler.rs              # Multi-depth node scheduling
│   ├── graph.rs                  # Computation graph management
│   ├── node.rs                   # Node building and lifecycle
│   ├── garbage_collector.rs      # Automatic node cleanup
│   ├── clock.rs                  # Clock trait and TriggerTime
│   ├── channel.rs                # Cross-thread communication (feature-gated)
│   ├── factory.rs                # Dependency injection factories (feature-gated)
│   ├── testing.rs                # Test utilities (feature-gated)
│   ├── clock/
│   │   ├── historical.rs         # Historical time replay
│   │   ├── precision_clock.rs    # High-precision timing
│   │   └── test_clock.rs         # Deterministic test timing
│   └── event_driver/
│       ├── io_driver.rs          # mio-based I/O event handling
│       ├── timer_driver.rs       # Timer event scheduling
│       └── yield_driver.rs       # Immediate re-scheduling
├── examples/                     # Usage examples
└── tests/                        # Integration tests
```

## Areas for Contribution

### 🚀 High Priority

1. **Networking Module** - WebSocket, UDP, TCP protocol implementations
2. **Standard Library Nodes** - Common processing patterns (aggregators, windows, filters)
3. **Examples and Documentation** - Real-world usage patterns
4. **Performance Optimization** - Benchmarking and optimization opportunities

### 🔬 Research Areas

1. **Observability** - Metrics, tracing, and logging around nodes & subgraphs
2. **Advanced Scheduling** - Priority-based or adaptive scheduling algorithms
3. **Multi-threading Support** - Exploring parallel execution models

### 🐛 Always Welcome

- Bug fixes and edge case handling
- Test coverage improvements
- Documentation clarifications
- Performance improvements

## Contribution Guidelines

### Code Quality

- **Tests Required**: All new features must include comprehensive tests
- **Documentation**: Public APIs must be documented with examples
- **Formatting**: Use `cargo fmt` before committing

### Design Principles

1. **Single-threaded First**: Maintain the deterministic execution model
2. **Predictable performance**: Self explanatory
3. **Composability**: Features should work well together, and orthogonal
4. **Memory Safety**: Leverage Rust's ownership model effectively

### Testing Strategy

```bash
# Run all tests
cargo test --all-features
```

## Making Changes

### Workflow

1. **Fork** the repository
2. **Create a feature branch**: `git checkout -b feature/networking-module`
3. **Write tests** (Rigorous tests are always preferred)
4. **Implement your changes**
5. **Update documentation** if needed
6. **Run the test suite**: `cargo test --all-features`
7. **Submit a pull request**

### Commit Messages

Use conventional commits:

```
feat: add WebSocket support to networking module
fix: resolve scheduler deadlock in edge case
docs: improve factory pattern examples
test: add integration tests for timer driver
```

### Pull Request Guidelines

- **Clear description** of what you're changing and why
- **Link to issues** if applicable
- **Include tests** demonstrating the feature/fix
- **Update documentation** for user-facing changes
- **Keep PRs focused** - one feature or fix per PR

## Architecture Deep Dive

### Key Components

- **Executor**: Manages the execution cycle and coordinates all drivers
- **Scheduler**: Multi-depth queue ensuring correct execution order
- **Graph**: Maintains node relationships and handles propagation
- **Event Drivers**: I/O, timer, and yield event integration
- **Factories**: Dependency injection and graph construction patterns

### Critical Invariants

1. **Epoch Deduplication**: Prevents duplicate scheduling within cycles
2. **Deterministic Scheduling**: Execution order must be predictable and follow dependency ordering
3. **Memory Safety**: No dangling references in the node graph

## Getting Help

- **GitHub Issues**: For bugs, feature requests, and questions
- **Discussions**: For design discussions and architecture questions
- **Code Review**: Don't hesitate to ask for feedback on approaches

## Code of Conduct

- Be respectful and constructive in all interactions
- Focus on technical merit in discussions
- Help newcomers learn the codebase
- Celebrate diverse perspectives and approaches

## Recognition

All contributors will be acknowledged in:

- `CONTRIBUTORS.md` file
- Release notes for significant contributions
- Documentation credits where appropriate

---

Thank you for helping make Wavelet better! 🌊
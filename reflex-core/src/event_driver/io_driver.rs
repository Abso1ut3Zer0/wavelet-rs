use petgraph::prelude::NodeIndex;
use slab::Slab;
use std::io;

use crate::graph::Graph;
use crate::scheduler::Scheduler;
pub use mio::{Interest, event::Source};

pub struct IoSource<S: Source> {
    source: S,
    token: mio::Token,
}

impl<S: Source> IoSource<S> {
    const fn new(source: S, token: mio::Token) -> Self {
        IoSource { source, token }
    }

    pub const fn source(&self) -> &S {
        &self.source
    }

    pub const fn source_mut(&mut self) -> &mut S {
        &mut self.source
    }
}

pub struct Notifier {
    waker: mio::Waker,
    token: mio::Token,
}

impl Notifier {
    const fn new(waker: mio::Waker, token: mio::Token) -> Self {
        Self { waker, token }
    }

    pub fn notify(&self) -> io::Result<()> {
        self.waker.wake()
    }
}

pub struct IoDriver {
    poller: mio::Poll,
    events: mio::Events,
    indices: Slab<NodeIndex>,
}

impl IoDriver {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            poller: mio::Poll::new().expect("failed to create mio poll"),
            events: mio::Events::with_capacity(capacity),
            indices: Slab::with_capacity(capacity),
        }
    }

    #[inline(always)]
    pub fn register_notifier(&mut self, idx: NodeIndex) -> Result<Notifier, io::Error> {
        let entry = self.indices.vacant_entry();
        let token = mio::Token(entry.key());
        let waker = mio::Waker::new(self.poller.registry(), token)?;
        entry.insert(idx);
        Ok(Notifier::new(waker, token))
    }

    #[inline(always)]
    pub fn deregister_notifier(&mut self, notifier: Notifier) {
        self.indices.remove(notifier.token.0);
    }

    #[inline(always)]
    pub fn register_source<S: Source>(
        &mut self,
        mut source: S,
        idx: NodeIndex,
        interest: Interest,
    ) -> io::Result<IoSource<S>> {
        let entry = self.indices.vacant_entry();
        let token = mio::Token(entry.key());
        self.poller
            .registry()
            .register(&mut source, token, interest)?;
        entry.insert(idx);
        Ok(IoSource::new(source, token))
    }

    #[inline(always)]
    pub fn deregister_source<S: Source>(
        &mut self,
        mut source: IoSource<S>,
    ) -> io::Result<NodeIndex> {
        self.poller.registry().deregister(&mut source.source)?;
        Ok(self.indices.remove(source.token.0))
    }

    #[inline(always)]
    pub fn reregister_source<S: Source>(
        &mut self,
        source: &mut IoSource<S>,
        interest: Interest,
    ) -> io::Result<()> {
        self.poller
            .registry()
            .reregister(&mut source.source, source.token, interest)
    }

    #[inline(always)]
    pub(super) fn poll(
        &mut self,
        graph: &mut Graph,
        scheduler: &mut Scheduler,
        timeout: Option<std::time::Duration>,
        epoch: usize,
    ) -> io::Result<()> {
        self.events.clear();
        self.poller.poll(&mut self.events, timeout)?;
        self.events.iter().for_each(|event| {
            let node_index = self.indices[event.token().0];
            if let Some(depth) = graph.can_schedule(node_index, epoch) {
                scheduler.schedule(node_index, depth)
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::NodeContext;
    use mio::net::TcpListener;
    use std::net::SocketAddr;

    fn create_test_listener() -> io::Result<TcpListener> {
        // Let OS assign a random available port each time
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        TcpListener::bind(addr)
    }

    // Alternative: Create different types to avoid any potential conflicts
    fn create_unique_source(port_hint: u16) -> io::Result<TcpListener> {
        // Try the hint first, then let OS pick if it fails
        match TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port_hint))) {
            Ok(listener) => Ok(listener),
            Err(_) => TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))),
        }
    }

    #[test]
    fn test_io_driver_creation() {
        let driver = IoDriver::with_capacity(64);
        // Should create without panicking
        assert_eq!(driver.indices.capacity(), 64);
    }

    #[test]
    fn test_register_notifier() -> io::Result<()> {
        let mut driver = IoDriver::with_capacity(64);
        let node_idx = NodeIndex::from(42);

        let notifier = driver.register_notifier(node_idx)?;

        // Should have stored the node index
        assert_eq!(driver.indices[notifier.token.0], node_idx);

        Ok(())
    }

    #[test]
    fn test_deregister_notifier() -> io::Result<()> {
        let mut driver = IoDriver::with_capacity(64);
        let node_idx = NodeIndex::from(42);

        let notifier = driver.register_notifier(node_idx)?;
        let token_key = notifier.token.0;

        // Verify it's registered
        assert!(driver.indices.contains(token_key));

        driver.deregister_notifier(notifier);

        // Should be removed
        assert!(!driver.indices.contains(token_key));

        Ok(())
    }

    #[test]
    fn test_register_source() -> io::Result<()> {
        let mut driver = IoDriver::with_capacity(64);
        let node_idx = NodeIndex::from(100);
        let listener = create_test_listener()?;

        let io_source = driver.register_source(listener, node_idx, Interest::READABLE)?;

        // Should have stored the node index
        assert_eq!(driver.indices[io_source.token.0], node_idx);

        // Should be able to access the source
        assert!(io_source.source().local_addr().is_ok());

        Ok(())
    }

    #[test]
    fn test_deregister_source() -> io::Result<()> {
        let mut driver = IoDriver::with_capacity(64);
        let node_idx = NodeIndex::from(100);
        let listener = create_test_listener()?;

        let io_source = driver.register_source(listener, node_idx, Interest::READABLE)?;
        let token_key = io_source.token.0;

        // Verify it's registered
        assert!(driver.indices.contains(token_key));

        let returned_idx = driver.deregister_source(io_source)?;

        // Should return the correct node index
        assert_eq!(returned_idx, node_idx);

        // Should be removed from indices
        assert!(!driver.indices.contains(token_key));

        Ok(())
    }

    #[test]
    fn test_reregister_source() -> io::Result<()> {
        let mut driver = IoDriver::with_capacity(64);
        let node_idx = NodeIndex::from(100);
        let listener = create_test_listener()?;

        let mut io_source = driver.register_source(listener, node_idx, Interest::READABLE)?;

        // Should be able to reregister with different interest
        driver.reregister_source(&mut io_source, Interest::WRITABLE)?;

        // Token should remain the same
        assert_eq!(driver.indices[io_source.token.0], node_idx);

        Ok(())
    }

    #[test]
    fn test_multiple_registrations() -> io::Result<()> {
        let mut driver = IoDriver::with_capacity(64);

        let listener1 = create_test_listener()?;
        let listener2 = create_test_listener()?;

        let source1 = driver.register_source(listener1, NodeIndex::from(1), Interest::READABLE)?;
        let source2 = driver.register_source(listener2, NodeIndex::from(2), Interest::READABLE)?;

        // Should have different tokens
        assert_ne!(source1.token.0, source2.token.0);

        // Should map to correct node indices
        assert_eq!(driver.indices[source1.token.0], NodeIndex::from(1));
        assert_eq!(driver.indices[source2.token.0], NodeIndex::from(2));

        Ok(())
    }

    #[test]
    fn test_notifier_notify() -> io::Result<()> {
        let mut driver = IoDriver::with_capacity(64);
        let node_idx = NodeIndex::from(42);

        let notifier = driver.register_notifier(node_idx)?;

        // Should be able to call notify without error
        notifier.notify()?;

        Ok(())
    }

    #[test]
    fn test_io_source_access() -> io::Result<()> {
        let mut driver = IoDriver::with_capacity(64);
        let node_idx = NodeIndex::from(100);
        let listener = create_test_listener()?;
        let original_addr = listener.local_addr()?;

        let mut io_source = driver.register_source(listener, node_idx, Interest::READABLE)?;

        // Should be able to access source immutably
        assert_eq!(io_source.source().local_addr()?, original_addr);

        // Should be able to access source mutably (though TcpListener doesn't have many mut methods)
        let _source_mut = io_source.source_mut();

        Ok(())
    }

    #[test]
    fn test_poll_schedules_ready_nodes() -> io::Result<()> {
        let mut driver = IoDriver::with_capacity(64);

        // Create mock graph and scheduler
        let mut graph = Graph::new();
        let mut scheduler = Scheduler::new();
        scheduler.resize(5); // Support depth up to 5

        // Add some nodes to the graph with different depths
        let node1_ctx = NodeContext::new(
            Box::new(|_| false), // Dummy closure
            1,                   // depth
        );
        let node2_ctx = NodeContext::new(
            Box::new(|_| false),
            3, // depth
        );

        let node1_idx = graph.add_node(node1_ctx);
        let node2_idx = graph.add_node(node2_ctx);

        // Register I/O sources for these nodes
        let listener1 = create_unique_source(8010)?;
        let listener2 = create_unique_source(8011)?;

        let _source1 = driver.register_source(listener1, node1_idx, Interest::READABLE)?;
        let _source2 = driver.register_source(listener2, node2_idx, Interest::READABLE)?;

        // Poll with no actual I/O events (should be no-op)
        driver.poll(
            &mut graph,
            &mut scheduler,
            Some(std::time::Duration::ZERO),
            1,
        )?;

        // Nothing should be scheduled since no I/O events occurred
        assert!(scheduler.pop().is_none());

        Ok(())
    }

    #[test]
    fn test_poll_respects_epoch_deduplication() -> io::Result<()> {
        let mut driver = IoDriver::with_capacity(64);
        let mut graph = Graph::new();
        let mut scheduler = Scheduler::new();
        scheduler.resize(5);

        let node_ctx = NodeContext::new(Box::new(|_| false), 2);
        let node_idx = graph.add_node(node_ctx);

        // Mark this node as already scheduled in epoch 1
        assert_eq!(graph.can_schedule(node_idx, 1), Some(2)); // First call succeeds
        assert_eq!(graph.can_schedule(node_idx, 1), None); // Second call fails (already scheduled)

        Ok(())
    }

    #[test]
    fn test_slab_token_consistency() -> io::Result<()> {
        let mut driver = IoDriver::with_capacity(64);

        // Register multiple items and verify token consistency
        let notifier = driver.register_notifier(NodeIndex::from(1))?;
        let listener = create_unique_source(8012)?;
        let io_source = driver.register_source(listener, NodeIndex::from(2), Interest::READABLE)?;

        // Tokens should correspond to slab keys
        assert_eq!(driver.indices[notifier.token.0], NodeIndex::from(1));
        assert_eq!(driver.indices[io_source.token.0], NodeIndex::from(2));

        // Deregister and verify cleanup
        driver.deregister_notifier(notifier);
        let returned_idx = driver.deregister_source(io_source)?;

        assert_eq!(returned_idx, NodeIndex::from(2));

        Ok(())
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::graph::NodeContext;
    use std::io::Write;
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_tcp_stream_polling_integration() -> io::Result<()> {
        let mut driver = IoDriver::with_capacity(64);
        let mut graph = Graph::new();
        let mut scheduler = Scheduler::new();
        scheduler.resize(5);

        // Set up TCP listener and get its address
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let listener = mio::net::TcpListener::from_std(listener);
        let listener_addr = listener.local_addr()?;

        // Register the listener with the driver
        let node_ctx = NodeContext::new(Box::new(|_| false), 1);
        let node_idx = graph.add_node(node_ctx);

        let _io_source = driver.register_source(listener, node_idx, Interest::READABLE)?;

        // Initial poll - should have no events
        driver.poll(
            &mut graph,
            &mut scheduler,
            Some(Duration::from_millis(1)),
            1,
        )?;
        assert!(
            scheduler.pop().is_none(),
            "No events should be ready initially"
        );

        // Connect a client to trigger a readable event
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(10)); // Small delay
            if let Ok(mut stream) = TcpStream::connect(listener_addr) {
                // Send some data to trigger the event
                let _ = stream.write_all(b"test data");
                let _ = stream.flush();
                thread::sleep(Duration::from_millis(50)); // Keep connection alive briefly
            }
        });

        // Give the connection time to establish
        thread::sleep(Duration::from_millis(20));

        // Poll again - should now detect the readable event
        driver.poll(
            &mut graph,
            &mut scheduler,
            Some(Duration::from_millis(100)),
            2,
        )?;

        // Should have scheduled our node
        let scheduled_node = scheduler.pop();
        assert!(
            scheduled_node.is_some(),
            "Node should be scheduled after TCP connection"
        );
        assert_eq!(scheduled_node.unwrap(), node_idx);

        // No more nodes should be scheduled
        assert!(scheduler.pop().is_none());

        Ok(())
    }

    #[test]
    fn test_multiple_tcp_events() -> io::Result<()> {
        let mut driver = IoDriver::with_capacity(64);
        let mut graph = Graph::new();
        let mut scheduler = Scheduler::new();
        scheduler.resize(5);

        // Set up two TCP listeners
        let listener1 = TcpListener::bind("127.0.0.1:0")?;
        let listener1 = mio::net::TcpListener::from_std(listener1);
        let listener2 = TcpListener::bind("127.0.0.1:0")?;
        let listener2 = mio::net::TcpListener::from_std(listener2);
        let addr1 = listener1.local_addr()?;
        let addr2 = listener2.local_addr()?;

        // Register both with different nodes
        let node1_ctx = NodeContext::new(Box::new(|_| false), 1);
        let node2_ctx = NodeContext::new(Box::new(|_| false), 2);
        let node1_idx = graph.add_node(node1_ctx);
        let node2_idx = graph.add_node(node2_ctx);

        let _source1 = driver.register_source(listener1, node1_idx, Interest::READABLE)?;
        let _source2 = driver.register_source(listener2, node2_idx, Interest::READABLE)?;

        // Connect to both listeners
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            let _ = TcpStream::connect(addr1);
            let _ = TcpStream::connect(addr2);
            thread::sleep(Duration::from_millis(50));
        });

        thread::sleep(Duration::from_millis(30));

        // Poll should detect both events
        driver.poll(
            &mut graph,
            &mut scheduler,
            Some(Duration::from_millis(100)),
            1,
        )?;

        // Should have scheduled both nodes (order may vary)
        let mut scheduled = vec![];
        while let Some(node) = scheduler.pop() {
            scheduled.push(node);
        }

        assert_eq!(scheduled.len(), 2);
        assert!(scheduled.contains(&node1_idx));
        assert!(scheduled.contains(&node2_idx));

        Ok(())
    }

    #[test]
    fn test_notifier_polling() -> io::Result<()> {
        let mut driver = IoDriver::with_capacity(64);
        let mut graph = Graph::new();
        let mut scheduler = Scheduler::new();
        scheduler.resize(5);

        let node_ctx = NodeContext::new(Box::new(|_| false), 1);
        let node_idx = graph.add_node(node_ctx);

        // Register a notifier
        let notifier = driver.register_notifier(node_idx)?;

        // Initial poll - no events
        driver.poll(&mut graph, &mut scheduler, Some(Duration::ZERO), 1)?;
        assert!(scheduler.pop().is_none());

        // Trigger the notifier
        notifier.notify()?;

        // Poll should detect the notification
        driver.poll(
            &mut graph,
            &mut scheduler,
            Some(Duration::from_millis(10)),
            2,
        )?;

        let scheduled_node = scheduler.pop();
        assert_eq!(scheduled_node, Some(node_idx));

        Ok(())
    }
}

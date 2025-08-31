use petgraph::prelude::NodeIndex;
use slab::Slab;
use std::io;

use crate::graph_manager::GraphManager;
pub use mio::Interest;

pub struct IoSource<S: mio::event::Source> {
    source: S,
    token: mio::Token,
}

impl<S: mio::event::Source> IoSource<S> {
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
    pub fn with_capacity(capacity: usize) -> Self {
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
        entry.insert(idx);
        let waker = mio::Waker::new(self.poller.registry(), token)?;
        Ok(Notifier::new(waker, token))
    }

    #[inline(always)]
    pub fn deregister_notifier(&mut self, notifier: Notifier) {
        self.indices.remove(notifier.token.0);
    }

    #[inline(always)]
    pub fn register_source<S: mio::event::Source>(
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
    pub fn deregister_source<S: mio::event::Source>(
        &mut self,
        mut handle: IoSource<S>,
    ) -> io::Result<NodeIndex> {
        self.poller.registry().deregister(&mut handle.source)?;
        Ok(self.indices.remove(handle.token.0))
    }

    #[inline(always)]
    pub fn reregister_source<S: mio::event::Source>(
        &mut self,
        handle: &mut IoSource<S>,
        interest: Interest,
    ) -> io::Result<()> {
        self.poller
            .registry()
            .reregister(&mut handle.source, handle.token, interest)
    }

    #[inline(always)]
    pub(super) fn poll(
        &mut self,
        graph_manager: &mut GraphManager,
        timeout: Option<std::time::Duration>,
        epoch: usize,
    ) -> io::Result<()> {
        self.events.clear();
        self.poller.poll(&mut self.events, timeout)?;
        self.events.iter().for_each(|event| {
            let node_index = self.indices[event.token().0];
            graph_manager.schedule_node(node_index, epoch);
        });
        Ok(())
    }
}

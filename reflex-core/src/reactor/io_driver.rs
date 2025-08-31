use petgraph::prelude::NodeIndex;
use slab::Slab;
use std::io;

pub struct IoHandle(mio::Token);

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

#[derive(Debug, Clone, Copy)]
pub(super) struct IoRegistration {
    node_index: NodeIndex,
    interest: mio::Interest,
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
    pub fn deregister_notifier(&mut self, mut notifier: Notifier) {
        self.indices.remove(notifier.token.0);
    }
}

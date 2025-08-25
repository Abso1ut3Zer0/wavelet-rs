use std::io;

const INITIAL_EVENT_CAPACITY: usize = 1024;

pub struct Reactor {
    poller: mio::Poll,
    events: mio::Events,
}

impl Reactor {
    pub fn new() -> Reactor {
        Reactor {
            poller: mio::Poll::new().unwrap(),
            events: mio::Events::with_capacity(INITIAL_EVENT_CAPACITY),
        }
    }

    pub fn poll(&mut self) -> Result<(), io::Error> {
        self.poller.poll(&mut self.events, None)
    }
}

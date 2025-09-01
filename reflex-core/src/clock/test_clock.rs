use crate::clock::Clock;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};
use time::OffsetDateTime;

pub struct TestClock(Rc<RefCell<TestClockInner>>);

impl TestClock {
    pub fn new() -> Self {
        TestClock(Rc::new(RefCell::new(TestClockInner::new())))
    }

    pub fn advance(&self, duration: Duration) {
        self.0.borrow_mut().advance(duration);
    }
}

impl Clock for TestClock {
    fn now(&self) -> Instant {
        self.0.borrow().now()
    }

    fn trigger_time(&self) -> OffsetDateTime {
        self.0.borrow().trigger_time()
    }
}

struct TestClockInner {
    now: Instant,
    trigger_time: OffsetDateTime,
}

impl TestClockInner {
    fn new() -> Self {
        let now = Instant::now();
        let trigger_time = OffsetDateTime::now_utc();
        Self { now, trigger_time }
    }

    fn advance(&mut self, duration: Duration) {
        self.now += duration;
        self.trigger_time += duration;
    }

    fn now(&self) -> Instant {
        self.now
    }

    fn trigger_time(&self) -> OffsetDateTime {
        self.trigger_time
    }
}

use crate::runtime::Notifier;
use crossbeam_queue::ArrayQueue;
use crossbeam_utils::{Backoff, CachePadded};
use enum_as_inner::EnumAsInner;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, thiserror::Error)]
#[error("channel closed")]
pub struct ChannelClosed<T>(T);

#[derive(Debug, thiserror::Error, EnumAsInner)]
pub enum TrySendError<T> {
    #[error("channel closed")]
    ChannelClosed(T),
    #[error("channel full")]
    ChannelFull(T),
}

#[derive(Debug, thiserror::Error, EnumAsInner)]
pub enum TryReceiveError {
    #[error("channel empty")]
    Empty,
    #[error("channel closed")]
    ChannelClosed,
}

pub struct Sender<T> {
    chan: Arc<Channel<T>>,
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        self.chan.senders.fetch_sub(1, Ordering::Release);
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        self.chan.senders.fetch_add(1, Ordering::AcqRel);
        Self {
            chan: self.chan.clone(),
        }
    }
}

impl<T> Sender<T> {
    #[inline(always)]
    pub fn try_send(&self, item: T) -> Result<(), TrySendError<T>> {
        self.chan.try_send(item)
    }

    #[inline(always)]
    pub fn blocking_send(&self, item: T) -> Result<(), ChannelClosed<T>> {
        self.chan.blocking_send(item)
    }

    #[inline(always)]
    pub fn force_send(&self, item: T) -> Result<(), ChannelClosed<T>> {
        self.chan.force_send(item)
    }
}

pub struct Receiver<T> {
    chan: Arc<Channel<T>>,
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.chan.receivers.fetch_sub(1, Ordering::Release);
    }
}

impl<T> Receiver<T> {
    #[inline(always)]
    pub fn try_receive(&self) -> Result<T, TryReceiveError> {
        self.chan.try_receive()
    }
}

struct Channel<T> {
    queue: ArrayQueue<T>,
    notifier: Notifier,
    senders: CachePadded<AtomicUsize>,
    receivers: CachePadded<AtomicUsize>,
}

impl<T> Channel<T> {
    fn new(capacity: usize, notifier: Notifier) -> Self {
        Self {
            queue: ArrayQueue::new(capacity),
            notifier,
            senders: CachePadded::new(AtomicUsize::new(1)),
            receivers: CachePadded::new(AtomicUsize::new(1)),
        }
    }

    #[inline(always)]
    fn try_send(&self, item: T) -> Result<(), TrySendError<T>> {
        if self.receivers.load(Ordering::Acquire) == 0 {
            return Err(TrySendError::ChannelClosed(item));
        }

        self.queue
            .push(item)
            .map_err(|item| TrySendError::ChannelFull(item))
            .map(|_| {
                let backoff = Backoff::new();
                while let Err(_) = self.notifier.notify() {
                    backoff.snooze(); // transient error, retry
                }
            })
    }

    #[inline(always)]
    fn blocking_send(&self, mut item: T) -> Result<(), ChannelClosed<T>> {
        let backoff = Backoff::new();
        loop {
            match self.try_send(item) {
                Ok(_) => {
                    return Ok(());
                }
                Err(TrySendError::ChannelFull(val)) => {
                    item = val;
                    backoff.snooze();
                }
                Err(TrySendError::ChannelClosed(val)) => {
                    return Err(ChannelClosed(val));
                }
            }
        }
    }

    #[inline(always)]
    fn force_send(&self, item: T) -> Result<(), ChannelClosed<T>> {
        if self.receivers.load(Ordering::Acquire) == 0 {
            return Err(ChannelClosed(item));
        }
        self.queue.force_push(item).map(|_| {
            let backoff = Backoff::new();
            while let Err(_) = self.notifier.notify() {
                backoff.snooze(); // transient error, retry
            }
        });
        Ok(())
    }

    fn try_receive(&self) -> Result<T, TryReceiveError> {
        match self.queue.pop() {
            Some(item) => Ok(item),
            None => {
                if self.senders.load(Ordering::Acquire) == 0 {
                    Err(TryReceiveError::ChannelClosed)
                } else {
                    Err(TryReceiveError::Empty)
                }
            }
        }
    }
}

pub(crate) fn new_channel<T>(capacity: usize, notifier: Notifier) -> (Sender<T>, Receiver<T>) {
    let chan = Arc::new(Channel::new(capacity, notifier));
    let sender = Sender { chan: chan.clone() };
    let receiver = Receiver { chan };
    (sender, receiver)
}

#[cfg(test)]
mod tests {
    use crate::Control;
    use crate::prelude::{Clock, Executor, TestClock};
    use crate::runtime::NodeBuilder;
    use std::time::Duration;

    #[test]
    fn test_channel() {
        let mut executor = Executor::new();
        let mut clock = TestClock::new();

        let (node, tx) = NodeBuilder::new("25".to_string())
            .build_with_channel(&mut executor, 8, |this, _, rx| {
                let item = rx.try_receive().unwrap();
                *this = format!("{}", item);
                Control::Broadcast
            })
            .unwrap();

        tx.try_send(1).unwrap();
        executor
            .cycle(clock.trigger_time(), Some(Duration::ZERO))
            .unwrap();
        assert!(executor.has_mutated(&node));
        assert_eq!(*node.borrow(), "1");
    }

    #[test]
    fn test_channel_force_send() {
        let mut executor = Executor::new();
        let mut clock = TestClock::new();

        let (node, tx) = NodeBuilder::new("25".to_string())
            .build_with_channel(&mut executor, 1, |this, _, rx| {
                let item = rx.try_receive().unwrap();
                *this = format!("{}", item);
                Control::Broadcast
            })
            .unwrap();

        tx.force_send(1).unwrap();
        tx.force_send(3).unwrap();
        executor
            .cycle(clock.trigger_time(), Some(Duration::ZERO))
            .unwrap();
        assert!(executor.has_mutated(&node));
        assert_eq!(*node.borrow(), "3");
    }

    #[test]
    fn test_channel_blocking_send() {
        let mut executor = Executor::new();
        let mut clock = TestClock::new();

        let (node, tx) = NodeBuilder::new("25".to_string())
            .build_with_channel(&mut executor, 1, |this, _, rx| {
                let item = rx.try_receive().unwrap();
                *this = format!("{}", item);
                Control::Broadcast
            })
            .unwrap();

        tx.blocking_send(1).unwrap();
        executor
            .cycle(clock.trigger_time(), Some(Duration::ZERO))
            .unwrap();
        assert!(executor.has_mutated(&node));
        assert_eq!(*node.borrow(), "1");
    }
}

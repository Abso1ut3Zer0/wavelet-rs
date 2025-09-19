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
        let prev = self.chan.senders.fetch_sub(1, Ordering::Release);
        if prev == 1 {
            self.chan.notifier.notify();
        }
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
                self.notifier.notify();
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
            self.notifier.notify();
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
    use crate::channel::{Receiver, TryReceiveError};
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

    #[test]
    fn test_multiple_senders_single_receiver() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::thread;

        let mut executor = Executor::new();
        let mut clock = TestClock::new();

        let received_count = Arc::new(AtomicUsize::new(0));
        let count_clone = received_count.clone();

        let (node, tx) = NodeBuilder::new(Vec::<i32>::new())
            .build_with_channel(&mut executor, 100, move |state, _, rx| {
                while let Ok(item) = rx.try_receive() {
                    state.push(item);
                    count_clone.fetch_add(1, Ordering::SeqCst);
                }
                Control::Unchanged
            })
            .unwrap();

        // Spawn multiple sender threads
        let handles: Vec<_> = (0..5)
            .map(|thread_id| {
                let tx = tx.clone();
                thread::spawn(move || {
                    for i in 0..10 {
                        let value = thread_id * 100 + i;
                        tx.try_send(value).unwrap();
                        thread::sleep(Duration::from_micros(10));
                    }
                })
            })
            .collect();

        // Wait for all senders to complete
        for handle in handles {
            handle.join().unwrap();
        }

        // Process all messages
        for _ in 0..10 {
            executor
                .cycle(clock.trigger_time(), Some(Duration::from_millis(1)))
                .unwrap();
            thread::sleep(Duration::from_millis(5));
        }

        // Should have received all 50 messages
        assert_eq!(received_count.load(Ordering::SeqCst), 50);
        assert_eq!(node.borrow().len(), 50);
    }

    #[test]
    fn test_sender_drop_detection() {
        let mut executor = Executor::new();
        let mut clock = TestClock::new();

        let (node, tx) = NodeBuilder::new(0usize)
            .build_with_channel(&mut executor, 10, |state, _, rx| {
                match rx.try_receive() {
                    Ok(value) => {
                        *state = value;
                        Control::Broadcast
                    }
                    Err(TryReceiveError::ChannelClosed) => {
                        *state = 999; // Sentinel value for closed channel
                        Control::Broadcast
                    }
                    Err(TryReceiveError::Empty) => Control::Unchanged,
                }
            })
            .unwrap();

        // Send a value
        tx.try_send(42).unwrap();
        executor
            .cycle(clock.trigger_time(), Some(Duration::ZERO))
            .unwrap();
        assert_eq!(*node.borrow(), 42);

        drop(tx);

        // Second cycle should be the sentinel value on drop
        executor
            .cycle(clock.trigger_time(), Some(Duration::ZERO))
            .unwrap();
        assert_eq!(*node.borrow(), 999);
    }

    #[test]
    fn test_concurrent_send_receive() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::thread;

        let mut executor = Executor::new();
        let mut clock = TestClock::new();

        let running = Arc::new(AtomicBool::new(true));
        let flag = running.clone();

        let (node, tx) = NodeBuilder::new(0usize)
            .build_with_channel(&mut executor, 8, move |state, _, rx| {
                for _ in 0..8 {
                    match rx.try_receive() {
                        Ok(value) => {
                            *state += value;
                        }
                        Err(_) => {
                            break;
                        }
                    }
                }
                Control::Unchanged
            })
            .unwrap();

        // Sender thread
        let sender = thread::spawn(move || {
            for i in 1..=100 {
                tx.blocking_send(i).unwrap();
            }

            std::thread::sleep(Duration::from_millis(100));
            flag.store(false, Ordering::Relaxed);
        });

        while running.load(Ordering::Relaxed) {
            executor
                .cycle(clock.trigger_time(), Some(Duration::ZERO))
                .unwrap();
            thread::sleep(Duration::from_millis(10));
        }

        sender.join().unwrap();

        // Sum of 1..=100 is 5050
        assert_eq!(*node.borrow(), 5050);
    }

    #[test]
    fn test_channel_close_with_pending_messages() {
        let mut executor = Executor::new();
        let mut clock = TestClock::new();

        let (node, tx) = NodeBuilder::new(Vec::<i32>::new())
            .build_with_channel(&mut executor, 100, |state, _, rx| {
                while let Ok(value) = rx.try_receive() {
                    state.push(value);
                }
                Control::Unchanged
            })
            .unwrap();

        // Send multiple messages
        for i in 0..10 {
            tx.try_send(i).unwrap();
        }

        // Drop all senders
        drop(tx);

        // Should still be able to receive pending messages
        executor
            .cycle(clock.trigger_time(), Some(Duration::ZERO))
            .unwrap();
        assert_eq!(node.borrow().len(), 10);
        assert_eq!(*node.borrow(), vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    }

    #[test]
    fn test_force_send_overwrites() {
        use std::sync::Arc;
        use std::thread;

        let mut executor = Executor::new();
        let clock = TestClock::new();

        let (node, tx) = NodeBuilder::new(Vec::<i32>::new())
            .build_with_channel(&mut executor, 3, |state, _, rx| {
                while let Ok(value) = rx.try_receive() {
                    state.push(value);
                }
                Control::Unchanged
            })
            .unwrap();

        let tx = Arc::new(tx);

        // Fill the channel
        tx.try_send(1).unwrap();
        tx.try_send(2).unwrap();
        tx.try_send(3).unwrap();

        // This should fail
        assert!(tx.try_send(4).is_err());

        // Force send should overwrite oldest
        tx.force_send(99).unwrap();

        // Spawn a thread to force more values
        let tx_clone = tx.clone();
        thread::spawn(move || {
            tx_clone.force_send(100).unwrap();
            tx_clone.force_send(101).unwrap();
        })
        .join()
        .unwrap();

        // Process messages
        let mut executor = executor;
        let mut clock = clock;
        executor
            .cycle(clock.trigger_time(), Some(Duration::ZERO))
            .unwrap();

        // Should have the most recent values (exact order depends on timing)
        let values = node.borrow();
        assert!(values.len() <= 3);
        assert!(values.contains(&99) || values.contains(&100) || values.contains(&101));
    }

    #[test]
    fn test_sender_clone_count() {
        use std::thread;

        let mut executor = Executor::new();
        let mut clock = TestClock::new();

        let (node, tx) = NodeBuilder::new(false)
            .build_with_channel(&mut executor, 10, |state, _, rx: &Receiver<()>| {
                match rx.try_receive() {
                    Ok(_) => Control::Unchanged,
                    Err(TryReceiveError::ChannelClosed) => {
                        *state = true; // Mark as closed
                        Control::Broadcast
                    }
                    Err(TryReceiveError::Empty) => Control::Unchanged,
                }
            })
            .unwrap();

        // Clone senders in multiple threads
        let handles: Vec<_> = (0..5)
            .map(|_| {
                let tx = tx.clone();
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(10));
                    drop(tx);
                })
            })
            .collect();

        // Drop original sender
        drop(tx);

        // Wait for all clones to be dropped
        for handle in handles {
            handle.join().unwrap();
        }

        // Channel should now be closed
        executor
            .cycle(clock.trigger_time(), Some(Duration::ZERO))
            .unwrap();
        assert_eq!(*node.borrow(), true);
    }
}

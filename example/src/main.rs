use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tokio::select;
use wavelet::Control;
use wavelet::prelude::{NodeBuilder, PrecisionClock, RealtimeRuntime, Spin};

const TOTAL_COUNT: usize = 10_000;

#[allow(dead_code)]
enum TestMode {
    Wavelet,
    Tokio,
}

#[derive(Debug, Clone)]
struct OrderBook {
    bids: Vec<(f64, f64)>,
    asks: Vec<(f64, f64)>,
    timestamp: Instant,
}

impl OrderBook {
    fn new() -> Self {
        Self {
            bids: Vec::new(),
            asks: Vec::new(),
            timestamp: Instant::now(),
        }
    }
}

fn main() {
    const TEST_MODE: TestMode = TestMode::Tokio;

    match TEST_MODE {
        TestMode::Wavelet => {
            run_wavelet();
        }
        TestMode::Tokio => {
            run_tokio_direct();
        }
    }
}

fn run_wavelet() {
    let instant_queue = Arc::new(crossbeam_queue::ArrayQueue::<Instant>::new(1024));

    let mut runtime = RealtimeRuntime::builder()
        .with_mode(Spin)
        .with_clock(PrecisionClock::new())
        .build()
        .unwrap();

    let (order_book, inbound) = NodeBuilder::new(OrderBook::new())
        .build_with_channel(runtime.executor(), 1, |this, _, rx| {
            if let Ok(book) = rx.try_receive() {
                *this = book;
                return Control::Broadcast;
            }
            Control::Unchanged
        })
        .unwrap();

    let parent = order_book.clone();
    let mid_price = NodeBuilder::new(f64::NAN).triggered_by(&parent).build(
        runtime.executor(),
        move |this, _| {
            let book = parent.borrow();
            match (book.bids.first(), book.asks.first()) {
                (Some(bid), Some(ask)) => {
                    *this = (bid.0 + ask.0) / 2.0;
                    return Control::Broadcast;
                }
                _ => Control::Unchanged,
            }
        },
    );

    let parent = order_book.clone();
    let imbalance =
        NodeBuilder::new(0f64)
            .triggered_by(&parent)
            .build(runtime.executor(), move |this, _| {
                let book = parent.borrow();
                match (book.bids.first(), book.asks.first()) {
                    (Some(bid), Some(ask)) => {
                        *this = bid.1 - ask.1;
                        return Control::Broadcast;
                    }
                    (Some(bid), None) => {
                        *this = bid.1;
                        return Control::Broadcast;
                    }
                    (None, Some(ask)) => {
                        *this = ask.1;
                        return Control::Broadcast;
                    }
                    _ => Control::Unchanged,
                }
            });

    let parent = order_book.clone();
    let depth =
        NodeBuilder::new(0usize)
            .triggered_by(&parent)
            .build(runtime.executor(), move |this, _| {
                let book = parent.borrow();
                *this = book.bids.len().min(book.asks.len());
                return Control::Broadcast;
            });

    let flag = Arc::new(AtomicBool::new(true));
    let running = flag.clone();
    let (_done, notifier) = NodeBuilder::new(())
        .build_with_notifier(runtime.executor(), move |_, _| {
            flag.store(false, std::sync::atomic::Ordering::Relaxed);
            Control::Terminate
        })
        .unwrap();

    let parent = order_book.clone();
    let tx = instant_queue.clone();
    NodeBuilder::new(())
        .triggered_by(&parent)
        .triggered_by(&mid_price)
        .triggered_by(&imbalance)
        .triggered_by(&depth)
        .spawn(runtime.executor(), move |_, _| {
            tx.push(parent.borrow().timestamp).unwrap();
            Control::Unchanged
        });

    let rx = instant_queue.clone();
    let _handle = std::thread::spawn(move || {
        let mut histogram = hdrhistogram::Histogram::<u64>::new(3).unwrap();
        let mut count = 0;
        while count <= TOTAL_COUNT {
            if let Some(instant) = rx.pop() {
                let elapsed = instant.elapsed().as_nanos() as u64;
                histogram.record(elapsed).unwrap();
                count += 1;

                if count % 1_000 == 0 {
                    let p50 = histogram.value_at_percentile(50.0);
                    let p75 = histogram.value_at_percentile(75.0);
                    let p95 = histogram.value_at_percentile(95.0);
                    let p99 = histogram.value_at_percentile(99.0);
                    let p999 = histogram.value_at_percentile(99.9);
                    println!(
                        "{}: p50={} p75={} p95={} p99={} p999={}",
                        count, p50, p75, p95, p99, p999
                    );
                }
            }
        }

        notifier.notify();
    });

    let _guard = std::thread::spawn(move || {
        while running.load(Ordering::Relaxed) {
            let mut book = OrderBook::new();
            book.bids.push((100.0, 1000.0));
            book.asks.push((101.0, 850.0));

            inbound.blocking_send(book).ok();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });

    runtime.run_forever();
}

fn run_tokio_direct() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let instant_queue = Arc::new(crossbeam_queue::ArrayQueue::<Instant>::new(1024));
            let instant_tx = instant_queue.clone();
            let (inbound, mut rx) = tokio::sync::mpsc::channel::<OrderBook>(1024);
            let cxl = tokio_util::sync::CancellationToken::new();
            let notifier = cxl.clone();

            let running = Arc::new(AtomicBool::new(true));
            let flag = running.clone();
            let task = tokio::spawn(async move {
                loop {
                    select! {
                        _ = cxl.cancelled() => {
                            flag.store(false, Ordering::Relaxed);
                            return
                        },
                        book = rx.recv() => {
                            if let Some(book) = book {
                                let _mid_price = std::hint::black_box({
                                    match (book.bids.first(), book.asks.first()) {
                                        (Some(bid), Some(ask)) => (bid.0 + ask.0) / 2.0,
                                        _ => f64::NAN,
                                    }
                                });

                                let _imbalance = std::hint::black_box({
                                    match (book.bids.first(), book.asks.first()) {
                                        (Some(bid), Some(ask)) => bid.1 - ask.1,
                                        (Some(bid), None) => bid.1,
                                        (None, Some(ask)) => ask.1,
                                        _ => 0f64,
                                    }
                                });

                                let _depth = std::hint::black_box({
                                    book.bids.len().min(book.asks.len())
                                });

                                instant_tx.push(book.timestamp).ok();
                            }
                        }
                    }
                }
            });

            let _handle = std::thread::spawn(move || {
                let mut histogram = hdrhistogram::Histogram::<u64>::new(3).unwrap();
                let mut count = 0;
                while count <= TOTAL_COUNT {
                    if let Some(instant) = instant_queue.pop() {
                        let elapsed = instant.elapsed().as_nanos() as u64;
                        histogram.record(elapsed).unwrap();
                        count += 1;

                        if count % 1_000 == 0 {
                            let p50 = histogram.value_at_percentile(50.0);
                            let p75 = histogram.value_at_percentile(75.0);
                            let p95 = histogram.value_at_percentile(95.0);
                            let p99 = histogram.value_at_percentile(99.0);
                            let p999 = histogram.value_at_percentile(99.9);
                            println!(
                                "{}: p50={} p75={} p95={} p99={} p999={}",
                                count, p50, p75, p95, p99, p999
                            );
                        }
                    }
                }

                notifier.cancel();
            });

            let _guard = std::thread::spawn(move || {
                while running.load(Ordering::Relaxed) {
                    let mut book = OrderBook::new();
                    book.bids.push((100.0, 1000.0));
                    book.asks.push((101.0, 850.0));

                    inbound.blocking_send(book).ok();
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            });

            let _ = task.await;
        });
}

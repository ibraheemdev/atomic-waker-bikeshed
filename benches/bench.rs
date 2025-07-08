use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::task::Poll;
use std::task::{Wake, Waker};
use std::thread::{self, Thread};

use atomic_waker_bikeshed::imp1;
use criterion::BenchmarkGroup;
use criterion::measurement::WallTime;
use criterion::{Criterion, criterion_group, criterion_main};

static WAKER: &'static Waker = Waker::noop();

fn compare(c: &mut Criterion) {
    let mut group = c.benchmark_group("compare");

    bench_suite::<imp1::AtomicWaker>(&mut group, "imp1");
    bench_suite::<futures_util::task::AtomicWaker>(&mut group, "futures");
    bench_suite::<diatomic_waker::DiatomicWaker>(&mut group, "diatomic-waker");

    group.finish();
}

fn bench_suite<W: AtomicWaker>(group: &mut BenchmarkGroup<'_, WallTime>, name: &'static str) {
    group.bench_function(&format!("{name}/ping-pong"), |b| {
        b.iter(|| ping_pong::<W>());
    });

    group.bench_function(&format!("{name}/register-notify"), |b| {
        let waker = black_box(W::default());
        b.iter(|| register_notify(&waker));
    });

    group.bench_function(&format!("{name}/notify-register"), |b| {
        let waker = black_box(W::default());
        b.iter(|| notify_register(&waker));
    });

    group.bench_function(&format!("{name}/dummy-notify"), |b| {
        let waker = black_box(W::default());
        b.iter(|| dummy_notify(&waker));
    });

    group.bench_function(&format!("{name}/reregister"), |b| {
        let waker = black_box(W::default());
        b.iter(|| reregister(&waker));
    });
}

fn ping_pong<W: AtomicWaker>() {
    thread_local! {
        static THREAD_WAKER: Waker = Waker::from(Arc::new(ThreadWaker(thread::current())));
    }

    struct ThreadWaker(Thread);

    impl Wake for ThreadWaker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }

    let data = AtomicBool::new(false);
    let (waker1, waker2) = black_box((W::default(), W::default()));

    const ITERATIONS: usize = 500;
    std::thread::scope(|s| {
        s.spawn(|| {
            let waker: Waker = THREAD_WAKER.with(Waker::clone);

            for _ in 0..ITERATIONS {
                data.store(true, Ordering::SeqCst);
                waker2.notify();

                loop {
                    if !data.load(Ordering::SeqCst) {
                        break;
                    }

                    if unsafe { waker1.register(&waker) }.is_ready() {
                        continue;
                    }

                    if !data.load(Ordering::SeqCst) {
                        unsafe { waker1.unregister() };
                        break;
                    }

                    thread::park();
                }
            }
        });

        s.spawn(|| {
            let waker: Waker = THREAD_WAKER.with(Waker::clone);

            for _ in 0..ITERATIONS {
                loop {
                    if data.load(Ordering::SeqCst) {
                        break;
                    }

                    if unsafe { waker2.register(&waker) }.is_ready() {
                        continue;
                    }

                    if data.load(Ordering::SeqCst) {
                        unsafe { waker2.unregister() };
                        break;
                    }

                    thread::park();
                }

                data.store(false, Ordering::SeqCst);
                waker1.notify();
            }
        });
    });
}

fn register_notify(waker: &impl AtomicWaker) {
    unsafe {
        let _ = waker.register(black_box(WAKER));
    };
    waker.notify();
}

fn notify_register(waker: &impl AtomicWaker) {
    waker.notify();
    unsafe {
        let _ = waker.register(black_box(WAKER));
    };
}

fn dummy_notify(waker: &impl AtomicWaker) {
    waker.notify();
}

fn reregister(waker: &impl AtomicWaker) {
    unsafe {
        let _ = waker.register(black_box(Waker::noop()));
    }
}

trait AtomicWaker: Default + Send + Sync {
    unsafe fn register(&self, waker: &Waker) -> Poll<()>;
    unsafe fn unregister(&self);
    fn notify(&self);
}

impl AtomicWaker for futures_util::task::AtomicWaker {
    unsafe fn register(&self, waker: &Waker) -> Poll<()> {
        self.register(waker);
        Poll::Pending
    }

    unsafe fn unregister(&self) {}

    fn notify(&self) {
        self.wake();
    }
}

impl AtomicWaker for imp1::AtomicWaker {
    unsafe fn register(&self, waker: &Waker) -> Poll<()> {
        unsafe { self.register_seqcst(waker) }
    }

    unsafe fn unregister(&self) {}

    fn notify(&self) {
        self.notify_seqcst();
    }
}

impl AtomicWaker for diatomic_waker::DiatomicWaker {
    unsafe fn register(&self, waker: &Waker) -> Poll<()> {
        unsafe { self.register(waker) };
        Poll::Pending
    }

    unsafe fn unregister(&self) {
        unsafe { self.unregister() };
    }

    fn notify(&self) {
        self.notify();
    }
}

criterion_group!(benches, compare);
criterion_main!(benches);

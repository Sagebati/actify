//! Counts the heap allocations one actor call costs.
//!
//! This has its own binary on purpose: the counter is process-wide, so a test
//! running beside it on another thread would be counted too. Keep this file to
//! a single test.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use actify::actify;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static COUNTING: AtomicBool = AtomicBool::new(false);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

#[derive(Clone, Debug)]
struct Counter(i32);

#[actify]
impl Counter {
    fn add(&mut self, value: i32) -> i32 {
        self.0 += value;
        self.0
    }

    async fn add_later(&mut self, value: i32) -> i32 {
        self.0 += value;
        self.0
    }
}

/// Runs `call` repeatedly and reports what each one allocated.
async fn per_call<F, Fut>(runs: usize, mut call: F) -> f64
where
    F: FnMut() -> Fut,
    Fut: Future<Output = i32>,
{
    // Warm up, so that anything a channel allocates in batches is already paid
    // for by the time the count starts.
    for _ in 0..runs {
        call().await;
    }

    ALLOCATIONS.store(0, Ordering::Relaxed);
    COUNTING.store(true, Ordering::Relaxed);
    for _ in 0..runs {
        call().await;
    }
    COUNTING.store(false, Ordering::Relaxed);

    ALLOCATIONS.load(Ordering::Relaxed) as f64 / runs as f64
}

/// A call costs the reply channel and the queue slot carrying it, and nothing
/// else. The arguments and the result travel inside the message at their own
/// types, so neither is boxed, and the actor runs the call without boxing a
/// future for it either.
#[test]
fn a_call_allocates_only_the_reply_and_its_queue_slot() {
    // A current-thread runtime keeps the actor on this thread, so every
    // allocation a call makes is one this thread made.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();

    runtime.block_on(async {
        let handle = CounterHandle::new(Counter(0));

        let builtin = per_call(200, || async { handle.get().await.0 }).await;
        let generated = per_call(200, || handle.add(1)).await;
        let generated_async = per_call(200, || handle.add_later(1)).await;

        println!("get:       {builtin} allocations per call");
        println!("add:       {generated} allocations per call");
        println!("add_later: {generated_async} allocations per call");

        assert!(builtin <= 2.0, "get allocated {builtin} per call");
        assert!(generated <= 2.0, "add allocated {generated} per call");

        // An async method costs no more: the enum's `dispatch` is one
        // `async fn`, so the body's future is part of it rather than boxed
        // beside it.
        assert!(
            generated_async <= 2.0,
            "add_later allocated {generated_async} per call"
        );
    });
}

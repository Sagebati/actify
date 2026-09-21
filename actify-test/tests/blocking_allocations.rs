//! Counts the heap allocations one blocking actor call costs.
//!
//! This has its own binary on purpose: the counter is process-wide, so a test
//! running beside it on another thread would be counted too. Keep this file to
//! a single test.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use actify::actify;
use actify::blocking::Wait;

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

#[actify(blocking)]
impl Counter {
    fn add(&mut self, value: i32) -> i32 {
        self.0 += value;
        self.0
    }
}

/// Runs `call` repeatedly and reports what each one allocated.
fn per_call<F: FnMut() -> i32>(runs: usize, mut call: F) -> f64 {
    // Warm up, so that anything a channel allocates in batches is already paid
    // for by the time the count starts.
    for _ in 0..runs {
        call();
    }

    ALLOCATIONS.store(0, Ordering::Relaxed);
    COUNTING.store(true, Ordering::Relaxed);
    for _ in 0..runs {
        call();
    }
    COUNTING.store(false, Ordering::Relaxed);

    ALLOCATIONS.load(Ordering::Relaxed) as f64 / runs as f64
}

/// A blocking call costs the same as an async one: the reply channel and the
/// queue slot carrying it, and nothing else. The arguments and the result
/// travel inside the message at their own types, so neither is boxed, and the
/// reply slot is one `Arc` exactly as a futures oneshot is.
///
/// The two wait modes are measured separately because they take different
/// paths out of the same slot, and neither is allowed to allocate on the way.
///
/// The budget asserted here is the async backend's two, so that neither can
/// regress past the other. The figure printed is lower, around one, because
/// `std::sync::mpsc` allocates its queue in blocks rather than per message.
#[test]
fn a_call_allocates_only_the_reply_and_its_queue_slot() {
    for wait in [Wait::Park, Wait::Spin] {
        // The actor is on its own thread, so its allocations are not counted
        // here. That is the same shape as the async test, which pins the actor
        // to the calling thread instead; either way what is measured is one
        // caller's cost per call.
        let (handle, actor) = CounterHandle::builder(Counter(0)).wait(wait).build();
        let running = std::thread::spawn(actor);

        let builtin = per_call(200, || handle.get().0);
        let generated = per_call(200, || handle.add(1));

        println!("{wait:?} get: {builtin} allocations per call");
        println!("{wait:?} add: {generated} allocations per call");

        assert!(builtin <= 2.0, "{wait:?} get allocated {builtin} per call");
        assert!(
            generated <= 2.0,
            "{wait:?} add allocated {generated} per call"
        );

        drop(handle);
        running.join().unwrap();
    }
}

//! A blocking actor that allocates nothing: not per call, not for its queue.
//!
//! `#[actum(blocking, no_alloc)]` puts each call's reply slot on the caller's
//! stack, and the job queue is a `static` ring buffer. The counting allocator
//! below is there so the claim is measured rather than stated: a thousand
//! calls from two threads, zero allocations.
//!
//! Run it with `cargo run --example no_alloc`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use actum::actum;
use actum::blocking::Wait;
use actum::blocking::no_alloc::Queue;

/// Counts every allocation the process makes, on every thread.
struct Counting;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

struct Counter(u64);

#[actum(blocking, no_alloc)]
impl Counter {
    fn add(&mut self, value: u64) -> u64 {
        self.0 += value;
        self.0
    }
}

// The job queue: eight slots in a static, so no allocation ever. `CounterCall`
// is the message enum `#[actum]` generates for `Counter`. The size is a power
// of two, at most 128.
static JOBS: Queue<CounterCall, 8> = Queue::new();

fn main() {
    // `no_alloc` has no default channel: `build` does not exist until one is
    // given, because the default would be a std queue, and allocate.
    let (handle, actor) = CounterHandle::builder(Counter(0))
        .channel(JOBS.split())
        .wait(Wait::Park)
        .build();
    let served = std::thread::spawn(actor);

    // A second caller on its own thread. Cloning a handle bumps a count in
    // the static and allocates nothing; spawning a thread does, so the helper
    // is started before the window opens and waits at the line for it.
    static START: AtomicBool = AtomicBool::new(false);
    let other = handle.clone();
    let helper = std::thread::spawn(move || {
        // Paid once and not what is measured: this thread's first park.
        other.add(0);
        while !START.load(Ordering::Acquire) {
            std::hint::spin_loop();
        }
        for _ in 0..500 {
            other.add(1);
        }
    });
    handle.add(0);

    let before = ALLOCATIONS.load(Ordering::Relaxed);
    START.store(true, Ordering::Release);
    for _ in 0..500 {
        // The reply slot is a local of this call, on this stack. The message
        // carries a pointer to it, and `add` does not return until the actor
        // has written through it. Nothing here touches the heap.
        handle.add(1);
    }
    helper.join().unwrap();
    let after = ALLOCATIONS.load(Ordering::Relaxed);

    println!("total {}", handle.add(0));
    println!(
        "1000 calls from two threads, {} allocations",
        after - before
    );

    drop(handle);
    served.join().unwrap();
}

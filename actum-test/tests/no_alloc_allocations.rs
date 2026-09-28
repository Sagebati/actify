//! Counts the heap allocations one `no_alloc` blocking actor call costs, which
//! is the number in the name.
//!
//! The reply is a slot on the caller's stack and the job queue is a static, so
//! there is nothing left for a call to allocate. Every call shape the plain
//! blocking file measures is measured here at zero, and so is a run of calls,
//! since a queue that never allocates has nothing to amortise.
//!
//! The counter is process-wide, so the actor's own thread is counted too,
//! which is the point: that is where the method runs and the reply is written.
//! It is also why this file has its own binary and holds a single test.

mod support;

use std::alloc::System;

use actum::actum;
use actum::blocking::Wait;
use actum::blocking::no_alloc::Queue;
use support::{COUNTING, StatsAlloc, assert_allocations, assert_every_call, measure, measure_each};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = COUNTING;

/// What a call costs: nothing.
const PER_CALL: usize = 0;

#[derive(Debug)]
struct Counter(i32);

#[actum(blocking, no_alloc)]
impl Counter {
    fn add(&mut self, value: i32) -> i32 {
        self.0 += value;
        self.0
    }

    /// Takes an owned `String` and hands it straight back, so the method body
    /// allocates nothing and the only thing measured is the plumbing.
    fn echo(&self, name: String) -> String {
        name
    }

    /// Returns something big but `Copy`, so producing it costs nothing and any
    /// allocation would be a box the plumbing put around it.
    fn block(&self) -> [u8; 64] {
        [self.0 as u8; 64]
    }

    fn nothing(&self) {}

    /// Carries a `where` clause of its own, so its call reaches it through a
    /// function pointer in the message rather than by being named directly.
    fn sorted(&self, values: Vec<i32>) -> Vec<i32>
    where
        i32: Ord,
    {
        values
    }
}

// One queue per wait mode, since a static queue is split once.
static PARKED: Queue<CounterCall, 64> = Queue::new();
static SPINNING: Queue<CounterCall, 64> = Queue::new();

#[test]
fn a_call_allocates_nothing() {
    // Both wait modes take different paths through the same slot, and neither
    // may allocate on the way.
    for (wait, jobs) in [(Wait::Park, &PARKED), (Wait::Spin, &SPINNING)] {
        let (handle, actor) = CounterHandle::builder(Counter(0))
            .channel(jobs.split())
            .wait(wait)
            .build();
        let running = std::thread::spawn(actor);

        // Anything paid once - a thread's first park, a thread handle - is
        // paid before any window opens.
        for _ in 0..50 {
            handle.add(1);
        }

        let case = |name: &str| format!("{wait:?} {name}");

        assert_allocations(measure(|| handle.add(1)), PER_CALL, &case("add"));

        let name = "Alfred".to_string();
        assert_allocations(measure(|| handle.echo(name)), PER_CALL, &case("echo"));

        assert_allocations(measure(|| handle.block()), PER_CALL, &case("block"));

        assert_allocations(measure(|| handle.nothing()), PER_CALL, &case("nothing"));

        let values = vec![3, 1, 2];
        assert_allocations(measure(|| handle.sorted(values)), PER_CALL, &case("sorted"));

        // Cloning a handle bumps a count in the static queue.
        assert_allocations(measure(|| handle.clone()), 0, &case("Handle::clone"));

        // A run of calls, so that a cost paid every few calls would show up
        // even if the one measured above happened to be free.
        let counts = measure_each(200, || handle.add(1));
        assert_every_call(&counts, PER_CALL, &case("add over the static queue"));

        drop(handle);
        running.join().unwrap();
    }
}

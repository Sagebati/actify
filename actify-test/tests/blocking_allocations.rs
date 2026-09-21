//! Counts the heap allocations one blocking actor call costs.
//!
//! The reply channel is one allocation, as the async backend's is. The queue
//! slot is the part that differs: a `sync_channel` allocates its buffer once at
//! construction and nothing per message, so over one of those a call costs
//! exactly one allocation - the reply, and nothing else. The default unbounded
//! `std::sync::mpsc` allocates its queue in blocks of about thirty-two
//! messages, so there the per-call cost is one with an occasional second.
//!
//! The counter is process-wide, not per-thread, so the actor's own thread is
//! counted here too - which is the point, since that is where the method runs
//! and where the reply is sent. It is also why this file has its own binary and
//! holds a single test: anything running beside it would be counted as well.

mod support;

use std::alloc::System;

use actify::actify;
use actify::blocking::Wait;
use support::{COUNTING, StatsAlloc, assert_allocations, assert_every_call, measure, measure_each};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = COUNTING;

/// What a call costs over a channel that has already allocated its queue: the
/// reply, and nothing else.
const PER_CALL: usize = 1;

/// How many slots the bounded channel gets. Large enough that a caller never
/// waits on it, so what is measured is a call and not a queue being drained.
const SLOTS: usize = 64;

#[derive(Debug)]
struct Counter(i32);

#[actify(blocking)]
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

#[test]
fn a_call_allocates_only_the_reply_and_its_queue_slot() {
    // Both wait modes take different paths out of the same channel, and
    // neither may allocate on the way.
    for wait in [Wait::Park, Wait::Spin] {
        let (handle, actor) = CounterHandle::builder(Counter(0))
            .channel(std::sync::mpsc::sync_channel(SLOTS))
            .wait(wait)
            .build();
        let running = std::thread::spawn(actor);

        // Anything paid once - the channel's buffer, a thread's first park -
        // is paid before any window opens.
        for _ in 0..50 {
            handle.add(1);
        }

        let case = |name: &str| format!("{wait:?} {name}");

        // A plain generated method.
        assert_allocations(measure(|| handle.add(1)), PER_CALL, &case("add"));

        // An owned argument is moved into the message variant. Built before
        // the window, so a clone on the way would show up as a second.
        let name = "Alfred".to_string();
        assert_allocations(measure(|| handle.echo(name)), PER_CALL, &case("echo"));

        // The result travels inside the message at its own type.
        assert_allocations(measure(|| handle.block()), PER_CALL, &case("block"));

        // The reply channel is made whether or not there is anything to put in
        // it, so a method returning nothing costs the same, not nothing.
        assert_allocations(measure(|| handle.nothing()), PER_CALL, &case("nothing"));

        // A `where` clause is carried by a function pointer in the variant,
        // which is a word, not a boxed closure.
        let values = vec![3, 1, 2];
        assert_allocations(measure(|| handle.sorted(values)), PER_CALL, &case("sorted"));

        // Handles share one channel through an `Arc`, so making another costs
        // a reference count and nothing on the heap.
        assert_allocations(measure(|| handle.clone()), 0, &case("Handle::clone"));

        // Not one call in isolation but every one of a run of them: over a
        // channel that has already allocated its queue there is nothing left
        // to amortise, so a single call costing more than the reply would show
        // up here even if the one measured above happened to be cheap.
        let counts = measure_each(200, || handle.add(1));
        assert_every_call(&counts, PER_CALL, &case("add over a preallocated channel"));

        drop(handle);
        running.join().unwrap();
    }

    // The default channel allocates its queue in blocks rather than per
    // message, so the per-call cost is the reply alone and the block shows up
    // on the occasional call that fills one.
    let (handle, actor) = CounterHandle::builder(Counter(0)).build();
    let running = std::thread::spawn(actor);

    for _ in 0..50 {
        handle.add(1);
    }
    let counts = measure_each(200, || handle.add(1));

    let min = *counts.iter().min().unwrap();
    let max = *counts.iter().max().unwrap();
    println!("default channel: min {min}, max {max} allocations per call");

    assert_eq!(min, PER_CALL, "the cheapest call is the reply alone");
    assert!(
        max <= PER_CALL + 1,
        "no call costs more than the reply and one queue block, saw {max}"
    );

    drop(handle);
    running.join().unwrap();
}

//! Counts the heap allocations one actor call costs.
//!
//! A call costs the reply channel and the queue slot carrying it, and nothing
//! else. Both are one allocation: a `futures_channel::oneshot` for the reply,
//! and one `Box<Node>` per message for the `futures_channel::mpsc` queue. So
//! every case here is exactly two, and every case is measured on its own rather
//! than averaged, because an average cannot tell two every call from three on
//! half of them.
//!
//! This has its own binary on purpose: the counter is process-wide, so a test
//! running beside it on another thread would be counted too. Keep this file to
//! a single test.

mod support;

use std::alloc::System;

use actify::actify;
use support::{COUNTING, StatsAlloc, assert_allocations, measure, measure_async};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = COUNTING;

/// What a call costs: the reply channel, and the queue slot carrying it.
const PER_CALL: usize = 2;

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
    // A current-thread runtime keeps the actor on this thread, so every
    // allocation a call makes is one this thread made.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();

    runtime.block_on(async {
        let mut handle = CounterHandle::new(Counter(0));

        // Anything paid once - the channel's first node, a waker's first
        // registration - is paid before any window opens.
        for _ in 0..50 {
            handle.add(1).await;
            handle.get().await;
        }

        // The built-in calls.
        assert_allocations(measure_async(handle.get()).await, PER_CALL, "get");
        assert_allocations(measure_async(handle.set(Counter(0))).await, PER_CALL, "set");

        // A plain generated method.
        assert_allocations(measure_async(handle.add(1)).await, PER_CALL, "add");

        // An async method costs no more: the generated loop is one `async fn`,
        // so the method's future is part of its state machine rather than
        // boxed beside it.
        assert_allocations(
            measure_async(handle.add_later(1)).await,
            PER_CALL,
            "add_later",
        );

        // An owned argument is moved into the message variant. Built before
        // the window, so a clone on the way would show up as a third.
        let name = "Alfred".to_string();
        assert_allocations(measure_async(handle.echo(name)).await, PER_CALL, "echo");

        // The result travels inside the message at its own type. A box around
        // it would show up as a third.
        assert_allocations(measure_async(handle.block()).await, PER_CALL, "block");

        // The reply channel is made whether or not there is anything to put in
        // it, so a method returning nothing costs the same two, not one.
        assert_allocations(measure_async(handle.nothing()).await, PER_CALL, "nothing");

        // A `where` clause is carried by a function pointer in the variant,
        // which is a word, not a boxed closure.
        let values = vec![3, 1, 2];
        assert_allocations(
            measure_async(handle.sorted(values)).await,
            PER_CALL,
            "sorted",
        );

        // A read handle is the same path.
        let mut reader = handle.read_handle();
        assert_allocations(
            measure_async(reader.get()).await,
            PER_CALL,
            "ReadHandle::get",
        );

        // A bounded channel costs the same two. It used to cost three: every
        // send cloned the sender, and cloning a bounded `futures_channel`
        // sender allocates an `Arc<Mutex<SenderTask>>` for the new sender's
        // waker slot. A handle sends through its own sender now, so there is
        // no clone and no third allocation.
        let (mut bounded, actor) = CounterHandle::builder(Counter(0))
            .channel(futures_channel::mpsc::channel(64))
            .build();
        tokio::spawn(actor);
        for _ in 0..50 {
            bounded.add(1).await;
        }
        assert_allocations(
            measure_async(bounded.add(1)).await,
            PER_CALL,
            "add over a bounded channel",
        );

        // Handles share one channel through an `Arc`, so making another costs
        // a reference count and nothing on the heap.
        assert_allocations(measure(|| handle.clone()), 0, "Handle::clone");
        assert_allocations(measure(|| handle.read_handle()), 0, "read_handle");
    });
}

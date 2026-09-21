//! Counting what a single call allocates.
//!
//! Each test binary that uses this installs the allocator itself:
//!
//! ```ignore
//! mod support;
//!
//! #[global_allocator]
//! static GLOBAL: &support::StatsAlloc<std::alloc::System> = support::COUNTING;
//! ```
//!
//! The counters behind it are **process-wide**, which is deliberate: a blocking
//! actor runs on its own thread, and a thread-local counter would be blind to
//! everything it allocates, which is half of what these tests measure. The
//! price is that a test running beside one of these on another thread would be
//! counted too, which is why each of these binaries holds a single test.
//!
//! Measure one call, not an average of many. An average cannot tell "two every
//! call" from "three on half the calls and one on the rest", and it cannot
//! notice a count going back up if the bound it is checked against is loose.

#![allow(dead_code)]

use std::alloc::System;

pub use stats_alloc::StatsAlloc;
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats};

/// The allocator a test binary installs as its global one.
pub const COUNTING: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

/// What one call allocated, and nothing else.
///
/// The value is dropped only after the reading, so what is counted is what the
/// call allocated rather than what dropping the result gave back.
pub fn measure<R>(call: impl FnOnce() -> R) -> Stats {
    let region = Region::new(COUNTING);
    let value = call();
    let change = region.change();
    drop(value);
    change
}

/// What one call allocated, where the call is a future.
///
/// The future is passed already built, so that whatever the caller did to make
/// its arguments is outside the window and only the call itself is counted.
pub async fn measure_async<F: Future>(call: F) -> Stats {
    let region = Region::new(COUNTING);
    let value = call.await;
    let change = region.change();
    drop(value);
    change
}

/// Each of `runs` calls counted separately, so the distribution is visible
/// rather than averaged away.
///
/// For a channel that allocates its queue in blocks rather than per message,
/// this is what separates the per-call cost from the amortised one.
pub fn measure_each<R>(runs: usize, mut call: impl FnMut() -> R) -> Vec<usize> {
    (0..runs).map(|_| measure(&mut call).allocations).collect()
}

/// Runs a call `runs` times without counting, so that anything paid once - a
/// channel's first block, a thread's first park, a tracing callsite - is
/// already paid when the window opens.
pub fn warm_up<R>(runs: usize, mut call: impl FnMut() -> R) {
    for _ in 0..runs {
        call();
    }
}

/// Asserts a single call's exact cost, and that nothing was reallocated on the
/// way.
///
/// `stats_alloc` counts a reallocation separately from an allocation. None of
/// these paths should ever reallocate, so a non-zero count means something grew
/// a buffer that a call has no business growing.
#[track_caller]
pub fn assert_allocations(change: Stats, expected: usize, what: &str) {
    assert_eq!(
        change.allocations, expected,
        "{what} allocated {} times, expected {expected} ({change:?})",
        change.allocations
    );
    assert_eq!(
        change.reallocations, 0,
        "{what} reallocated {} times ({change:?})",
        change.reallocations
    );
}

/// Asserts that every one of a run of calls cost exactly `expected`.
#[track_caller]
pub fn assert_every_call(counts: &[usize], expected: usize, what: &str) {
    let wrong: Vec<_> = counts
        .iter()
        .enumerate()
        .filter(|&(_, &count)| count != expected)
        .take(5)
        .collect();
    assert!(
        wrong.is_empty(),
        "{what}: expected {expected} allocations on every call, but saw {wrong:?} \
         (call index, count) out of {} calls",
        counts.len()
    );
}

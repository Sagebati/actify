//! A `no_alloc` actor behaves as any blocking actor does, from the outside.
//!
//! Calls arrive, the actor stops once the last handle is dropped, a caller
//! learns when the actor is gone, and a full queue makes a caller wait. What
//! differs is under the surface, and `no_alloc_allocations.rs` measures that.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use actum::actum;
use actum::blocking::Wait;
use actum::blocking::no_alloc::Queue;

#[derive(Debug)]
struct Greeter {
    greeting: String,
}

#[actum(blocking, no_alloc)]
impl Greeter {
    fn say_hi(&self, name: String) -> String {
        format!("{} {name}", self.greeting)
    }

    fn shout(&mut self) {
        self.greeting = self.greeting.to_uppercase();
    }

    fn boom(&self) {
        panic!("the actor's own panic message")
    }

    fn slowly(&self, millis: u64) {
        std::thread::sleep(Duration::from_millis(millis));
    }
}

fn greeter() -> Greeter {
    Greeter {
        greeting: "hi".to_string(),
    }
}

/// Waits for the actor's thread to finish, which it does only once the last
/// handle has been dropped. Fails rather than hanging if it never does.
fn stops(running: JoinHandle<()>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !running.is_finished() {
        assert!(
            Instant::now() < deadline,
            "the actor stops once its last handle is dropped"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    running.join().unwrap();
}

#[test]
fn test_calls_arrive_and_the_actor_stops_with_its_last_handle() {
    static JOBS: Queue<GreeterCall, 4> = Queue::new();
    let (handle, actor) = GreeterHandle::builder(greeter())
        .channel(JOBS.split())
        .build();
    let running = std::thread::spawn(actor);

    assert_eq!(handle.say_hi("Alfred".to_string()), "hi Alfred");
    handle.shout();
    assert_eq!(handle.say_hi("Alfred".to_string()), "HI Alfred");

    // A clone on another thread shares the actor, and its drop is not the
    // last one.
    let other = handle.clone();
    let elsewhere = std::thread::spawn(move || other.say_hi("Bertha".to_string()));
    assert_eq!(elsewhere.join().unwrap(), "HI Bertha");

    drop(handle);
    stops(running);
}

#[test]
fn test_a_spinning_actor_is_served() {
    static JOBS: Queue<GreeterCall, 4> = Queue::new();
    let (handle, actor) = GreeterHandle::builder(greeter())
        .channel(JOBS.split())
        .wait(Wait::Spin)
        .build();
    let running = std::thread::spawn(actor);

    assert_eq!(handle.say_hi("Alfred".to_string()), "hi Alfred");

    drop(handle);
    stops(running);
}

/// A panicking method takes the actor's thread with it. The caller waiting on
/// that call is told the actor is gone rather than waiting forever: the
/// reply half is dropped by the unwinding, and reports `Closed` on its way.
#[test]
fn test_a_panicking_method_reaches_its_caller() {
    static JOBS: Queue<GreeterCall, 4> = Queue::new();
    let (handle, actor) = GreeterHandle::builder(greeter())
        .channel(JOBS.split())
        .build();
    let running = std::thread::spawn(actor);

    let called = catch_unwind(AssertUnwindSafe(|| handle.boom()));
    let payload = called.expect_err("the call panics");
    let message = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .expect("a string payload");
    assert!(message.contains("no longer running"), "{message}");

    assert!(running.join().is_err(), "the thread unwound");

    // And every later call, on any clone, learns the same.
    let later = catch_unwind(AssertUnwindSafe(|| handle.say_hi("x".to_string())));
    assert!(later.is_err(), "the actor is gone for good");
}

/// Calls already queued when the actor stops are answered `Closed`, because
/// dropping the receiving half drops them, and dropping a reply half reports.
#[test]
fn test_calls_queued_behind_a_stopped_actor_are_refused() {
    static JOBS: Queue<GreeterCall, 4> = Queue::new();
    let (handle, actor) = GreeterHandle::builder(greeter())
        .channel(JOBS.split())
        .build();
    // Never started: the closure, and with it the receiving half, is dropped.
    drop(actor);

    let called = catch_unwind(AssertUnwindSafe(|| handle.say_hi("x".to_string())));
    assert!(called.is_err(), "the actor was never started");
}

/// A queue of two with a slow actor: more callers than slots, and every call
/// still lands, because a full queue makes a sender wait for room rather than
/// fail.
#[test]
fn test_a_full_queue_applies_backpressure() {
    static JOBS: Queue<GreeterCall, 2> = Queue::new();
    let (handle, actor) = GreeterHandle::builder(greeter())
        .channel(JOBS.split())
        .build();
    let running = std::thread::spawn(actor);

    let callers: Vec<_> = (0..6)
        .map(|_| {
            let handle = handle.clone();
            std::thread::spawn(move || {
                handle.slowly(10);
                handle.say_hi("x".to_string())
            })
        })
        .collect();
    for caller in callers {
        assert_eq!(caller.join().unwrap(), "hi x");
    }

    drop(handle);
    stops(running);
}

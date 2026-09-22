//! Actors that run on a thread, for callers that have no executor.
//!
//! `#[actum(blocking)]` generates the same message enum, handle and builder as
//! `#[actum]` does, with `async` taken out of all three: the actor's loop is a
//! plain `fn` that a [`std::thread`] runs, and a handle's methods are ordinary
//! synchronous calls that return when the actor has answered.
//!
//! ```
//! use actum::actum;
//!
//! #[derive(Debug)]
//! struct Counter(i32);
//!
//! #[actum(blocking)]
//! impl Counter {
//!     fn add(&mut self, value: i32) -> i32 {
//!         self.0 += value;
//!         self.0
//!     }
//! }
//!
//! let handle = CounterHandle::new(Counter(0));
//! assert_eq!(handle.add(2), 2);
//! assert_eq!(handle.add(3), 5);
//! ```
//!
//! # Waiting
//!
//! Two threads wait on every call: the actor waits for the next job, and the
//! caller waits for the reply. [`Wait`] decides how both of them do it, and is
//! chosen once when the actor is built.
//!
//! [`Wait::Park`], the default, blocks: the actor sits in the channel's `recv`
//! and the caller sits on the reply, and neither costs anything while it waits.
//!
//! [`Wait::Spin`] busy-waits: both loop on [`std::hint::spin_loop`] and never
//! sleep. A reply arrives as fast as the hardware can carry it, at the price of
//! a core held for the whole wait - including while the actor is idle. It is
//! for a pinned thread with nothing else to do, and is the wrong default for
//! everything else.
//!
//! ```
//! # use actum::actum;
//! use actum::blocking::Wait;
//!
//! # #[derive(Debug)]
//! # struct Counter(i32);
//! # #[actum(blocking)]
//! # impl Counter {
//! #     fn add(&mut self, value: i32) -> i32 { self.0 += value; self.0 }
//! # }
//! let (handle, actor) = CounterHandle::builder(Counter(0)).wait(Wait::Spin).build();
//! let running = std::thread::spawn(actor);
//!
//! assert_eq!(handle.add(1), 1);
//!
//! drop(handle);
//! running.join().unwrap();
//! ```
//!
//! # What a call costs
//!
//! One allocation for the reply channel, plus whatever the job channel charges
//! to carry a message. A [`sync_channel`](std::sync::mpsc::sync_channel)
//! allocates its buffer once and nothing per message, so over one of those a
//! call is exactly one allocation; the unbounded default allocates its queue in
//! blocks, so it adds an amortised fraction of one. Either way it is at or
//! under the async backend's two.
//!
//! Arguments and results travel inside the message enum at their own types, so
//! nothing is boxed or downcast.
//!
//! # Deadlocks
//!
//! A blocking call occupies its thread until the actor answers, so an actor
//! method that calls back into its own handle blocks forever, and no runtime is
//! there to notice. The async backend has the same rule; here it costs a thread
//! rather than a task.

use std::any::type_name;
use std::fmt::{self, Debug};
use std::marker::PhantomData;

mod builder;
mod channel;
mod reply;

pub use builder::{DefaultChannel, HandleBuilder};
pub use channel::{Closed, JobReceiver, JobSender, Next};

use reply::Answer;

/// How a blocking actor and its callers wait.
///
/// Chosen once with [`HandleBuilder::wait`], and used by both ends: the actor
/// waits for jobs this way, and every call on the handle waits for its reply
/// this way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Wait {
    /// Block the thread. Costs nothing while waiting. The default.
    #[default]
    Park,
    /// Busy-wait, never sleeping. Lowest latency, and holds a core throughout.
    Spin,
}

/// Takes the next job the way this actor was built to wait for it.
///
/// The spin loop lives here rather than in generated code so that it is
/// written, and can be tuned, in one place.
#[doc(hidden)]
pub fn next<M, R: JobReceiver<M>>(rx: &mut R, wait: Wait) -> Option<M> {
    match wait {
        Wait::Park => rx.recv(),
        Wait::Spin => loop {
            match rx.try_recv() {
                Next::Job(job) => return Some(job),
                Next::Closed => return None,
                Next::Empty => std::hint::spin_loop(),
            }
        },
    }
}

/// Panics because the actor is gone, whether it stopped or a method of it
/// panicked. Which of the two it was is on the actor's own exit event, at
/// ERROR level for a panic, because only the actor thread can tell them apart.
fn report_actor_gone<T>() -> ! {
    panic!("Actor of type {} is no longer running", type_name::<T>());
}

/// The plumbing behind a generated blocking handle: the sending half of the
/// actor's job channel, and how to wait for a reply.
///
/// `#[actum(blocking)]` generates a handle of the actor's own that wraps this
/// one, so a caller never names it. Cloning it shares access to the same actor
/// across threads.
pub struct Handle<T, M, S = DefaultSender<M>> {
    // The last handle to drop drops the last sending half, which is what stops
    // the actor.
    sender: S,
    // A word beside the sender, rather than a type parameter on every
    // generated handle. The branch it costs is on a path that is about to
    // block or spin for far longer.
    wait: Wait,
    actor: Marker<T, M>,
}

/// Names the types a handle is for without holding one of them, and without
/// borrowing their auto traits: a handle is `Send` and `Sync` on the strength
/// of its channel alone.
type Marker<T, M> = PhantomData<fn() -> (T, M)>;

/// The channel a blocking handle uses when the caller supplies none: an
/// unbounded [`std::sync::mpsc`] queue, so a call never waits to be queued.
pub type DefaultSender<M> = std::sync::mpsc::Sender<M>;

/// The receiving half of [`DefaultSender`], which the actor thread reads.
pub type DefaultReceiver<M> = std::sync::mpsc::Receiver<M>;

impl<T, M, S: Clone> Clone for Handle<T, M, S> {
    fn clone(&self) -> Self {
        Handle {
            sender: self.sender.clone(),
            wait: self.wait,
            actor: PhantomData,
        }
    }
}

impl<T, M, S> Debug for Handle<T, M, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Handle<{}>", type_name::<T>())
    }
}

impl<T, M, S> Handle<T, M, S> {
    /// Only the builder makes one, and it is a child of this module.
    fn from_sender(sender: S, wait: Wait) -> Self {
        Handle {
            sender,
            wait,
            actor: PhantomData,
        }
    }
}

impl<T, M, S: JobSender<M>> Handle<T, M, S> {
    /// Queues one call and blocks until its reply arrives.
    ///
    /// The reply channel is the caller's to make, so that it carries the
    /// method's own return type rather than something erased.
    #[doc(hidden)]
    pub fn __call<R>(&self, message: M, answer: Answer<R>) -> R {
        if self.sender.send(message).is_ok() {
            if let Ok(res) = answer.recv(self.wait) {
                return res;
            }
        }
        report_actor_gone::<T>()
    }
}

/// The crate's own items that generated blocking code names.
///
/// Not part of the public API.
#[doc(hidden)]
pub mod __private {
    pub use super::builder::{builder, spawn};
    pub use super::next;
    pub use super::reply::{Answer, Reply, reply};
    pub use crate::actor::Actor;
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::mpsc::sync_channel;

    use super::Wait;
    use crate::actum;

    #[derive(Debug)]
    struct Counter(i32);

    #[actum(blocking)]
    impl Counter {
        fn add(&mut self, value: i32) -> i32 {
            self.0 += value;
            self.0
        }

        fn boom(&self) {
            panic!("{SYNC_PANIC_PAYLOAD}")
        }
    }

    const SYNC_PANIC_PAYLOAD: &str = "the actor's own panic message";

    /// A handle is its sending half and a word saying how to wait, and nothing
    /// else. The `Wait` is a byte that pads out to a word, which is the whole
    /// cost of the choice being a value rather than a type parameter. How big
    /// the sending half is belongs to the channel: std's is two words.
    #[test]
    fn test_a_handle_is_its_sender_and_a_wait() {
        assert_eq!(
            size_of::<CounterHandle>(),
            size_of::<(super::DefaultSender<CounterCall>, Wait)>(),
            "the handle carries nothing beyond its sender and its wait"
        );
        assert_eq!(
            size_of::<CounterHandle>() - size_of::<super::DefaultSender<CounterCall>>(),
            size_of::<usize>(),
            "the wait costs one padded word"
        );
    }

    /// Nothing runs until the caller puts the actor on a thread, so a handle
    /// whose actor was never started is a handle to nothing.
    #[test]
    fn test_a_built_actor_serves_only_once_it_is_running() {
        let (handle, actor) = CounterHandle::builder(Counter(0)).build();
        drop(actor);

        let called = catch_unwind(AssertUnwindSafe(|| handle.add(1)));
        assert!(called.is_err(), "the actor was never started");
    }

    /// A panicking method takes the actor's thread with it, and the caller
    /// waiting on that call learns the actor is gone rather than waiting
    /// forever. Both wait modes have to notice.
    #[test]
    fn test_a_panicking_method_reaches_its_caller() {
        for wait in [Wait::Park, Wait::Spin] {
            let (handle, actor) = CounterHandle::builder(Counter(0)).wait(wait).build();
            let running = std::thread::spawn(actor);

            let called = catch_unwind(AssertUnwindSafe(|| handle.boom()));
            let payload = called.expect_err("the call panics");
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .expect("a string payload");
            // The actor's own payload stays on the actor's thread: the caller
            // is told the actor is gone, not what it was doing.
            assert!(message.contains("no longer running"), "{wait:?}: {message}");
            assert!(!message.contains(SYNC_PANIC_PAYLOAD), "{wait:?}: {message}");

            assert!(running.join().is_err(), "{wait:?}: the thread unwound");
        }
    }

    /// One method panicking ends the actor for every handle on it, not only
    /// for the caller that tripped it.
    #[test]
    fn test_a_panic_kills_the_actor_for_every_clone() {
        let (handle, actor) = CounterHandle::builder(Counter(0)).build();
        let running = std::thread::spawn(actor);
        let other = handle.clone();

        let _ = catch_unwind(AssertUnwindSafe(|| handle.boom()));
        let _ = running.join();

        let called = catch_unwind(AssertUnwindSafe(|| other.add(1)));
        assert!(called.is_err(), "the actor is gone for this handle too");
    }

    /// A caller that panics while its call is in flight is the blocking
    /// analogue of a dropped call future: the actor answers into a slot nobody
    /// is holding, which must not stop it.
    #[test]
    fn test_a_panicking_caller_does_not_stop_the_actor() {
        let (handle, actor) = CounterHandle::builder(Counter(0)).build();
        let running = std::thread::spawn(actor);

        let caller = handle.clone();
        let panicked = std::thread::spawn(move || {
            caller.add(1);
            panic!("the caller gives up");
        });
        assert!(panicked.join().is_err(), "the caller panicked");

        assert_eq!(handle.add(1), 2, "the actor is still serving");

        drop(handle);
        running.join().unwrap();
    }

    /// A bounded channel makes a caller wait to be queued, which is the
    /// backpressure it was chosen for. The default one never does.
    #[test]
    fn test_a_bounded_channel_still_answers_every_call() {
        let (handle, actor) = CounterHandle::builder(Counter(0))
            .channel(sync_channel(1))
            .build();
        let running = std::thread::spawn(actor);

        for expected in 1..=50 {
            assert_eq!(handle.add(1), expected);
        }

        drop(handle);
        running.join().unwrap();
    }

    #[test]
    fn test_debug_names_the_actor_type() {
        let (handle, actor) = CounterHandle::builder(Counter(0)).build();
        let running = std::thread::spawn(actor);

        assert_eq!(
            format!("{handle:?}"),
            format!("CounterHandle<{}>", std::any::type_name::<Counter>())
        );

        drop(handle);
        running.join().unwrap();
    }
}

//! Actors that run on a thread, for callers that have no executor.
//!
//! `#[actify(blocking)]` generates the same message enum, handle and builder as
//! `#[actify]` does, with `async` taken out of all three: the actor's loop is a
//! plain `fn` that a [`std::thread`] runs, and a handle's methods are ordinary
//! synchronous calls that return when the actor has answered.
//!
//! ```
//! use actify::actify;
//!
//! #[derive(Clone, Debug, PartialEq)]
//! struct Counter(i32);
//!
//! #[actify(blocking)]
//! impl Counter {
//!     fn add(&mut self, value: i32) -> i32 {
//!         self.0 += value;
//!         self.0
//!     }
//! }
//!
//! let handle = CounterHandle::new(Counter(0));
//! assert_eq!(handle.add(2), 2);
//! assert_eq!(handle.get(), Counter(2));
//! ```
//!
//! # Waiting
//!
//! Two threads wait on every call: the actor waits for the next job, and the
//! caller waits for the reply. [`Wait`] decides how both of them do it, and is
//! chosen once when the actor is built.
//!
//! [`Wait::Park`], the default, blocks: the actor sits in the channel's `recv`
//! and the caller sits on a condvar, and neither costs anything while it waits.
//!
//! [`Wait::Spin`] busy-waits: both loop on [`std::hint::spin_loop`] and never
//! sleep. A reply arrives as fast as the hardware can carry it, at the price of
//! a core held for the whole wait — including while the actor is idle. It is
//! for a pinned thread with nothing else to do, and is the wrong default for
//! everything else.
//!
//! ```
//! # use actify::actify;
//! use actify::blocking::Wait;
//!
//! # #[derive(Clone, Debug, PartialEq)]
//! # struct Counter(i32);
//! # #[actify(blocking)]
//! # impl Counter {}
//! let (handle, actor) = CounterHandle::builder(Counter(0)).wait(Wait::Spin).build();
//! let running = std::thread::spawn(actor);
//!
//! assert_eq!(handle.get(), Counter(0));
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

mod builder;
mod channel;
mod handle;
mod message;
mod read_handle;
pub(crate) mod reply;

pub use builder::{DefaultChannel, HandleBuilder};
pub use channel::{JobReceiver, JobSender, Next};
pub use handle::{DefaultReceiver, DefaultSender, Handle};
pub use message::Builtin;
pub use read_handle::ReadHandle;

use crate::actor::Actor;

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

/// Runs an actor's loop inside the `actor` span that names the instance, and
/// inside the guard that reports why it stopped.
///
/// The async backend instruments a future with the span; a thread enters it
/// instead, which is simpler and needs no combinator. The guard is declared
/// after the span guard so that it drops first, putting the exit event inside
/// the span it belongs to.
///
/// `run` is the loop itself, which generated code supplies.
#[doc(hidden)]
pub fn serve<T, R, F>(rx: R, actor: Actor<T>, wait: Wait, run: F)
where
    T: Send + Sync + 'static,
    F: FnOnce(R, Actor<T>, Wait),
{
    let span = actor.span();
    let _entered = span.enter();
    let _guard = actor.exit_guard();
    run(rx, actor, wait)
}

/// The crate's own items that generated blocking code names.
///
/// Not part of the public API.
#[doc(hidden)]
pub mod __private {
    pub use super::builder::{builder, spawn};
    pub use super::message::run_builtin;
    pub use super::reply::{Reply, reply};
    pub use super::{next, serve};
    pub use crate::actor::Actor;
}

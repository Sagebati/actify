//! The job channel a blocking actor is served through.
//!
//! The async backend's [`JobSender`](crate::JobSender) and
//! [`JobReceiver`](crate::JobReceiver) are blanket-implemented over [`Sink`]
//! and [`Stream`], because those are the traits runtime-agnostic channel crates
//! agree on. Blocking channels agree on no such trait, so these two are
//! implemented for [`std::sync::mpsc`] here and left public for anything else:
//! a crossbeam, flume or ring-buffer channel is a two-line implementation.
//!
//! [`Sink`]: https://docs.rs/futures-sink/latest/futures_sink/trait.Sink.html
//! [`Stream`]: https://docs.rs/futures-core/latest/futures_core/trait.Stream.html

use std::sync::mpsc::{Receiver, Sender, SyncSender, TryRecvError};

use crate::channel::Closed;

/// What a non-blocking look at the channel found.
///
/// The three cases are what separates "come back later" from "never again",
/// which a blocking `recv` folds into one `Option`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next<M> {
    /// A job was waiting.
    Job(M),
    /// Nothing is waiting yet, but a sender is still alive.
    Empty,
    /// Every sending half is gone, so nothing will ever arrive.
    Closed,
}

/// The sending half of a blocking actor's job channel, as a handle holds it.
///
/// Sending takes `&self`, unlike the async backend's, because a blocking
/// channel's sender genuinely sends that way and parks the calling thread
/// rather than a waker. There is nothing to clone and nothing to serialise, so
/// a blocking handle stays shareable.
pub trait JobSender<M>: Clone + Send + Sync + 'static {
    /// Sends one job, waiting only if the channel applies backpressure.
    ///
    /// An unbounded channel never waits, which is what actify's own default is,
    /// so a call blocks on its reply alone. A bounded one blocks here, which is
    /// the backpressure being asked for; it parks whatever the handle's
    /// [`Wait`](super::Wait) is, because spinning while another thread is meant
    /// to drain the queue only makes it slower.
    fn send(&self, job: M) -> Result<(), Closed>;
}

/// The receiving half of a blocking actor's job channel, as its loop reads it.
///
/// The actor stops when `recv` yields `None` or `try_recv` yields
/// [`Next::Closed`], which a channel reports once every sending half has been
/// dropped. Since a handle holds the only sending half, that is the last handle
/// going away.
pub trait JobReceiver<M>: Send + 'static {
    /// Blocks until the next job arrives, or reports that no sender is left.
    fn recv(&mut self) -> Option<M>;

    /// Looks for a job without blocking, for an actor that busy-waits.
    fn try_recv(&mut self) -> Next<M>;
}

/// The unbounded default: a send never waits to be queued.
impl<M: Send + 'static> JobSender<M> for Sender<M> {
    fn send(&self, job: M) -> Result<(), Closed> {
        Sender::send(self, job).map_err(|_| Closed)
    }
}

/// A bounded channel, which is how backpressure is asked for.
impl<M: Send + 'static> JobSender<M> for SyncSender<M> {
    fn send(&self, job: M) -> Result<(), Closed> {
        SyncSender::send(self, job).map_err(|_| Closed)
    }
}

/// The receiving half of both of the above.
impl<M: Send + 'static> JobReceiver<M> for Receiver<M> {
    fn recv(&mut self) -> Option<M> {
        Receiver::recv(self).ok()
    }

    fn try_recv(&mut self) -> Next<M> {
        match Receiver::try_recv(self) {
            Ok(job) => Next::Job(job),
            Err(TryRecvError::Empty) => Next::Empty,
            Err(TryRecvError::Disconnected) => Next::Closed,
        }
    }
}

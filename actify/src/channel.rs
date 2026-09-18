//! The job channel an actor is served through.
//!
//! An actor reads its calls from one channel, which the caller supplies. The
//! two traits here name the halves of that channel, and blanket
//! implementations cover any [`Sink`] and [`Stream`], which is what the
//! runtime-agnostic channel crates provide.

use std::fmt;
use std::future::{Future, poll_fn};
use std::pin::Pin;

use futures_core::Stream;
use futures_sink::Sink;

/// The actor is gone: nothing is left to serve the call.
///
/// The sending half reports this once the actor's receiving half has been
/// dropped, which happens when the actor future ends or is never spawned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Closed;

impl fmt::Display for Closed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the actor is no longer serving jobs")
    }
}

impl std::error::Error for Closed {}

/// The sending half of an actor's job channel, as a [`Handle`] holds it.
///
/// Sending takes `&self` because a handle is shared and cloned freely. Every
/// channel crate sends that way; [`Sink`] does not, which is what the blanket
/// implementation below bridges.
///
/// [`Handle`]: crate::Handle
pub trait JobSender<M>: Clone + Send + Sync + 'static {
    /// Sends one job, waiting only if the channel applies backpressure.
    ///
    /// An unbounded channel never waits, which is what actify's own default
    /// is, so a call's `await` covers the reply alone.
    fn send(&self, job: M) -> impl Future<Output = Result<(), Closed>> + Send;
}

/// The receiving half of an actor's job channel, as the actor future reads it.
///
/// The actor stops when this yields `None`, which a channel reports once every
/// sending half has been dropped. Since a [`Handle`] holds the only sending
/// half, that is the last handle going away.
///
/// [`Handle`]: crate::Handle
pub trait JobReceiver<M>: Send + 'static {
    /// Waits for the next job, or reports that no sender is left.
    fn recv(&mut self) -> impl Future<Output = Option<M>> + Send;
}

/// Any [`Sink`] that a shared handle can send through.
///
/// `Clone` is what makes `&self` sending possible: a sink sends through
/// `&mut self`, so each call sends through its own clone. Every channel's
/// sender is a cheap handle onto shared state, so the clone costs a reference
/// count rather than a copy of the queue.
impl<M, S> JobSender<M> for S
where
    M: Send + 'static,
    S: Sink<M> + Clone + Unpin + Send + Sync + 'static,
{
    async fn send(&self, job: M) -> Result<(), Closed> {
        let mut sink = self.clone();
        poll_fn(|cx| Pin::new(&mut sink).poll_ready(cx))
            .await
            .map_err(|_| Closed)?;
        Pin::new(&mut sink).start_send(job).map_err(|_| Closed)?;
        poll_fn(|cx| Pin::new(&mut sink).poll_flush(cx))
            .await
            .map_err(|_| Closed)
    }
}

/// Any [`Stream`] the actor future can read its jobs from.
impl<M, R> JobReceiver<M> for R
where
    M: Send,
    R: Stream<Item = M> + Unpin + Send + 'static,
{
    async fn recv(&mut self) -> Option<M> {
        poll_fn(|cx| Pin::new(&mut *self).poll_next(cx)).await
    }
}

use std::fmt::{self, Debug};
use std::future::Future;
use std::marker::PhantomData;
use std::panic::Location;

use super::handle::{DefaultReceiver, DefaultSender, Handle, ToView};
use crate::actor::{Actor, Dispatch, serve};
use crate::channel::{JobReceiver, JobSender};
use crate::message::Job;

/// Starts a builder for an actor served with the message type `M`.
///
/// [`Handle::builder`] fixes the message to the library's own; generated code
/// calls this to fix it to the actor's.
#[doc(hidden)]
#[track_caller]
pub fn builder<T, V, M>(val: T) -> HandleBuilder<T, V, M>
where
    T: ToView<V> + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
    M: Dispatch<T>,
{
    HandleBuilder::new(val)
}

/// Builds an actor, spawns it on Tokio and returns its handle, as
/// [`Handle::new`] does, with the message type `M`.
#[cfg(feature = "tokio")]
#[doc(hidden)]
#[track_caller]
pub fn spawn<T, V, M>(val: T) -> Handle<T, V, M>
where
    T: ToView<V> + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
    M: Dispatch<T>,
{
    let (handle, actor) = builder::<T, V, M>(val).build();
    tokio::spawn(actor);
    handle
}

/// A builder that has not been given a channel, so [`build`] makes the default
/// one.
///
/// [`build`]: HandleBuilder::build
#[derive(Debug)]
pub struct DefaultChannel;

/// Builds a [`Handle`] and the actor future that serves it, without spawning
/// anything.
///
/// Created by [`Handle::builder`]. [`build`](Self::build) hands back both
/// halves, so the caller chooses the executor:
///
/// ```
/// # use actify::Handle;
/// # #[tokio::main]
/// # async fn main() {
/// let (handle, actor) = Handle::builder(0).build();
/// tokio::spawn(actor);
///
/// handle.set(1).await;
/// assert_eq!(handle.get().await, 1);
/// # }
/// ```
///
/// [`channel`](Self::channel) replaces the default queue with one the caller
/// owns, which is how the bound and the backing crate are chosen:
///
/// ```
/// # use actify::Handle;
/// # #[tokio::main]
/// # async fn main() {
/// let (tx, rx) = futures_channel::mpsc::channel(8);
///
/// let (handle, actor) = Handle::builder(0).channel((tx, rx)).build();
/// tokio::spawn(actor);
///
/// assert_eq!(handle.get().await, 0);
/// # }
/// ```
pub struct HandleBuilder<T, V = T, M = Job<T, V>, C = DefaultChannel> {
    val: T,
    spawned_at: &'static Location<'static>,
    channel: C,
    view: PhantomData<fn() -> (V, M)>,
}

impl<T, V, M, C> Debug for HandleBuilder<T, V, M, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HandleBuilder")
            .field("spawned_at", &self.spawned_at)
            .finish_non_exhaustive()
    }
}

impl<T, V, M> HandleBuilder<T, V, M, DefaultChannel>
where
    T: ToView<V> + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
    M: Dispatch<T>,
{
    /// Captures the call site so the actor's span names where it was built.
    #[track_caller]
    pub(crate) fn new(val: T) -> Self {
        HandleBuilder {
            val,
            spawned_at: Location::caller(),
            channel: DefaultChannel,
            view: PhantomData,
        }
    }

    /// Serves the actor through a channel the caller owns, in place of the
    /// default one.
    ///
    /// The halves go in the order every channel constructor returns them, so
    /// `flume::unbounded()` or `futures_channel::mpsc::channel(8)` can be
    /// passed straight through. Both halves are taken by value: a sending half
    /// kept back would outlive the last handle and keep the actor running.
    ///
    /// Whether a call waits to be queued is the channel's choice, so a bounded
    /// channel is how backpressure is asked for.
    pub fn channel<S, R>(self, channel: (S, R)) -> HandleBuilder<T, V, M, (S, R)>
    where
        S: JobSender<M>,
        R: JobReceiver<M>,
    {
        HandleBuilder {
            val: self.val,
            spawned_at: self.spawned_at,
            channel,
            view: PhantomData,
        }
    }

    /// Returns the handle and the future that serves it, over the default
    /// channel.
    ///
    /// The actor does nothing until the future is polled, so nothing runs
    /// until the caller spawns it. Dropping it without spawning leaves a
    /// handle whose every call panics, reporting that the actor is not
    /// running.
    pub fn build(self) -> (Handle<T, V, M>, impl Future<Output = ()> + Send) {
        let (tx, rx): (DefaultSender<M>, DefaultReceiver<M>) = futures_channel::mpsc::unbounded();
        self.with_channel(tx, rx)
    }
}

impl<T, V, M, S, R> HandleBuilder<T, V, M, (S, R)>
where
    T: ToView<V> + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
    M: Dispatch<T>,
    S: JobSender<M>,
    R: JobReceiver<M>,
{
    /// Returns the handle and the future that serves it, over the channel
    /// given to [`channel`](HandleBuilder::channel).
    ///
    /// The actor does nothing until the future is polled, so nothing runs
    /// until the caller spawns it.
    pub fn build(self) -> (Handle<T, V, M, S>, impl Future<Output = ()> + Send) {
        let (tx, rx) = self.channel;
        HandleBuilder {
            val: self.val,
            spawned_at: self.spawned_at,
            channel: DefaultChannel,
            view: PhantomData,
        }
        .with_channel(tx, rx)
    }
}

impl<T, V, M> HandleBuilder<T, V, M, DefaultChannel>
where
    T: ToView<V> + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
    M: Dispatch<T>,
{
    fn with_channel<S, R>(
        self,
        tx: S,
        rx: R,
    ) -> (Handle<T, V, M, S>, impl Future<Output = ()> + Send)
    where
        S: JobSender<M>,
        R: JobReceiver<M>,
    {
        let actor = Actor::new(self.val, self.spawned_at);
        (Handle::from_sender(tx), serve(rx, actor))
    }
}

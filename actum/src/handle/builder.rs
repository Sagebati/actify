//! Building an async actor, and choosing where it runs.

use std::fmt::{self, Debug};
use std::future::Future;
use std::marker::PhantomData;
use std::panic::Location;

use futures_core::Stream;
use futures_sink::Sink;

use super::{DefaultReceiver, DefaultSender, Handle};
use crate::actor::Actor;

/// Starts a builder for an actor served with the message type `M`.
///
/// Generated code calls this, which is the only way to build an actor.
#[doc(hidden)]
#[track_caller]
pub fn builder<T, M>(val: T) -> HandleBuilder<T, M> {
    HandleBuilder::new(val)
}

/// Builds an actor, spawns its loop on Tokio and returns its handle.
#[cfg(feature = "tokio")]
#[doc(hidden)]
#[track_caller]
pub fn spawn<T, M, F, Fut>(val: T, run: F) -> Handle<T, M>
where
    T: Send + Sync + 'static,
    M: Send + 'static,
    F: FnOnce(DefaultReceiver<M>, Actor<T>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (handle, actor) = builder(val).build(run);
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
/// Created by the `builder` on a generated handle, which wraps this one and
/// hands back both halves so the caller chooses the executor:
///
/// ```
/// # use actum::actum;
/// # #[derive(Debug)]
/// # struct Counter(i32);
/// # #[actum]
/// # impl Counter {
/// #     fn add(&mut self, value: i32) -> i32 { self.0 += value; self.0 }
/// # }
/// # #[tokio::main]
/// # async fn main() {
/// let (mut handle, actor) = CounterHandle::builder(Counter(0)).build();
/// tokio::spawn(actor);
///
/// assert_eq!(handle.add(1).await, 1);
/// # }
/// ```
///
/// [`channel`](Self::channel) replaces the default queue with one the caller
/// owns, which is how the bound and the backing crate are chosen. Any
/// [`Sink`] and [`Stream`] pair over the message type qualifies:
///
/// ```
/// # use actum::actum;
/// # #[derive(Debug)]
/// # struct Counter(i32);
/// # #[actum]
/// # impl Counter {
/// #     fn add(&mut self, value: i32) -> i32 { self.0 += value; self.0 }
/// # }
/// # #[tokio::main]
/// # async fn main() {
/// let (tx, rx) = futures_channel::mpsc::channel(8);
///
/// let (mut handle, actor) = CounterHandle::builder(Counter(0)).channel((tx, rx)).build();
/// tokio::spawn(actor);
///
/// assert_eq!(handle.add(1).await, 1);
/// # }
/// ```
pub struct HandleBuilder<T, M, C = DefaultChannel> {
    val: T,
    spawned_at: &'static Location<'static>,
    channel: C,
    message: PhantomData<fn() -> M>,
}

impl<T, M, C> Debug for HandleBuilder<T, M, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HandleBuilder")
            .field("spawned_at", &self.spawned_at)
            .finish_non_exhaustive()
    }
}

impl<T, M> HandleBuilder<T, M, DefaultChannel> {
    /// Captures the call site so the actor's span names where it was built.
    #[track_caller]
    fn new(val: T) -> Self {
        HandleBuilder {
            val,
            spawned_at: Location::caller(),
            channel: DefaultChannel,
            message: PhantomData,
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
    /// channel is how backpressure is asked for. What it bounds is the
    /// channel's business: `futures_channel` holds `buffer + one slot per
    /// sender`, and a handle is a sender, so a program raises its own ceiling
    /// by cloning handles. Concurrency costs handles either way, since a
    /// handle sends through `&mut` and so carries one call at a time.
    pub fn channel<S, R>(self, channel: (S, R)) -> HandleBuilder<T, M, (S, R)>
    where
        S: Sink<M>,
        R: Stream<Item = M>,
    {
        HandleBuilder {
            val: self.val,
            spawned_at: self.spawned_at,
            channel,
            message: PhantomData,
        }
    }

    /// Returns the handle and the future that serves it, over the default
    /// channel.
    ///
    /// The actor does nothing until the future is polled, so nothing runs
    /// until the caller spawns it. Dropping it without spawning leaves a
    /// handle whose every call panics, reporting that the actor is not
    /// running.
    pub fn build<F, Fut>(self, run: F) -> (Handle<T, M>, impl Future<Output = ()> + Send)
    where
        T: Send + Sync + 'static,
        M: Send + 'static,
        F: FnOnce(DefaultReceiver<M>, Actor<T>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send,
    {
        let (tx, rx): (DefaultSender<M>, DefaultReceiver<M>) = futures_channel::mpsc::unbounded();
        self.with_channel(tx, rx, run)
    }
}

impl<T, M, S, R> HandleBuilder<T, M, (S, R)> {
    /// Returns the handle and the future that serves it, over the channel
    /// given to [`channel`](HandleBuilder::channel).
    ///
    /// The actor does nothing until the future is polled, so nothing runs
    /// until the caller spawns it.
    pub fn build<F, Fut>(self, run: F) -> (Handle<T, M, S>, impl Future<Output = ()> + Send)
    where
        T: Send + Sync + 'static,
        R: Send + 'static,
        F: FnOnce(R, Actor<T>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send,
    {
        let (tx, rx) = self.channel;
        HandleBuilder {
            val: self.val,
            spawned_at: self.spawned_at,
            channel: DefaultChannel,
            message: PhantomData,
        }
        .with_channel(tx, rx, run)
    }
}

impl<T, M> HandleBuilder<T, M, DefaultChannel> {
    fn with_channel<S, R, F, Fut>(
        self,
        tx: S,
        rx: R,
        run: F,
    ) -> (Handle<T, M, S>, impl Future<Output = ()> + Send)
    where
        T: Send + Sync + 'static,
        R: Send + 'static,
        F: FnOnce(R, Actor<T>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send,
    {
        (
            Handle::from_sender(tx),
            Actor::serve(self.val, self.spawned_at, rx, run),
        )
    }
}

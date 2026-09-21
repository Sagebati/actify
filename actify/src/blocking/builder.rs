//! Building a blocking actor, and putting it on a thread.

use std::fmt::{self, Debug};
use std::marker::PhantomData;
use std::panic::Location;

use super::Wait;
use super::channel::{JobReceiver, JobSender};
use super::handle::{DefaultReceiver, DefaultSender, Handle};
use super::serve;
use crate::actor::Actor;
use crate::handles::ToView;

/// Starts a builder for a blocking actor served with the message type `M`.
///
/// Generated code calls this, which is the only way to build an actor.
#[doc(hidden)]
#[track_caller]
pub fn builder<T, V, M>(val: T) -> HandleBuilder<T, V, M>
where
    T: ToView<V> + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    HandleBuilder::new(val)
}

/// Builds a blocking actor, puts its loop on a thread and returns its handle.
///
/// The [`JoinHandle`](std::thread::JoinHandle) is dropped, so the thread is
/// detached: it ends when the last handle does. Use
/// [`build`](HandleBuilder::build) to keep it.
#[doc(hidden)]
#[track_caller]
pub fn spawn<T, V, M, F>(val: T, run: F) -> Handle<T, V, M>
where
    T: ToView<V> + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
    M: Send + 'static,
    F: FnOnce(DefaultReceiver<M>, Actor<T>, Wait) + Send + 'static,
{
    let (handle, actor) = builder::<T, V, M>(val).build(run);
    std::thread::spawn(actor);
    handle
}

/// A builder that has not been given a channel, so [`build`] makes the default
/// one.
///
/// [`build`]: HandleBuilder::build
#[derive(Debug)]
pub struct DefaultChannel;

/// Builds a blocking [`Handle`] and the closure that serves it, without
/// starting a thread.
///
/// Created by the `builder` on a generated handle, which wraps this one and
/// hands back both halves so the caller chooses where the actor runs:
///
/// ```
/// # use actify::actify;
/// # #[derive(Clone, Debug, PartialEq)]
/// # struct Counter(i32);
/// # #[actify(blocking)]
/// # impl Counter {}
/// let (handle, actor) = CounterHandle::builder(Counter(0)).build();
/// let running = std::thread::spawn(actor);
///
/// handle.set(Counter(1));
/// assert_eq!(handle.get(), Counter(1));
///
/// drop(handle);
/// running.join().unwrap();
/// ```
pub struct HandleBuilder<T, V, M, C = DefaultChannel> {
    val: T,
    spawned_at: &'static Location<'static>,
    channel: C,
    wait: Wait,
    view: PhantomData<fn() -> (V, M)>,
}

impl<T, V, M, C> Debug for HandleBuilder<T, V, M, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HandleBuilder")
            .field("spawned_at", &self.spawned_at)
            .field("wait", &self.wait)
            .finish_non_exhaustive()
    }
}

impl<T, V, M, C> HandleBuilder<T, V, M, C> {
    /// Chooses how the actor waits for jobs and how a caller waits for replies.
    ///
    /// [`Wait::Park`] by default. See [the module docs](super#waiting) for what
    /// [`Wait::Spin`] costs.
    pub fn wait(self, wait: Wait) -> Self {
        HandleBuilder { wait, ..self }
    }
}

impl<T, V, M> HandleBuilder<T, V, M, DefaultChannel>
where
    T: ToView<V> + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    /// Captures the call site so the actor's span names where it was built.
    #[track_caller]
    pub(crate) fn new(val: T) -> Self {
        HandleBuilder {
            val,
            spawned_at: Location::caller(),
            channel: DefaultChannel,
            wait: Wait::Park,
            view: PhantomData,
        }
    }

    /// Serves the actor through a channel the caller owns, in place of the
    /// default one.
    ///
    /// The halves go in the order every channel constructor returns them, so
    /// `std::sync::mpsc::sync_channel(8)` can be passed straight through. Both
    /// halves are taken by value: a sending half kept back would outlive the
    /// last handle and keep the actor running.
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
            wait: self.wait,
            view: PhantomData,
        }
    }

    /// Returns the handle and the closure that serves it, over the default
    /// channel.
    ///
    /// The actor does nothing until the closure is called, so nothing runs
    /// until the caller puts it on a thread. Dropping it without doing so
    /// leaves a handle whose every call panics, reporting that the actor is not
    /// running.
    pub fn build<F>(self, run: F) -> (Handle<T, V, M>, impl FnOnce() + Send + 'static)
    where
        M: Send + 'static,
        F: FnOnce(DefaultReceiver<M>, Actor<T>, Wait) + Send + 'static,
    {
        let (tx, rx): (DefaultSender<M>, DefaultReceiver<M>) = std::sync::mpsc::channel();
        self.with_channel(tx, rx, run)
    }
}

impl<T, V, M, S, R> HandleBuilder<T, V, M, (S, R)>
where
    T: ToView<V> + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
    S: JobSender<M>,
    R: JobReceiver<M>,
{
    /// Returns the handle and the closure that serves it, over the channel
    /// given to [`channel`](HandleBuilder::channel).
    pub fn build<F>(self, run: F) -> (Handle<T, V, M, S>, impl FnOnce() + Send + 'static)
    where
        F: FnOnce(R, Actor<T>, Wait) + Send + 'static,
    {
        let (tx, rx) = self.channel;
        HandleBuilder {
            val: self.val,
            spawned_at: self.spawned_at,
            channel: DefaultChannel,
            wait: self.wait,
            view: PhantomData,
        }
        .with_channel(tx, rx, run)
    }
}

impl<T, V, M> HandleBuilder<T, V, M, DefaultChannel>
where
    T: ToView<V> + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    fn with_channel<S, R, F>(
        self,
        tx: S,
        rx: R,
        run: F,
    ) -> (Handle<T, V, M, S>, impl FnOnce() + Send + 'static)
    where
        S: JobSender<M>,
        R: Send + 'static,
        F: FnOnce(R, Actor<T>, Wait) + Send + 'static,
    {
        let actor = Actor::new(self.val, self.spawned_at);
        let wait = self.wait;
        (Handle::from_sender(tx, wait), move || {
            serve(rx, actor, wait, run)
        })
    }
}

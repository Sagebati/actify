use std::fmt::{self, Debug};
use std::future::Future;
use std::marker::PhantomData;
use std::panic::Location;
use std::sync::Arc;

use super::handle::{CHANNEL_SIZE, Channels, Handle, ToView};
use crate::actor::{Actor, serve};

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
pub struct HandleBuilder<T, V = T> {
    val: T,
    spawned_at: &'static Location<'static>,
    view: PhantomData<fn() -> V>,
}

impl<T, V> Debug for HandleBuilder<T, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HandleBuilder")
            .field("spawned_at", &self.spawned_at)
            .finish_non_exhaustive()
    }
}

impl<T, V> HandleBuilder<T, V>
where
    T: ToView<V> + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    /// Captures the call site so the actor's span names where it was built.
    #[track_caller]
    pub(super) fn new(val: T) -> Self {
        HandleBuilder {
            val,
            spawned_at: Location::caller(),
            view: PhantomData,
        }
    }

    /// Returns the handle and the future that serves it.
    ///
    /// The actor does nothing until the future is polled, so nothing runs
    /// until the caller spawns it. Dropping it without spawning leaves a
    /// handle whose every call panics, reporting that the actor is not
    /// running.
    pub fn build(self) -> (Handle<T, V>, impl Future<Output = ()> + Send) {
        let (tx, rx) = tokio::sync::mpsc::channel(CHANNEL_SIZE);
        let actor = Actor::new(self.val, self.spawned_at);
        let handle = Handle::from_channels(Arc::new(Channels { tx }));
        (handle, serve(rx, actor))
    }
}

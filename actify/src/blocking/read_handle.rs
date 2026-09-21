//! A read-only blocking handle.

use std::any::type_name;
use std::fmt::{self, Debug};

use super::channel::JobSender;
use super::handle::{DefaultSender, Handle};
use crate::handles::ToView;

/// A clonable read-only handle that can only be used to read the internal
/// value.
///
/// Obtained via [`Handle::read_handle`]. Supports [`ReadHandle::get`].
pub struct ReadHandle<T, V, M, S = DefaultSender<M>>(Handle<T, V, M, S>);

impl<T, V, M, S> Clone for ReadHandle<T, V, M, S> {
    fn clone(&self) -> Self {
        ReadHandle(self.0.clone())
    }
}

impl<T, V, M, S> Debug for ReadHandle<T, V, M, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let actor = type_name::<T>();
        let view = type_name::<V>();
        if actor == view {
            write!(f, "ReadHandle<{actor}>")
        } else {
            write!(f, "ReadHandle<{actor}, {view}>")
        }
    }
}

impl<T, V, M, S> ReadHandle<T, V, M, S> {
    pub(super) fn new(handle: Handle<T, V, M, S>) -> Self {
        ReadHandle(handle)
    }
}

impl<T, V, M, S> ReadHandle<T, V, M, S>
where
    T: ToView<V> + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
    M: From<super::message::Builtin<T, V>> + Send + 'static,
    S: JobSender<M>,
{
    /// Returns the actor's current view. See [`Handle::get`].
    ///
    /// # Panics
    ///
    /// Panics if the actor has stopped.
    pub fn get(&self) -> V {
        self.0.get()
    }
}

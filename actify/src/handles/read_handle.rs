use std::any::type_name;
use std::fmt::{self, Debug};

use super::handle::{Handle, ToView};

/// A clonable read-only handle that can only be used to read the internal value.
///
/// Obtained via [`Handle::read_handle`]. Supports [`ReadHandle::get`].
pub struct ReadHandle<T, V = T, M = crate::message::Job<T, V>, S = crate::handles::DefaultSender<M>>(
    Handle<T, V, M, S>,
);

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
    M: From<crate::message::Builtin<T, V>> + Send + 'static,
    S: crate::channel::JobSender<M>,
{
    /// Returns the actor's current view. See [`Handle::get`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use actify::Handle;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let handle = Handle::new(1);
    /// let read_handle = handle.read_handle();
    /// let result = read_handle.get().await;
    /// assert_eq!(result, 1);
    /// # }
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the actor has stopped, either because one of its methods
    /// panicked or because its runtime shut down. See [Actor lifetime and
    /// panics](crate#actor-lifetime-and-panics).
    pub async fn get(&self) -> V {
        self.0.get().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_handle_is_pointer_sized() {
        assert_eq!(size_of::<ReadHandle<u8>>(), size_of::<usize>());
    }

    #[tokio::test]
    async fn test_read_handle() {
        let handle = Handle::new(1);
        let read_handle = handle.read_handle();
        assert_eq!(read_handle.get().await, 1);

        handle.set(2).await;
        assert_eq!(read_handle.get().await, 2);
    }

    #[derive(Clone, Debug)]
    struct Counted(Vec<u8>);

    impl crate::ToView<usize> for Counted {
        fn to_view(&self) -> usize {
            self.0.len()
        }
    }

    #[tokio::test]
    async fn test_debug_names_the_view_only_when_it_differs() {
        let plain: Handle<i32> = Handle::new(1);
        assert_eq!(format!("{:?}", plain.read_handle()), "ReadHandle<i32>");

        let viewed: Handle<Counted, usize> = Handle::new(Counted(vec![1, 2]));
        assert_eq!(
            format!("{:?}", viewed.read_handle()),
            format!("ReadHandle<{}, usize>", type_name::<Counted>())
        );
    }
}

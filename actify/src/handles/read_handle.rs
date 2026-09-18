use std::any::type_name;
use std::fmt::{self, Debug};
use tokio::sync::broadcast;

use super::handle::{Handle, ToView};

/// A clonable read-only handle that can only be used to read the internal value.
///
/// Obtained via [`Handle::read_handle`]. Supports [`ReadHandle::get`],
/// [`ReadHandle::with`] and [`ReadHandle::subscribe`].
pub struct ReadHandle<T, V = T>(Handle<T, V>);

impl<T, V> Clone for ReadHandle<T, V> {
    fn clone(&self) -> Self {
        ReadHandle(self.0.clone())
    }
}

impl<T, V> Debug for ReadHandle<T, V> {
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

impl<T, V> ReadHandle<T, V> {
    /// Returns a [`tokio::sync::broadcast::Receiver`] that receives all broadcasted values.
    ///
    /// # Examples
    ///
    /// ```
    /// # use actify::Handle;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let handle = Handle::new(None);
    /// let read_handle = handle.read_handle();
    /// let mut rx = read_handle.subscribe();
    /// handle.set(Some("testing!")).await;
    /// assert_eq!(rx.recv().await.unwrap(), Some("testing!"));
    /// # }
    /// ```
    pub fn subscribe(&self) -> broadcast::Receiver<V> {
        self.0.subscribe()
    }

    pub(super) fn new(handle: Handle<T, V>) -> Self {
        ReadHandle(handle)
    }
}

impl<T: Send + Sync + 'static, V> ReadHandle<T, V> {
    /// Runs a read-only closure on the actor's value and returns the result.
    ///
    /// Unlike [`ReadHandle::get`], which returns the view, this reads the actor
    /// type itself.
    ///
    /// # Examples
    ///
    /// ```
    /// # use actify::{Handle, ToView};
    /// # #[tokio::main]
    /// # async fn main() {
    /// // A non-Clone type, so its view is a separate type
    /// struct Inventory { items: Vec<String> }
    ///
    /// #[derive(Clone, Debug)]
    /// struct Count(usize);
    ///
    /// impl ToView<Count> for Inventory {
    ///     fn to_view(&self) -> Count { Count(self.items.len()) }
    /// }
    ///
    /// let handle: Handle<Inventory, Count> = Handle::new(Inventory {
    ///     items: vec!["sword".into(), "shield".into()],
    /// });
    /// let read_handle = handle.read_handle();
    ///
    /// let count = read_handle.with(|inv| inv.items.len()).await;
    /// assert_eq!(count, 2);
    ///
    /// let first = read_handle.with(|inv| inv.items[0].clone()).await;
    /// assert_eq!(first, "sword");
    /// # }
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the actor has stopped, either because one of its methods
    /// panicked or because its runtime shut down. See [Actor lifetime and
    /// panics](crate#actor-lifetime-and-panics).
    pub async fn with<R, F>(&self, f: F) -> R
    where
        F: FnOnce(&T) -> R + Send + 'static,
        R: Send + 'static,
    {
        self.0.with(f).await
    }
}

impl<T, V> ReadHandle<T, V>
where
    T: ToView<V> + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
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

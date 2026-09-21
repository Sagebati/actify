use std::any::type_name;
use std::fmt::{self, Debug};

use super::handle::{Handle, ToView};

/// A clonable read-only handle that can only be used to read the internal value.
///
/// Obtained via [`Handle::read_handle`]. Supports [`ReadHandle::get`].
pub struct ReadHandle<T, V, M, S = crate::handles::DefaultSender<M>>(Handle<T, V, M, S>);

impl<T, V, M, S: Clone> Clone for ReadHandle<T, V, M, S> {
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
    /// # use actify::actify;
    /// # #[derive(Clone, Debug, PartialEq)]
    /// # struct Counter(i32);
    /// # #[actify]
    /// # impl Counter {}
    /// # #[tokio::main]
    /// # async fn main() {
    /// let mut handle = CounterHandle::new(Counter(1));
    /// let mut read_handle = handle.read_handle();
    /// let result = read_handle.get().await;
    /// assert_eq!(result, Counter(1));
    /// # }
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the actor has stopped, either because one of its methods
    /// panicked or because its runtime shut down. See [Actor lifetime and
    /// panics](crate#actor-lifetime-and-panics).
    pub async fn get(&mut self) -> V {
        self.0.get().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actify_macros::actify;

    #[derive(Clone, Debug)]
    struct Counter(i32);

    #[actify]
    impl Counter {}

    #[derive(Clone, Debug)]
    struct Counted(Vec<u8>);

    #[actify]
    impl Counted {}

    impl crate::ToView<usize> for Counted {
        fn to_view(&self) -> usize {
            self.0.len()
        }
    }

    #[test]
    fn test_read_handle_is_pointer_sized() {
        assert_eq!(
            size_of::<ReadHandle<Counter, Counter, CounterCall>>(),
            size_of::<usize>()
        );
    }

    #[tokio::test]
    async fn test_read_handle() {
        let mut handle = CounterHandle::new(Counter(1));
        let mut read_handle = handle.read_handle();
        assert_eq!(read_handle.get().await.0, 1);

        handle.set(Counter(2)).await;
        assert_eq!(read_handle.get().await.0, 2);
    }

    #[tokio::test]
    async fn test_debug_names_the_view_only_when_it_differs() {
        let plain = CounterHandle::new(Counter(1));
        assert_eq!(
            format!("{:?}", plain.read_handle()),
            format!("ReadHandle<{}>", type_name::<Counter>())
        );

        let viewed = CountedHandle::<usize>::new(Counted(vec![1, 2]));
        assert_eq!(
            format!("{:?}", viewed.read_handle()),
            format!("ReadHandle<{}, usize>", type_name::<Counted>())
        );
    }
}

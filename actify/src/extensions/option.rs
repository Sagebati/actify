use actify_macros::actify;

/// An extension trait for `Option<T>` actors, made available on the [`Handle`](crate::Handle)
/// as [`OptionHandle`](crate::OptionHandle).
trait ActorOption<T> {
    fn is_some(&self) -> bool;

    fn is_none(&self) -> bool;

    fn take(&mut self) -> Option<T>;

    fn replace(&mut self, value: T) -> Option<T>;

    fn unwrap_or(&self, default: T) -> T;

    fn unwrap_or_default(&self) -> T
    where
        T: Default;
}

/// Methods on [`OptionHandle`](crate::OptionHandle), for an actor holding a `Option<T>>`, exposed as [`OptionHandle`](crate::OptionHandle).
#[actify]
impl<T> ActorOption<T> for Option<T>
where
    T: Clone + Send + Sync + 'static,
{
    /// Returns true if the option is a Some value.
    ///
    /// # Examples
    ///
    /// ```
    /// # use actify::OptionHandle;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let handle = OptionHandle::new(Some(1));
    /// assert!(handle.is_some().await);
    /// # }
    /// ```
    fn is_some(&self) -> bool {
        self.is_some()
    }

    /// Returns true if the option is a None value.
    ///
    /// # Examples
    ///
    /// ```
    /// # use actify::OptionHandle;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let handle = OptionHandle::new(Option::<i32>::None);
    /// assert!(handle.is_none().await);
    /// # }
    /// ```
    fn is_none(&self) -> bool {
        self.is_none()
    }

    /// Takes the value out of the option, leaving a None in its place.
    ///
    /// # Examples
    ///
    /// ```
    /// # use actify::OptionHandle;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let handle = OptionHandle::new(Some(42));
    /// assert_eq!(handle.take().await, Some(42));
    /// assert!(handle.is_none().await);
    /// # }
    /// ```
    fn take(&mut self) -> Option<T> {
        self.take()
    }

    /// Replaces the value in the option, returning the old value if present.
    ///
    /// # Examples
    ///
    /// ```
    /// # use actify::OptionHandle;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let handle = OptionHandle::new(Some(1));
    /// assert_eq!(handle.replace(2).await, Some(1));
    /// assert_eq!(handle.get().await, Some(2));
    /// # }
    /// ```
    fn replace(&mut self, value: T) -> Option<T> {
        self.replace(value)
    }

    /// Returns the contained value or a provided default.
    ///
    /// # Examples
    ///
    /// ```
    /// # use actify::OptionHandle;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let handle = OptionHandle::new(Some(10));
    /// assert_eq!(handle.unwrap_or(0).await, 10);
    ///
    /// let handle = OptionHandle::new(Option::<i32>::None);
    /// assert_eq!(handle.unwrap_or(0).await, 0);
    /// # }
    /// ```
    fn unwrap_or(&self, default: T) -> T {
        self.clone().unwrap_or(default)
    }

    /// Returns the contained value or a default.
    ///
    /// # Examples
    ///
    /// ```
    /// # use actify::OptionHandle;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let handle = OptionHandle::new(Option::<i32>::None);
    /// assert_eq!(handle.unwrap_or_default().await, 0);
    /// # }
    /// ```
    fn unwrap_or_default(&self) -> T
    where
        T: Default,
    {
        self.clone().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_the_defaults_apply_only_when_none() {
        let handle: OptionHandle<i32> = OptionHandle::new(None);

        assert_eq!(handle.unwrap_or(9).await, 9);
        assert_eq!(handle.unwrap_or_default().await, 0);
        assert!(handle.is_none().await);
    }

    #[tokio::test]
    async fn test_take_when_none() {
        let handle = OptionHandle::new(Option::<i32>::None);

        assert_eq!(handle.take().await, None);
        assert!(handle.is_none().await);
    }
}

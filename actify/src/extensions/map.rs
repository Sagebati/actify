use actify_macros::actify;
use std::collections::HashMap;
use std::hash::Hash;

/// An extension trait for `HashMap<K, V>` actors, made available on the [`Handle`](crate::Handle)
/// as [`HashMapHandle`](crate::HashMapHandle).
trait ActorMap<K, V> {
    fn get_key(&self, key: K) -> Option<V>;

    fn insert(&mut self, key: K, val: V) -> Option<V>;

    fn remove(&mut self, key: K) -> Option<V>;

    fn clear(&mut self);

    fn is_empty(&self) -> bool;

    fn keys(&self) -> Vec<K>;

    fn values(&self) -> Vec<V>;

    fn len(&self) -> usize;

    fn contains_key(&self, key: K) -> bool;

    fn drain(&mut self) -> Vec<(K, V)>;

    fn extend(&mut self, items: Vec<(K, V)>);

    fn remove_entry(&mut self, key: K) -> Option<(K, V)>;
}

/// Methods on [`HashMapHandle`](crate::HashMapHandle), for an actor holding a `HashMap<K, V>>`, exposed as [`HashMapHandle`](crate::HashMapHandle).
#[actify]
impl<K, V> ActorMap<K, V> for HashMap<K, V>
where
    K: Clone + Eq + Hash + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    /// Returns a clone of the value corresponding to the key if it exists
    /// It is equivalent to the Hashmap get(), but the method name is changed
    ///
    /// # Examples
    ///
    /// ```
    /// # use actify::HashMapHandle;
    /// # use std::collections::HashMap;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let mut handle = HashMapHandle::new(HashMap::new());
    /// handle.insert("test", 10).await;
    /// let res = handle.get_key("test").await;
    /// assert_eq!(res, Some(10));
    /// # }
    /// ```
    fn get_key(&self, key: K) -> Option<V> {
        self.get(&key).cloned()
    }

    /// Inserts a key-value pair into the map.
    /// If the map did not have this key present, [`None`] is returned.
    /// If the map did have this key present, the value is updated, and the old value is returned.
    /// In that case the key is not updated.
    ///
    /// # Examples
    ///
    /// ```
    /// # use actify::HashMapHandle;
    /// # use std::collections::HashMap;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let mut handle = HashMapHandle::new(HashMap::new());
    /// let res = handle.insert("test", 10).await;
    /// assert_eq!(res, None);
    ///
    /// let old_value = handle.insert("test", 20).await;
    /// assert_eq!(old_value, Some(10));
    /// # }
    /// ```
    fn insert(&mut self, key: K, val: V) -> Option<V> {
        self.insert(key, val)
    }

    /// Removes a key from the map, returning the value at the key if the key was previously in the map.
    /// Equivalent to [`HashMap::remove`](std::collections::HashMap::remove).
    ///
    /// # Examples
    ///
    /// ```
    /// # use actify::HashMapHandle;
    /// # use std::collections::HashMap;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let mut handle = HashMapHandle::new(HashMap::new());
    /// handle.insert("test", 10).await;
    /// let res = handle.remove("test").await;
    /// assert_eq!(res, Some(10));
    ///
    /// let res = handle.remove("test").await;
    /// assert_eq!(res, None);
    /// # }
    /// ```
    fn remove(&mut self, key: K) -> Option<V> {
        self.remove(&key)
    }

    /// Clears the map, removing all key-value pairs.
    /// Equivalent to [`HashMap::clear`](std::collections::HashMap::clear).
    ///
    /// # Examples
    ///
    /// ```
    /// # use actify::HashMapHandle;
    /// # use std::collections::HashMap;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let mut handle = HashMapHandle::new(HashMap::new());
    /// handle.insert("test", 10).await;
    /// handle.clear().await;
    /// assert!(handle.is_empty().await);
    /// # }
    /// ```
    fn clear(&mut self) {
        self.clear()
    }

    /// Returns `true` if the map contains no elements.
    ///
    /// # Examples
    ///
    /// ```
    /// # use actify::HashMapHandle;
    /// # use std::collections::HashMap;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let mut handle = HashMapHandle::new(HashMap::<&str, i32>::new());
    /// assert!(handle.is_empty().await);
    /// # }
    /// ```
    fn is_empty(&self) -> bool {
        self.is_empty()
    }

    /// Returns a `Vec` of all keys in the map.
    /// Equivalent to [`HashMap::keys`](std::collections::HashMap::keys), but collected into a `Vec`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use actify::HashMapHandle;
    /// # use std::collections::HashMap;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let mut handle = HashMapHandle::new(HashMap::new());
    /// handle.insert("a", 1).await;
    /// handle.insert("b", 2).await;
    /// let mut keys = handle.keys().await;
    /// keys.sort();
    /// assert_eq!(keys, vec!["a", "b"]);
    /// # }
    /// ```
    fn keys(&self) -> Vec<K> {
        self.keys().cloned().collect()
    }

    /// Returns a `Vec` of all values in the map.
    /// Equivalent to [`HashMap::values`](std::collections::HashMap::values), but collected into a `Vec`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use actify::HashMapHandle;
    /// # use std::collections::HashMap;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let mut handle = HashMapHandle::new(HashMap::new());
    /// handle.insert("a", 1).await;
    /// handle.insert("b", 2).await;
    /// let mut values = handle.values().await;
    /// values.sort();
    /// assert_eq!(values, vec![1, 2]);
    /// # }
    /// ```
    fn values(&self) -> Vec<V> {
        self.values().cloned().collect()
    }

    /// Returns the number of elements in the map.
    ///
    /// # Examples
    ///
    /// ```
    /// # use actify::HashMapHandle;
    /// # use std::collections::HashMap;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let mut handle = HashMapHandle::new(HashMap::new());
    /// handle.insert("a", 1).await;
    /// handle.insert("b", 2).await;
    /// assert_eq!(handle.len().await, 2);
    /// # }
    /// ```
    fn len(&self) -> usize {
        self.len()
    }

    /// Returns `true` if the map contains a value for the specified key.
    ///
    /// # Examples
    ///
    /// ```
    /// # use actify::HashMapHandle;
    /// # use std::collections::HashMap;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let mut handle = HashMapHandle::new(HashMap::new());
    /// handle.insert("a", 1).await;
    /// assert!(handle.contains_key("a").await);
    /// assert!(!handle.contains_key("b").await);
    /// # }
    /// ```
    fn contains_key(&self, key: K) -> bool {
        self.contains_key(&key)
    }

    /// Removes all key-value pairs from the map and returns them as a `Vec`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use actify::HashMapHandle;
    /// # use std::collections::HashMap;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let mut handle = HashMapHandle::new(HashMap::new());
    /// handle.insert("a", 1).await;
    /// let items = handle.drain().await;
    /// assert_eq!(items, vec![("a", 1)]);
    /// assert!(handle.is_empty().await);
    /// # }
    /// ```
    fn drain(&mut self) -> Vec<(K, V)> {
        self.drain().collect()
    }

    /// Extends the map with the contents of the given `Vec` of key-value pairs.
    ///
    /// # Examples
    ///
    /// ```
    /// # use actify::HashMapHandle;
    /// # use std::collections::HashMap;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let mut handle = HashMapHandle::new(HashMap::new());
    /// handle.extend(vec![("a", 1), ("b", 2)]).await;
    /// assert_eq!(handle.len().await, 2);
    /// # }
    /// ```
    fn extend(&mut self, items: Vec<(K, V)>) {
        <Self as Extend<(K, V)>>::extend(self, items)
    }

    /// Removes the entry for `key` and returns both halves of it, or `None` if the
    /// map holds no such key.
    ///
    /// Where [`remove`](crate::HashMapHandle::remove) returns the value alone, this also hands
    /// back the key the map was storing, which can carry more than the key looked
    /// up with.
    ///
    /// # Examples
    ///
    /// ```
    /// # use actify::HashMapHandle;
    /// # use std::collections::HashMap;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let mut handle = HashMapHandle::new(HashMap::from([("a", 1)]));
    /// assert_eq!(handle.remove_entry("a").await, Some(("a", 1)));
    /// assert_eq!(handle.remove_entry("a").await, None);
    /// # }
    /// ```
    fn remove_entry(&mut self, key: K) -> Option<(K, V)> {
        self.remove_entry(&key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> HashMapHandle<String, i32> {
        HashMapHandle::new(HashMap::from([("a".to_string(), 1), ("b".to_string(), 2)]))
    }

    #[tokio::test]
    async fn test_drain_returns_every_pair_and_empties_the_map() {
        let mut handle = map();

        let mut drained = handle.drain().await;
        drained.sort();

        assert_eq!(drained, vec![("a".to_string(), 1), ("b".to_string(), 2)]);
        assert!(handle.is_empty().await);
    }

    #[tokio::test]
    async fn test_extend_adds_and_overwrites() {
        let mut handle = map();

        handle
            .extend(vec![("b".to_string(), 20), ("c".to_string(), 3)])
            .await;

        assert_eq!(handle.len().await, 3);
        assert_eq!(handle.get_key("b".to_string()).await, Some(20));
        assert_eq!(handle.get_key("c".to_string()).await, Some(3));
    }

    /// `Eq` and `Hash` read only the id, so two keys can be equal while carrying
    /// different labels. Without that, nothing shows which key `remove_entry`
    /// hands back.
    #[derive(Clone, Debug)]
    struct Tagged {
        id: i32,
        label: &'static str,
    }

    impl PartialEq for Tagged {
        fn eq(&self, other: &Self) -> bool {
            self.id == other.id
        }
    }

    impl Eq for Tagged {}

    impl Hash for Tagged {
        fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
            self.id.hash(state);
        }
    }

    #[tokio::test]
    async fn test_remove_entry_returns_the_stored_key() {
        let stored = Tagged {
            id: 1,
            label: "stored",
        };
        let lookup = Tagged {
            id: 1,
            label: "lookup",
        };
        let mut handle = HashMapHandle::new(HashMap::from([(stored, 7)]));

        let (key, value) = handle.remove_entry(lookup.clone()).await.unwrap();
        assert_eq!(key.label, "stored");
        assert_eq!(value, 7);
        assert!(handle.is_empty().await);

        assert!(handle.remove_entry(lookup).await.is_none());
    }
}

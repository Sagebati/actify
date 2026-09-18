use std::any::Any;
use std::any::type_name;
use std::fmt::{self, Debug};
use std::marker::PhantomData;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, watch};

use super::read_handle::ReadHandle;
use crate::actor::{Actor, ActorExit, ActorMethod, ExitState, Job, serve};

pub(crate) const CHANNEL_SIZE: usize = 100;
const DOWNCAST_FAIL: &str =
    "Actify Macro error: failed to downcast arguments to their concrete type";

/// Defines the view an actor exposes: the type `V` that [`Handle::get`]
/// returns.
///
/// A blanket implementation is provided for [`Clone`] types, whose view is
/// themselves. Implement this trait to expose a different type `V` from your
/// actor type `T`, which allows:
///
/// - Non-Clone types to be read
/// - Clone types to expose a lightweight summary instead of the full value
///
/// [`Handle::with`] reads the actor type itself either way.
///
/// # Examples
///
/// ```
/// use actify::ToView;
///
/// struct HeavyState {
///     data: Vec<u8>,
///     summary: String,
/// }
///
/// #[derive(Clone, Debug)]
/// struct Summary(String);
///
/// impl ToView<Summary> for HeavyState {
///     fn to_view(&self) -> Summary {
///         Summary(self.summary.clone())
///     }
/// }
/// ```
pub trait ToView<V> {
    /// Produces the view of the actor.
    ///
    /// Runs on the actor task, on every [`Handle::get`].
    fn to_view(&self) -> V;
}

impl<T: Clone> ToView<T> for T {
    fn to_view(&self) -> T {
        self.clone()
    }
}

/// A clonable handle that can be used to remotely execute a closure on the corresponding [`Actor`].
///
/// Handles are the primary way to interact with actors. Cloning a handle shares
/// access to the same actor across tasks. For read-only access, see
/// [`ReadHandle`].
///
/// The second type parameter `V` is the view the handle exposes: what
/// [`Handle::get`] returns. By default `V = T`, so a read is a clone of the
/// actor itself. To expose a different type, implement [`ToView<V>`] and
/// specify `V` explicitly (e.g. `Handle::<MyType, Summary>::new(val)`).
/// [`Handle::with`] always reads the actor type.
pub struct Handle<T, V = T> {
    channels: Arc<Channels<T>>,
    view: PhantomData<fn() -> V>,
}

/// The channel endpoints every clone of a [`Handle`] shares. One `Arc` holds
/// both, so a handle is a single pointer and cloning it is one reference count
/// increment.
struct Channels<T> {
    tx: mpsc::Sender<Job<T>>,
    exit_rx: watch::Receiver<ExitState>,
}

impl<T, V> Clone for Handle<T, V> {
    fn clone(&self) -> Self {
        Handle {
            channels: Arc::clone(&self.channels),
            view: PhantomData,
        }
    }
}

impl<T, V> Debug for Handle<T, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let actor = type_name::<T>();
        let view = type_name::<V>();
        if actor == view {
            write!(f, "Handle<{actor}>")
        } else {
            write!(f, "Handle<{actor}, {view}>")
        }
    }
}

impl<T: Default + Clone + Send + Sync + 'static> Default for Handle<T> {
    #[track_caller]
    fn default() -> Self {
        Handle::new(T::default())
    }
}

impl<T, V> Handle<T, V>
where
    T: ToView<V> + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    /// Creates a new [`Handle`] and spawns the corresponding [`Actor`].
    ///
    /// For `Clone` types, `V` defaults to `T`: a read is a clone of the actor
    /// itself and you can simply write `Handle::new(val)`.
    ///
    /// For non-Clone types (or to read a lightweight summary), implement
    /// [`ToView<V>`] and specify `V` explicitly:
    ///
    /// ```
    /// # use actify::{Handle, ToView};
    /// # #[tokio::main]
    /// # async fn main() {
    /// #[derive(Clone, Debug, PartialEq)]
    /// struct Size(usize);
    ///
    /// impl ToView<Size> for Vec<u8> {
    ///     fn to_view(&self) -> Size { Size(self.len()) }
    /// }
    ///
    /// let handle: Handle<Vec<u8>, Size> = Handle::new(vec![1, 2, 3]);
    /// assert_eq!(handle.get().await, Size(3));
    /// # }
    /// ```
    #[track_caller]
    pub fn new(val: T) -> Handle<T, V> {
        let (tx, rx) = mpsc::channel(CHANNEL_SIZE);
        let (exit_tx, exit_rx) = watch::channel(None);
        let actor = Actor::new(val, std::panic::Location::caller());
        tokio::spawn(serve(rx, actor, exit_tx));
        Handle {
            channels: Arc::new(Channels { tx, exit_rx }),
            view: PhantomData,
        }
    }

    /// Returns the actor's current view.
    ///
    /// For a `Clone` actor type without a [`ToView`] implementation of its own,
    /// `V` is the actor type and this is a clone of the whole value. Otherwise it
    /// is whatever [`ToView::to_view`] produces, and [`Handle::with`] reads the
    /// actor value itself.
    ///
    /// # Examples
    ///
    /// ```
    /// # use actify::Handle;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let handle = Handle::new(1);
    /// let result = handle.get().await;
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
        self.run((), |s, _| s.inner.to_view()).await
    }
}

impl<T, V> Handle<T, V> {
    /// Returns a [`ReadHandle`] that provides read-only access to this actor.
    pub fn read_handle(&self) -> ReadHandle<T, V> {
        ReadHandle::new(self.clone())
    }

    /// Waits until the actor stops serving jobs, and reports why.
    ///
    /// Returns immediately if it has already stopped.
    async fn wait_for_exit(&self) -> ActorExit {
        let mut exit_rx = self.channels.exit_rx.clone();
        loop {
            if let Some(exit) = *exit_rx.borrow_and_update() {
                return exit;
            }

            // The sender is dropped without a value only if the actor task was
            // discarded before it ever ran, which still means it is gone.
            if exit_rx.changed().await.is_err() {
                return ActorExit::Stopped;
            }
        }
    }
}

impl<T: Send + Sync + 'static, V> Handle<T, V> {
    /// Returns how many more jobs can be queued before a call has to wait.
    ///
    /// Falls as calls queue up and rises again as the actor serves them, so it
    /// is the way to observe the actor falling behind.
    pub fn remaining_capacity(&self) -> usize {
        self.channels.tx.capacity()
    }

    #[doc(hidden)]
    pub async fn __send_job(
        &self,
        call: ActorMethod<T>,
        args: Box<dyn Any + Send>,
    ) -> Box<dyn Any + Send> {
        let (respond_to, get_result) = oneshot::channel();
        let job = Job {
            call,
            args,
            respond_to,
        };
        if self.channels.tx.send(job).await.is_ok() {
            if let Ok(res) = get_result.await {
                return res;
            }
        }
        self.report_actor_gone().await
    }

    /// Panics with the reason the actor stopped serving jobs.
    ///
    /// The exit signal may not have been written yet when the channel first
    /// reports its failure, so this waits for it rather than guessing from
    /// scheduling order.
    async fn report_actor_gone(&self) -> ! {
        if self.wait_for_exit().await == ActorExit::Panicked {
            panic!("A panic occurred in the Actor of type {}", type_name::<T>());
        }
        panic!("Actor of type {} is no longer running", type_name::<T>());
    }

    /// Sends a closure to the actor, handling all boxing/unboxing internally.
    async fn run<F, A, R>(&self, args: A, f: F) -> R
    where
        F: FnOnce(&mut Actor<T>, A) -> R + Send + 'static,
        A: Send + 'static,
        R: Send + 'static,
    {
        let res = self
            .__send_job(
                Box::new(move |s: &mut Actor<T>, boxed_args: Box<dyn Any + Send>| {
                    Box::pin(async move {
                        let args = *boxed_args.downcast::<A>().expect(DOWNCAST_FAIL);
                        Box::new(f(s, args)) as Box<dyn Any + Send>
                    })
                }),
                Box::new(args),
            )
            .await;
        *res.downcast::<R>().expect(DOWNCAST_FAIL)
    }

    /// Overwrites the inner value of the actor with the new value.
    ///
    /// # Examples
    ///
    /// ```
    /// # use actify::Handle;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let handle = Handle::new(None);
    /// handle.set(Some(1)).await;
    /// assert_eq!(handle.get().await, Some(1));
    /// # }
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the actor has stopped, either because one of its methods
    /// panicked or because its runtime shut down. See [Actor lifetime and
    /// panics](crate#actor-lifetime-and-panics).
    pub async fn set(&self, val: T) {
        self.run(val, |s, val| s.inner = val).await
    }

    /// Runs a read-only closure on the actor's value and returns the result.
    ///
    /// This reads parts of the actor state without cloning the entire value,
    /// and works with non-Clone types.
    ///
    /// # Examples
    ///
    /// ```
    /// # use actify::Handle;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let handle = Handle::new(vec![1, 2, 3]);
    ///
    /// let len = handle.with(|v| v.len()).await;
    /// assert_eq!(len, 3);
    ///
    /// let first = handle.with(|v| v.first().copied()).await;
    /// assert_eq!(first, Some(1));
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
        self.run(f, |s, f| f(&s.inner)).await
    }

    /// Runs a closure on the actor's value mutably and returns the result.
    ///
    /// This performs an atomic read-modify-return without a dedicated
    /// `#[actify]` method.
    ///
    /// [`Handle::with`] is the read-only counterpart.
    ///
    /// # Examples
    ///
    /// ```
    /// # use actify::Handle;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let handle = Handle::new(vec![1, 2, 3]);
    ///
    /// // Mutate and return a result in one atomic operation
    /// let popped = handle.with_mut(|v| v.pop()).await;
    /// assert_eq!(popped, Some(3));
    /// assert_eq!(handle.get().await, vec![1, 2]);
    /// # }
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the actor has stopped, either because one of its methods
    /// panicked or because its runtime shut down. See [Actor lifetime and
    /// panics](crate#actor-lifetime-and-panics).
    pub async fn with_mut<R, F>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R + Send + 'static,
        R: Send + 'static,
    {
        self.run(f, |s, f| f(&mut s.inner)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A handle travels by value: it is held in structs and captured by
    /// futures, so its size is paid on every copy.
    #[test]
    fn test_handle_is_pointer_sized() {
        assert_eq!(size_of::<Handle<u8>>(), size_of::<usize>());
    }

    /// A panicking actor method must surface as a panic naming that cause, not
    /// as the generic message used when the actor merely stopped.
    ///
    /// The caller's panic is raised by the handle: the actor's own payload
    /// unwinds the actor task and is not forwarded, which is why the assertion
    /// checks for the handle's message and against the actor's.
    #[tokio::test]
    async fn test_actor_panic_is_reported_as_a_panic() {
        let handle = Handle::new(PanicStruct {});
        let clone = handle.clone();

        let result = tokio::spawn(async move { clone.panic().await }).await;

        let message = panic_message(result.unwrap_err());
        assert_eq!(
            message,
            format!(
                "A panic occurred in the Actor of type {}",
                type_name::<PanicStruct>()
            )
        );
        assert!(
            !message.contains(SYNC_PANIC_PAYLOAD),
            "the actor's own payload is not forwarded to the caller: {message}"
        );
    }

    /// The same for a panic inside an async method, which unwinds from a
    /// different point in the job's lifetime than a sync one.
    #[tokio::test]
    async fn test_async_actor_panic_is_reported_as_a_panic() {
        let handle = Handle::new(PanicStruct {});
        let clone = handle.clone();

        let result = tokio::spawn(async move { clone.panic_async().await }).await;

        let message = panic_message(result.unwrap_err());
        assert_eq!(
            message,
            format!(
                "A panic occurred in the Actor of type {}",
                type_name::<PanicStruct>()
            )
        );
        assert!(
            !message.contains(ASYNC_PANIC_PAYLOAD),
            "the actor's own payload is not forwarded to the caller: {message}"
        );
    }

    /// One clone's call kills the shared actor, so every other clone is left
    /// holding a handle to a dead actor - and learns it was a panic that
    /// killed it, not an ordinary shutdown.
    #[tokio::test]
    async fn test_actor_panic_is_reported_to_other_clones() {
        let handle = Handle::new(PanicStruct {});
        let victim = handle.clone();
        let bystander = handle.clone();

        // The same call succeeds while the actor is alive, so the failure
        // below can only come from the actor being gone
        assert_eq!(handle.innocent().await, 7);

        let _ = tokio::spawn(async move { victim.panic().await }).await;

        let result = tokio::spawn(async move { bystander.innocent().await }).await;

        let message = panic_message(result.unwrap_err());
        assert_eq!(
            message,
            format!(
                "A panic occurred in the Actor of type {}",
                type_name::<PanicStruct>()
            )
        );
    }

    /// A handle outliving its runtime is a different failure from a panicking
    /// method, and saying "a panic occurred" there sends readers hunting for a
    /// panic that never happened.
    #[test]
    fn test_orphaned_handle_reports_a_stopped_actor() {
        let actor_rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let handle = actor_rt.block_on(async {
            let handle = Handle::new(0i32);
            handle.set(42).await;
            assert_eq!(handle.get().await, 42); // The actor served jobs normally
            handle
        });

        drop(actor_rt); // Cancels the actor task without unwinding it

        let caller_rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        caller_rt.block_on(async {
            let orphaned = handle.clone();
            let result = tokio::spawn(async move { orphaned.set(99).await }).await;

            let message = panic_message(result.unwrap_err());
            assert!(
                message.contains("no longer running"),
                "expected a stopped-actor message, got: {message}"
            );
        });
    }

    mod views {
        use super::*;

        fn big() -> Handle<BigState, usize> {
            Handle::new(BigState {
                data: vec![1, 2, 3],
                count: 7,
            })
        }

        #[tokio::test]
        async fn test_get_returns_the_view_not_the_state() {
            assert_eq!(big().get().await, 7);
        }

        #[tokio::test]
        async fn test_a_non_clone_actor_can_be_read() {
            let handle: Handle<NonCloneActor, i32> = Handle::new(NonCloneActor { value: 1 });

            assert_eq!(handle.get().await, 1);
            assert_eq!(handle.read_handle().get().await, 1);
        }

        #[tokio::test]
        async fn test_the_state_stays_reachable_through_with() {
            let handle = big();

            assert_eq!(handle.with(|state| state.data.clone()).await, vec![1, 2, 3]);
        }
    }

    fn panic_message(error: tokio::task::JoinError) -> String {
        let panic = error.into_panic();
        panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
            .expect("panic payload was neither String nor &str")
    }

    /// A caller that stops waiting must not stop the actor. Wrapping a call
    /// in a timeout or a select drops the future, which drops the response
    /// channel while the job is still queued or running.
    #[tokio::test(start_paused = true)]
    async fn test_abandoned_call_does_not_stop_the_actor() {
        let handle = Handle::new(SlowActor {});

        let slow = handle.clone();
        let abandoned =
            tokio::time::timeout(std::time::Duration::from_millis(10), slow.linger()).await;
        assert!(abandoned.is_err(), "the call should have timed out");

        // The actor finishes the abandoned job with nobody listening, and
        // still serves the next caller
        assert_eq!(handle.quick().await, 7);
    }

    #[derive(Debug, Clone)]
    struct SlowActor {}

    #[actify_macros::actify]
    impl SlowActor {
        async fn linger(&self) {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        fn quick(&self) -> i32 {
            7
        }
    }

    /// Callers past the channel capacity wait for a slot instead of failing,
    /// so every job is served. The sleep in each job holds the actor long
    /// enough for all callers to pile up on the bounded channel.
    #[tokio::test(start_paused = true)]
    async fn test_callers_wait_when_the_job_channel_is_full() {
        let handle = Handle::new(Ledger { seen: Vec::new() });

        let mut calls = tokio::task::JoinSet::new();
        for i in 0..2 * CHANNEL_SIZE {
            let handle = handle.clone();
            calls.spawn(async move { handle.record(i).await });
        }
        while calls.join_next().await.is_some() {}

        let mut seen = handle.with(|ledger| ledger.seen.clone()).await;
        seen.sort();
        assert_eq!(seen, (0..2 * CHANNEL_SIZE).collect::<Vec<_>>());
    }

    #[derive(Debug, Clone)]
    struct Ledger {
        seen: Vec<usize>,
    }

    #[actify_macros::actify]
    impl Ledger {
        async fn record(&mut self, i: usize) {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            self.seen.push(i);
        }
    }

    /// Payloads distinctive enough that a test can tell whose panic it caught:
    /// the actor's own, or the one the handle raises on the caller's behalf.
    const SYNC_PANIC_PAYLOAD: &str = "sync actor method blew up";
    const ASYNC_PANIC_PAYLOAD: &str = "async actor method blew up";

    #[derive(Debug, Clone)]
    struct PanicStruct {}

    #[actify_macros::actify]
    impl PanicStruct {
        fn panic(&self) {
            panic!("{SYNC_PANIC_PAYLOAD}")
        }

        async fn panic_async(&self) {
            panic!("{ASYNC_PANIC_PAYLOAD}")
        }

        /// A method that cannot fail on its own, so any panic it raises must
        /// have come from the actor being gone.
        fn innocent(&self) -> i32 {
            7
        }
    }

    #[derive(Debug)]
    struct NonCloneActor {
        value: i32,
    }

    #[actify_macros::actify]
    impl NonCloneActor {
        fn get_value(&self) -> i32 {
            self.value
        }

        fn set_value(&mut self, val: i32) {
            self.value = val;
        }
    }

    impl ToView<i32> for NonCloneActor {
        fn to_view(&self) -> i32 {
            self.value
        }
    }

    #[tokio::test]
    async fn test_non_clone_actor() {
        let handle: Handle<NonCloneActor, i32> = Handle::new(NonCloneActor { value: 42 });
        assert_eq!(handle.get_value().await, 42);

        handle.set_value(100).await;
        assert_eq!(handle.get_value().await, 100);

        let handle2 = handle.clone();
        assert_eq!(handle2.get_value().await, 100);
    }

    /// A non-Clone actor is read through its view, which `set` on the actor
    /// type itself must also keep current.
    #[tokio::test]
    async fn test_non_clone_actor_is_read_through_its_view() {
        let handle: Handle<NonCloneActor, i32> = Handle::new(NonCloneActor { value: 42 });

        handle.set_value(100).await;
        assert_eq!(handle.get().await, 100);

        handle.set(NonCloneActor { value: 45 }).await;
        assert_eq!(handle.get().await, 45);
    }

    #[derive(Clone, Debug, PartialEq)]
    struct BigState {
        data: Vec<u8>,
        count: usize,
    }

    impl ToView<usize> for BigState {
        fn to_view(&self) -> usize {
            self.count
        }
    }

    /// `Handle<BigState, usize>` and `Handle<BigState>` used to print the same
    /// thing, so the view a handle exposes was invisible in a log line.
    #[tokio::test]
    async fn test_debug_names_the_view_only_when_it_differs() {
        let plain: Handle<i32> = Handle::new(1);
        assert_eq!(format!("{plain:?}"), "Handle<i32>");

        let viewed: Handle<BigState, usize> = Handle::new(BigState {
            data: vec![1],
            count: 1,
        });
        assert_eq!(
            format!("{viewed:?}"),
            format!("Handle<{}, usize>", type_name::<BigState>())
        );
    }

    #[tokio::test]
    async fn test_clone_actor_with_custom_view() {
        let handle: Handle<BigState, usize> = Handle::new(BigState {
            data: vec![1, 2, 3],
            count: 3,
        });

        assert_eq!(handle.get().await, 3);

        let updated = BigState {
            data: vec![1, 2, 3, 4],
            count: 4,
        };
        handle.set(updated.clone()).await;

        assert_eq!(handle.get().await, 4);
        assert_eq!(handle.with(|state| state.clone()).await, updated);
    }
}

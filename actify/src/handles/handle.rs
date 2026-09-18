use futures_channel::oneshot;
use std::any::Any;
use std::any::type_name;
use std::fmt::{self, Debug};
use std::marker::PhantomData;
use std::sync::Arc;

use super::builder::HandleBuilder;
use super::read_handle::ReadHandle;
use crate::actor::{Actor, ActorMethod, ClosureJob, reply};
use crate::channel::JobSender;
use crate::message::{Builtin, Job};

/// Panics because the actor is gone, whether it stopped or a method of it
/// panicked. Which of the two it was is on the actor's own exit event, at
/// ERROR level for a panic, because only the actor task can tell them apart.
fn report_actor_gone<T>() -> ! {
    panic!("Actor of type {} is no longer running", type_name::<T>());
}
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
pub struct Handle<T, V = T, M = Job<T, V>, S = DefaultSender<M>> {
    // The `Arc` is what makes a handle one pointer wide and what stops the
    // actor: the last handle to drop drops the only sending half with it.
    sender: Arc<S>,
    actor: Marker<T, V, M>,
}

/// Names the types a handle is for without holding one of them, and without
/// borrowing their auto traits: a handle is `Send` and `Sync` on the strength
/// of its channel alone.
type Marker<T, V, M> = PhantomData<fn() -> (T, V, M)>;

/// The channel a handle uses when the caller supplies none: an unbounded
/// [`futures_channel`] queue, so a call never waits to be queued.
pub type DefaultSender<M> = futures_channel::mpsc::UnboundedSender<M>;

/// The receiving half of [`DefaultSender`], which the actor future reads.
pub type DefaultReceiver<M> = futures_channel::mpsc::UnboundedReceiver<M>;

impl<T, V, M, S> Clone for Handle<T, V, M, S> {
    fn clone(&self) -> Self {
        Handle {
            sender: Arc::clone(&self.sender),
            actor: PhantomData,
        }
    }
}

impl<T, V, M, S> Debug for Handle<T, V, M, S> {
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

/// Spawns on Tokio, as [`Handle::new`] does.
#[cfg(feature = "tokio")]
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
    /// Creates a new [`Handle`] and spawns the corresponding [`Actor`] on
    /// Tokio.
    ///
    /// The Tokio spelling of [`Handle::builder`] followed by a
    /// `tokio::spawn`, behind the default `tokio` feature. Turn that feature
    /// off and the caller spawns the actor future itself, on any executor.
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
    #[cfg(feature = "tokio")]
    #[track_caller]
    pub fn new(val: T) -> Handle<T, V> {
        let (handle, actor) = Handle::builder(val).build();
        tokio::spawn(actor);
        handle
    }

    /// Starts building a [`Handle`] whose actor future the caller spawns.
    ///
    /// Where [`Handle::new`] spawns on Tokio, this hands the future back:
    ///
    /// ```
    /// # use actify::Handle;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let (handle, actor) = Handle::builder(0).build();
    /// tokio::spawn(actor);
    ///
    /// assert_eq!(handle.get().await, 0);
    /// # }
    /// ```
    #[track_caller]
    pub fn builder(val: T) -> HandleBuilder<T, V> {
        HandleBuilder::new(val)
    }
}

impl<T, V, M, S> Handle<T, V, M, S>
where
    T: ToView<V> + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
    M: From<Builtin<T, V>> + Send + 'static,
    S: JobSender<M>,
{
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
        let (reply, get_result) = reply();
        self.__call(Builtin::Get(reply).into(), get_result).await
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
        let (reply, get_result) = reply();
        self.__call(Builtin::Set(val, reply).into(), get_result)
            .await
    }
}

impl<T, V, M, S> Handle<T, V, M, S> {
    pub(super) fn from_sender(sender: S) -> Self {
        Handle {
            sender: Arc::new(sender),
            actor: PhantomData,
        }
    }

    /// Returns a [`ReadHandle`] that provides read-only access to this actor.
    pub fn read_handle(&self) -> ReadHandle<T, V, M, S> {
        ReadHandle::new(self.clone())
    }
}

impl<T, V, M, S> Handle<T, V, M, S>
where
    T: 'static,
    M: Send + 'static,
    S: JobSender<M>,
{
    /// Queues one call and waits for its reply.
    ///
    /// The reply channel is the caller's to make, so that it carries the
    /// method's own return type rather than something erased.
    #[doc(hidden)]
    pub async fn __call<R>(&self, message: M, get_result: oneshot::Receiver<R>) -> R {
        if self.sender.send(message).await.is_ok() {
            if let Ok(res) = get_result.await {
                return res;
            }
        }
        report_actor_gone::<T>()
    }
}

impl<T, V, M, S> Handle<T, V, M, S>
where
    T: Send + Sync + 'static,
    M: From<ClosureJob<T>> + Send + 'static,
    S: JobSender<M>,
{
    #[doc(hidden)]
    pub async fn __send_job(
        &self,
        call: ActorMethod<T>,
        args: Box<dyn Any + Send + Sync>,
    ) -> Box<dyn Any + Send + Sync> {
        let (respond_to, get_result) = oneshot::channel();
        let job = ClosureJob {
            call,
            args,
            respond_to,
        };
        self.__call(job.into(), get_result).await
    }

    /// Sends a closure to the actor, handling all boxing/unboxing internally.
    async fn run<F, A, R>(&self, args: A, f: F) -> R
    where
        F: FnOnce(&mut Actor<T>, A) -> R + Send + Sync + 'static,
        A: Send + Sync + 'static,
        R: Send + Sync + 'static,
    {
        let res = self
            .__send_job(
                Box::new(
                    move |s: &mut Actor<T>, boxed_args: Box<dyn Any + Send + Sync>| {
                        Box::pin(async move {
                            let args = *boxed_args.downcast::<A>().expect(DOWNCAST_FAIL);
                            Box::new(f(s, args)) as Box<dyn Any + Send + Sync>
                        })
                    },
                ),
                Box::new(args),
            )
            .await;
        *res.downcast::<R>().expect(DOWNCAST_FAIL)
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
        F: FnOnce(&T) -> R + Send + Sync + 'static,
        R: Send + Sync + 'static,
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
        F: FnOnce(&mut T) -> R + Send + Sync + 'static,
        R: Send + Sync + 'static,
    {
        self.run(f, |s, f| f(&mut s.inner)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A built handle serves calls only once its actor future is spawned, and
    /// every call panics until then, so the two halves are useless apart.
    #[tokio::test]
    async fn test_a_built_actor_serves_once_the_caller_spawns_it() {
        let (handle, actor) = Handle::builder(1).build();

        let caller = handle.clone();
        let call = tokio::spawn(async move { caller.get().await });

        tokio::spawn(actor);

        assert_eq!(call.await.unwrap(), 1);
    }

    /// Dropping the actor future is the same failure as an actor that stopped:
    /// nothing will ever serve the handle.
    #[tokio::test]
    async fn test_dropping_the_actor_future_leaves_a_dead_handle() {
        let (handle, actor) = Handle::builder(1).build();
        drop(actor);

        let result = tokio::spawn(async move { handle.get().await }).await;

        assert!(panic_message(result.unwrap_err()).contains("no longer running"));
    }

    /// A handle travels by value: it is held in structs and captured by
    /// futures, so its size is paid on every copy.
    #[test]
    fn test_handle_is_pointer_sized() {
        assert_eq!(size_of::<Handle<u8>>(), size_of::<usize>());
    }

    /// A panicking actor method leaves the actor gone, which the next call
    /// surfaces as a panic.
    ///
    /// The caller's panic is raised by the handle: the actor's own payload
    /// unwinds the actor task and is not forwarded, which is why the assertion
    /// checks for the handle's message and against the actor's.
    #[tokio::test]
    async fn test_actor_panic_is_reported_as_a_panic() {
        let handle = PanicStructHandle::new(PanicStruct {});
        let clone = handle.clone();

        let result = tokio::spawn(async move { clone.panic().await }).await;

        let message = panic_message(result.unwrap_err());
        assert_eq!(
            message,
            format!(
                "Actor of type {} is no longer running",
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
        let handle = PanicStructHandle::new(PanicStruct {});
        let clone = handle.clone();

        let result = tokio::spawn(async move { clone.panic_async().await }).await;

        let message = panic_message(result.unwrap_err());
        assert_eq!(
            message,
            format!(
                "Actor of type {} is no longer running",
                type_name::<PanicStruct>()
            )
        );
        assert!(
            !message.contains(ASYNC_PANIC_PAYLOAD),
            "the actor's own payload is not forwarded to the caller: {message}"
        );
    }

    /// One clone's call kills the shared actor, so every other clone is left
    /// holding a handle to a dead actor.
    #[tokio::test]
    async fn test_actor_panic_is_reported_to_other_clones() {
        let handle = PanicStructHandle::new(PanicStruct {});
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
                "Actor of type {} is no longer running",
                type_name::<PanicStruct>()
            )
        );
    }

    /// A handle outliving its runtime holds an actor that was cancelled
    /// rather than one that panicked, and a call on it still reports that the
    /// actor is gone.
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
            let handle = NonCloneActorHandle::<i32>::new(NonCloneActor { value: 1 });

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
        let handle = SlowActorHandle::new(SlowActor {});

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

    /// A caller asks for backpressure by supplying a bounded channel. Callers
    /// past its capacity then wait for a slot instead of failing, so every job
    /// is still served. The sleep in each job holds the actor long enough for
    /// all of them to pile up.
    #[tokio::test(start_paused = true)]
    async fn test_callers_wait_when_a_bounded_job_channel_is_full() {
        const CAPACITY: usize = 4;
        let (handle, actor) = LedgerHandle::builder(Ledger { seen: Vec::new() })
            .channel(futures_channel::mpsc::channel(CAPACITY))
            .build();
        tokio::spawn(actor);
        let handle = LedgerHandle::from_handle(handle);

        let mut calls = tokio::task::JoinSet::new();
        for i in 0..8 * CAPACITY {
            let handle = handle.clone();
            calls.spawn(async move { handle.record(i).await });
        }
        while calls.join_next().await.is_some() {}

        let mut seen = handle.seen().await;
        seen.sort();
        assert_eq!(seen, (0..8 * CAPACITY).collect::<Vec<_>>());
    }

    /// The default channel is unbounded, so queueing a call never waits: the
    /// only thing a call awaits is the reply. A bounded channel this small
    /// would make the sends alone outlive the actor's first job.
    #[tokio::test(start_paused = true)]
    async fn test_the_default_channel_never_makes_a_caller_wait_to_queue() {
        let handle = LedgerHandle::new(Ledger { seen: Vec::new() });

        let mut calls = tokio::task::JoinSet::new();
        for i in 0..1000 {
            let handle = handle.clone();
            calls.spawn(async move { handle.record(i).await });
        }
        while calls.join_next().await.is_some() {}

        assert_eq!(handle.count().await, 1000);
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

        fn seen(&self) -> Vec<usize> {
            self.seen.clone()
        }

        fn count(&self) -> usize {
            self.seen.len()
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
        let handle = NonCloneActorHandle::<i32>::new(NonCloneActor { value: 42 });
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
        let handle = NonCloneActorHandle::<i32>::new(NonCloneActor { value: 42 });

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

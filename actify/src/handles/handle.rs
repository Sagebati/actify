use futures_channel::oneshot;
use std::any::type_name;
use std::fmt::{self, Debug};
use std::marker::PhantomData;

use super::read_handle::ReadHandle;
use crate::actor::reply;
use crate::channel::JobSender;
use crate::message::Builtin;

/// Panics because the actor is gone, whether it stopped or a method of it
/// panicked. Which of the two it was is on the actor's own exit event, at
/// ERROR level for a panic, because only the actor task can tell them apart.
fn report_actor_gone<T>() -> ! {
    panic!("Actor of type {} is no longer running", type_name::<T>());
}

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

/// The plumbing behind a generated handle: a shared sending half, and the
/// calls every actor answers.
///
/// `#[actify]` generates a handle of the actor's own that wraps this one, so a
/// caller never names it. Cloning it shares access to the same actor across
/// tasks.
///
/// The second type parameter `V` is the view the handle exposes: what
/// [`Handle::get`] returns. By default `V = T`, so a read is a clone of the
/// actor itself. To expose a different type, implement [`ToView<V>`] and name
/// it on the generated handle, as in `MyTypeHandle::<Summary>::new(val)`.
pub struct Handle<T, V, M, S = DefaultSender<M>> {
    // Owned outright, not shared: a sending half sent through by `&mut` is one
    // a bounded channel can refuse, and the channel counts handles rather than
    // calls. The last handle to drop drops the last sending half with it, which
    // is what stops the actor. A `futures_channel` sender is a niche-optimised
    // `Option<Arc<_>>`, so the handle is still one pointer wide.
    sender: S,
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

/// Cloning a handle clones its sending half, which is the only clone actify
/// makes. Every channel's sender is a cheap handle onto shared state, so it
/// costs a reference count rather than a copy of the queue.
impl<T, V, M, S: Clone> Clone for Handle<T, V, M, S> {
    fn clone(&self) -> Self {
        Handle {
            sender: self.sender.clone(),
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
    /// `V` is the actor type and this is a clone of the whole value. Otherwise
    /// it is whatever [`ToView::to_view`] produces.
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
    /// assert_eq!(handle.get().await, Counter(1));
    /// # }
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the actor has stopped, either because one of its methods
    /// panicked or because its runtime shut down. See [Actor lifetime and
    /// panics](crate#actor-lifetime-and-panics).
    pub async fn get(&mut self) -> V {
        let (reply, get_result) = reply();
        self.__call(Builtin::Get(reply).into(), get_result).await
    }

    /// Overwrites the inner value of the actor with the new value.
    ///
    /// # Examples
    ///
    /// ```
    /// # use actify::OptionHandle;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let mut handle = OptionHandle::new(None);
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
    pub async fn set(&mut self, val: T) {
        let (reply, get_result) = reply();
        self.__call(Builtin::Set(val, reply).into(), get_result)
            .await
    }
}

impl<T, V, M, S> Handle<T, V, M, S> {
    pub(super) fn from_sender(sender: S) -> Self {
        Handle {
            sender,
            actor: PhantomData,
        }
    }
}

impl<T, V, M, S: Clone> Handle<T, V, M, S> {
    /// Returns a [`ReadHandle`] that provides read-only access to this actor.
    ///
    /// It carries a sending half of its own, so it keeps the actor alive on its
    /// own and can be read from without the handle it came from.
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
    pub async fn __call<R>(&mut self, message: M, get_result: oneshot::Receiver<R>) -> R {
        if self.sender.send(message).await.is_ok() {
            if let Ok(res) = get_result.await {
                return res;
            }
        }
        report_actor_gone::<T>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// An actor with no methods of its own, so its handle carries the
    /// built-in calls and nothing else. An empty `#[actify]` block is how an
    /// actor asks for exactly that.
    #[derive(Clone, Debug)]
    struct Counter(i32);

    #[actify_macros::actify]
    impl Counter {}

    /// A built handle serves calls only once its actor future is spawned, and
    /// every call panics until then, so the two halves are useless apart.
    #[tokio::test]
    async fn test_a_built_actor_serves_once_the_caller_spawns_it() {
        let (handle, actor) = CounterHandle::builder(Counter(1)).build();

        let mut caller = handle.clone();
        let call = tokio::spawn(async move { caller.get().await });

        tokio::spawn(actor);

        assert_eq!(call.await.unwrap().0, 1);
    }

    /// Dropping the actor future is the same failure as an actor that stopped:
    /// nothing will ever serve the handle.
    #[tokio::test]
    async fn test_dropping_the_actor_future_leaves_a_dead_handle() {
        let (mut handle, actor) = CounterHandle::builder(Counter(1)).build();
        drop(actor);

        let result = tokio::spawn(async move { handle.get().await }).await;

        assert!(panic_message(result.unwrap_err()).contains("no longer running"));
    }

    /// A handle travels by value: it is held in structs and captured by
    /// futures, so its size is paid on every copy.
    #[test]
    fn test_handle_is_pointer_sized() {
        assert_eq!(size_of::<CounterHandle>(), size_of::<usize>());
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
        let mut clone = handle.clone();

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
        let mut clone = handle.clone();

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
        let mut handle = PanicStructHandle::new(PanicStruct {});
        let mut victim = handle.clone();
        let mut bystander = handle.clone();

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
            let mut handle = CounterHandle::new(Counter(0));
            handle.set(Counter(42)).await;
            assert_eq!(handle.get().await.0, 42); // The actor served jobs normally
            handle
        });

        drop(actor_rt); // Cancels the actor task without unwinding it

        let caller_rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        caller_rt.block_on(async {
            let mut orphaned = handle.clone();
            let result = tokio::spawn(async move { orphaned.set(Counter(99)).await }).await;

            let message = panic_message(result.unwrap_err());
            assert!(
                message.contains("no longer running"),
                "expected a stopped-actor message, got: {message}"
            );
        });
    }

    mod views {
        use super::*;

        fn big() -> BigStateHandle<usize> {
            BigStateHandle::<usize>::new(BigState {
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
            let mut handle = NonCloneActorHandle::<i32>::new(NonCloneActor { value: 1 });

            assert_eq!(handle.get().await, 1);
            assert_eq!(handle.read_handle().get().await, 1);
        }

        /// The view hides the state, so a method is what reaches past it.
        #[tokio::test]
        async fn test_the_state_stays_reachable_through_a_method() {
            let mut handle = big();

            assert_eq!(handle.data().await, vec![1, 2, 3]);
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
        let mut handle = SlowActorHandle::new(SlowActor {});

        let mut slow = handle.clone();
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

    /// Counts what a channel actually accepted, which is the only place
    /// backpressure is visible.
    ///
    /// Awaiting a call cannot show it: a call waits for its reply as well as
    /// for its slot, so a caller that got queued and one that did not look
    /// alike from the outside. `start_send` is where the queue says yes.
    struct Counted<M>(
        futures_channel::mpsc::Sender<M>,
        std::sync::Arc<AtomicUsize>,
    );

    // Derived `Clone` would demand `M: Clone`; a sender clones whatever it
    // carries.
    impl<M> Clone for Counted<M> {
        fn clone(&self) -> Self {
            Counted(self.0.clone(), std::sync::Arc::clone(&self.1))
        }
    }

    impl<M> futures_sink::Sink<M> for Counted<M> {
        type Error = futures_channel::mpsc::SendError;

        fn poll_ready(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::pin::Pin::new(&mut self.0).poll_ready(cx)
        }

        fn start_send(mut self: std::pin::Pin<&mut Self>, item: M) -> Result<(), Self::Error> {
            self.1.fetch_add(1, Ordering::Relaxed);
            std::pin::Pin::new(&mut self.0).start_send(item)
        }

        fn poll_flush(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::pin::Pin::new(&mut self.0).poll_flush(cx)
        }

        fn poll_close(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::pin::Pin::new(&mut self.0).poll_close(cx)
        }
    }

    /// A bounded channel's ceiling is its buffer plus one slot per handle,
    /// whatever number of calls those handles make.
    ///
    /// That is `futures_channel`'s contract - `buffer + num_senders` - and
    /// under `&mut` sending a sender is a handle, so the ceiling is something
    /// a program sets rather than something its call volume sets. Here eight
    /// slots hold back a thousand calls.
    ///
    /// What this does *not* prove is the thing that would be nice to prove.
    /// Concurrent callers need a handle each either way, so the old
    /// clone-per-send code bounded the queue at `buffer + calls in flight`,
    /// which for this shape is the same number. The difference between the two
    /// is not observable from here: it is that a single handle can no longer
    /// have two sends in flight, which the borrow checker now refuses, and
    /// that a send no longer allocates a sender. `compile_fail/
    /// two_calls_on_one_handle.rs` pins the first; `allocations.rs` pins the
    /// second.
    #[tokio::test]
    async fn test_a_bounded_job_channel_stops_accepting_while_the_actor_is_busy() {
        const CAPACITY: usize = 4;
        const HANDLES: usize = 4;
        const CALLS_EACH: usize = 250;
        const CALLS: usize = HANDLES * CALLS_EACH;

        let accepted = std::sync::Arc::new(AtomicUsize::new(0));
        let (tx, rx) = futures_channel::mpsc::channel(CAPACITY);
        let (handle, actor) = SlowActorHandle::builder(SlowActor {})
            .channel((Counted(tx, std::sync::Arc::clone(&accepted)), rx))
            .build();
        tokio::spawn(actor);

        // Every call after the first queues behind it, and the first sleeps a
        // second, so nothing drains while this is measured.
        let mut calls = tokio::task::JoinSet::new();
        for _ in 0..HANDLES {
            let mut handle = handle.clone();
            calls.spawn(async move {
                for _ in 0..CALLS_EACH {
                    handle.linger().await;
                }
            });
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let accepted = accepted.load(Ordering::Relaxed);
        assert!(
            accepted <= CAPACITY + HANDLES,
            "a channel bounded at {CAPACITY} with {HANDLES} handles accepted {accepted} \
             of {CALLS} calls while the actor was still on the first"
        );

        calls.abort_all();
    }

    /// Backpressure must not lose jobs: a caller that waits for a slot still
    /// gets served, so every job arrives.
    #[tokio::test(start_paused = true)]
    async fn test_callers_wait_rather_than_fail_when_the_channel_is_full() {
        const CAPACITY: usize = 4;
        let (mut handle, actor) = LedgerHandle::builder(Ledger { seen: Vec::new() })
            .channel(futures_channel::mpsc::channel(CAPACITY))
            .build();
        tokio::spawn(actor);

        let mut calls = tokio::task::JoinSet::new();
        for i in 0..8 * CAPACITY {
            let mut handle = handle.clone();
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
        let mut handle = LedgerHandle::new(Ledger { seen: Vec::new() });

        let mut calls = tokio::task::JoinSet::new();
        for i in 0..1000 {
            let mut handle = handle.clone();
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
        let mut handle = NonCloneActorHandle::<i32>::new(NonCloneActor { value: 42 });
        assert_eq!(handle.get_value().await, 42);

        handle.set_value(100).await;
        assert_eq!(handle.get_value().await, 100);

        let mut handle2 = handle.clone();
        assert_eq!(handle2.get_value().await, 100);
    }

    /// A non-Clone actor is read through its view, which `set` on the actor
    /// type itself must also keep current.
    #[tokio::test]
    async fn test_non_clone_actor_is_read_through_its_view() {
        let mut handle = NonCloneActorHandle::<i32>::new(NonCloneActor { value: 42 });

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

    /// Reading part of an actor without cloning all of it is what an
    /// `#[actify]` method is for, now that no handle takes a closure.
    #[actify_macros::actify]
    impl BigState {
        fn data(&self) -> Vec<u8> {
            self.data.clone()
        }

        fn state(&self) -> BigState {
            self.clone()
        }
    }

    impl ToView<usize> for BigState {
        fn to_view(&self) -> usize {
            self.count
        }
    }

    /// Two handles on the same actor that expose different views used to
    /// print the same thing, so the view was invisible in a log line.
    #[tokio::test]
    async fn test_debug_names_the_view_only_when_it_differs() {
        let plain = CounterHandle::new(Counter(1));
        assert_eq!(
            format!("{plain:?}"),
            format!("CounterHandle<{}>", type_name::<Counter>())
        );

        let viewed = BigStateHandle::<usize>::new(BigState {
            data: vec![1],
            count: 1,
        });
        assert_eq!(
            format!("{viewed:?}"),
            format!("BigStateHandle<{}, usize>", type_name::<BigState>())
        );
    }

    #[tokio::test]
    async fn test_clone_actor_with_custom_view() {
        let mut handle = BigStateHandle::<usize>::new(BigState {
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
        assert_eq!(handle.state().await, updated);
    }
}

//! The handle a caller holds to an async actor.

use std::any::type_name;
use std::fmt::{self, Debug};
use std::future::poll_fn;
use std::marker::PhantomData;
use std::pin::Pin;

use futures_channel::oneshot;
use futures_sink::Sink;

mod builder;

pub use builder::builder;
#[cfg(feature = "tokio")]
pub use builder::spawn;
pub use builder::{DefaultChannel, HandleBuilder};

/// Panics because the actor is gone, whether it stopped or a method of it
/// panicked. Which of the two it was is on the actor's own exit event, at
/// ERROR level for a panic, because only the actor task can tell them apart.
fn report_actor_gone<T>() -> ! {
    panic!("Actor of type {} is no longer running", type_name::<T>());
}

/// The plumbing behind a generated handle: the sending half of the actor's job
/// channel.
///
/// `#[actum]` generates a handle of the actor's own that wraps this one, so a
/// caller never names it. Cloning it clones the sending half, which shares
/// access to the same actor across tasks.
pub struct Handle<T, M, S = DefaultSender<M>> {
    // Owned outright, not shared: a sending half sent through by `&mut` is one
    // a bounded channel can refuse, and the channel counts handles rather than
    // calls. The last handle to drop drops the last sending half with it, which
    // is what stops the actor. A `futures_channel` sender is a niche-optimised
    // `Option<Arc<_>>`, so the handle is one pointer wide.
    sender: S,
    actor: Marker<T, M>,
}

/// Names the types a handle is for without holding one of them, and without
/// borrowing their auto traits: a handle is `Send` and `Sync` on the strength
/// of its channel alone.
type Marker<T, M> = PhantomData<fn() -> (T, M)>;

/// The channel a handle uses when the caller supplies none: an unbounded
/// [`futures_channel`] queue, so a call never waits to be queued.
pub type DefaultSender<M> = futures_channel::mpsc::UnboundedSender<M>;

/// The receiving half of [`DefaultSender`], which the actor future reads.
pub type DefaultReceiver<M> = futures_channel::mpsc::UnboundedReceiver<M>;

/// Cloning a handle clones its sending half, which is the only clone actum
/// makes. Every channel's sender is a cheap handle onto shared state, so it
/// costs a reference count rather than a copy of the queue.
impl<T, M, S: Clone> Clone for Handle<T, M, S> {
    fn clone(&self) -> Self {
        Handle {
            sender: self.sender.clone(),
            actor: PhantomData,
        }
    }
}

impl<T, M, S> Debug for Handle<T, M, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Handle<{}>", type_name::<T>())
    }
}

impl<T, M, S> Handle<T, M, S> {
    /// Only the builder makes one, and it is a child of this module.
    fn from_sender(sender: S) -> Self {
        Handle {
            sender,
            actor: PhantomData,
        }
    }
}

impl<T, M, S: Sink<M> + Unpin> Handle<T, M, S> {
    /// Queues one call and waits for its reply.
    ///
    /// The reply channel is the caller's to make, so that it carries the
    /// method's own return type rather than something erased.
    #[doc(hidden)]
    pub async fn __call<R>(&mut self, message: M, get_result: oneshot::Receiver<R>) -> R {
        if send(&mut self.sender, message).await.is_ok() {
            if let Ok(res) = get_result.await {
                return res;
            }
        }
        report_actor_gone::<T>()
    }
}

/// Sends one job through the sink: ready, send, flush.
///
/// This is `SinkExt::send` without a dependency on `futures-util` for one
/// method. The sink is the handle's own, sent through directly: nothing is
/// cloned, which is what lets a bounded sink refuse. A sender it has already
/// parked reports itself unready, where a fresh clone never would.
async fn send<M, S: Sink<M> + Unpin>(sink: &mut S, job: M) -> Result<(), S::Error> {
    poll_fn(|cx| Pin::new(&mut *sink).poll_ready(cx)).await?;
    Pin::new(&mut *sink).start_send(job)?;
    poll_fn(|cx| Pin::new(&mut *sink).poll_flush(cx)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug)]
    struct Counter(i32);

    #[actum_macros::actum]
    impl Counter {
        fn add(&mut self, value: i32) -> i32 {
            self.0 += value;
            self.0
        }

        fn value(&self) -> i32 {
            self.0
        }
    }

    /// A built handle serves calls only once its actor future is spawned, and
    /// every call panics until then, so the two halves are useless apart.
    #[tokio::test]
    async fn test_a_built_actor_serves_once_the_caller_spawns_it() {
        let (handle, actor) = CounterHandle::builder(Counter(1)).build();

        let mut caller = handle.clone();
        let call = tokio::spawn(async move { caller.value().await });

        tokio::spawn(actor);

        assert_eq!(call.await.unwrap(), 1);
    }

    /// Dropping the actor future is the same failure as an actor that stopped:
    /// nothing will ever serve the handle.
    #[tokio::test]
    async fn test_dropping_the_actor_future_leaves_a_dead_handle() {
        let (mut handle, actor) = CounterHandle::builder(Counter(1)).build();
        drop(actor);

        let result = tokio::spawn(async move { handle.value().await }).await;

        assert!(panic_message(result.unwrap_err()).contains("no longer running"));
    }

    /// A handle travels by value: it is held in structs and captured by
    /// futures, so its size is paid on every copy. It owns its sender, and a
    /// `futures_channel` sender is a niche-optimised `Option<Arc<_>>`.
    #[test]
    fn test_handle_is_pointer_sized() {
        assert_eq!(size_of::<CounterHandle>(), size_of::<usize>());
    }

    /// An actor type needs neither `Clone` nor `Debug`: nothing reads it whole
    /// and nothing prints it. Its handle is still both.
    #[tokio::test]
    async fn test_an_actor_needs_no_derives() {
        struct Plain {
            value: i32,
        }

        #[actum_macros::actum]
        impl Plain {
            fn value(&self) -> i32 {
                self.value
            }
        }

        let mut handle = PlainHandle::new(Plain { value: 42 });
        assert_eq!(handle.value().await, 42);

        let mut other = handle.clone();
        assert_eq!(other.value().await, 42);
        assert_eq!(
            format!("{handle:?}"),
            format!("PlainHandle<{}>", type_name::<Plain>())
        );
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
            assert_eq!(handle.add(42).await, 42); // The actor served jobs normally
            handle
        });

        drop(actor_rt); // Cancels the actor task without unwinding it

        let caller_rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        caller_rt.block_on(async {
            let mut orphaned = handle.clone();
            let result = tokio::spawn(async move { orphaned.add(1).await }).await;

            let message = panic_message(result.unwrap_err());
            assert!(
                message.contains("no longer running"),
                "expected a stopped-actor message, got: {message}"
            );
        });
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

    #[derive(Debug)]
    struct SlowActor {}

    #[actum_macros::actum]
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

    impl<M> Sink<M> for Counted<M> {
        type Error = futures_channel::mpsc::SendError;

        fn poll_ready(
            mut self: Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            Pin::new(&mut self.0).poll_ready(cx)
        }

        fn start_send(mut self: Pin<&mut Self>, item: M) -> Result<(), Self::Error> {
            self.1.fetch_add(1, Ordering::Relaxed);
            Pin::new(&mut self.0).start_send(item)
        }

        fn poll_flush(
            mut self: Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            Pin::new(&mut self.0).poll_flush(cx)
        }

        fn poll_close(
            mut self: Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            Pin::new(&mut self.0).poll_close(cx)
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
    /// Concurrent callers need a handle each, so the ceiling is also what a
    /// clone-per-send design would give this shape; what tells the two apart
    /// is that one handle cannot have two sends in flight, which
    /// `compile_fail/two_calls_on_one_handle.rs` pins, and that a send does
    /// not allocate a sender, which `allocations.rs` pins.
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

    #[derive(Debug)]
    struct Ledger {
        seen: Vec<usize>,
    }

    #[actum_macros::actum]
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

    #[derive(Debug)]
    struct PanicStruct {}

    #[actum_macros::actum]
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
}

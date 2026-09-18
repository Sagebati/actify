use futures_channel::oneshot;
use std::any::type_name;
use std::fmt::{self, Debug};
use std::future::Future;
use tracing::Instrument;

use std::panic::Location;
use std::sync::atomic::{AtomicU64, Ordering};

/// Names actor instances in spawn order, process-wide, which is what the
/// span's `actor_id` carries.
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// The internal actor wrapper that runs in a separate task.
///
/// You do not create this directly. It is spawned by [`Handle::new`](super::Handle::new).
/// The `inner` field holds the wrapped value.
#[doc(hidden)]
pub struct Actor<T> {
    pub inner: T,
    id: u64,
    spawned_at: &'static Location<'static>,
}

impl<T: Debug> Debug for Actor<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Actor").field("inner", &self.inner).finish()
    }
}

impl<T> Actor<T> {
    /// The id and the spawn site name the instance, and both go on the actor
    /// span.
    pub(crate) fn new(inner: T, spawned_at: &'static Location<'static>) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            inner,
            id,
            spawned_at,
        }
    }
}

/// The half of a call's reply channel that the actor holds.
///
/// It carries the concrete return type, so a result travels home as itself
/// rather than as a boxed `Any` the caller has to downcast. Answering it is
/// [`Actor::respond`].
pub struct Reply<R>(oneshot::Sender<R>);

impl<R> Debug for Reply<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Reply<{}>", type_name::<R>())
    }
}

/// Creates the two halves of one call's reply channel.
///
/// The actor keeps the [`Reply`], the caller awaits the receiver.
#[doc(hidden)]
pub fn reply<R>() -> (Reply<R>, oneshot::Receiver<R>) {
    let (respond_to, get_result) = oneshot::channel();
    (Reply(respond_to), get_result)
}

impl<T> Actor<T> {
    /// Answers one call.
    ///
    /// The caller is gone whenever it dropped the call before the actor got to
    /// it, which is worth a line because it means the work was wasted.
    pub fn respond<R>(&self, reply: Reply<R>, value: R) {
        if reply.0.send(value).is_err() {
            tracing::debug!(
                actor_type = type_name::<T>(),
                actor_id = self.id,
                "Actor failed to respond as the receiver is dropped"
            );
        }
    }
}

/// Why an actor stopped serving jobs. Reported on the actor's own exit event;
/// a caller only learns that the actor is gone, not which of the two it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActorExit {
    /// A method panicked, unwinding the actor task.
    Panicked,
    /// The actor task ended without unwinding, because every handle to it was
    /// dropped or the runtime shut down and cancelled it.
    Stopped,
}

/// Reports the exit reason when the actor task ends, however it ends.
///
/// `std::thread::panicking()` is true while a panic unwinds the task, which is
/// what separates a panicking actor method from a runtime shutdown or a
/// cancelled task - both of which drop the task without unwinding.
struct ExitGuard {
    actor_type: &'static str,
    actor_id: u64,
}

impl Drop for ExitGuard {
    fn drop(&mut self) {
        let reason = if std::thread::panicking() {
            ActorExit::Panicked
        } else {
            ActorExit::Stopped
        };
        // A panic also reaches the std panic hook, but that prints to stderr,
        // which a subscriber shipping structured logs never sees.
        if reason == ActorExit::Panicked {
            tracing::error!(
                actor_type = self.actor_type,
                actor_id = self.actor_id,
                reason = ?reason,
                "Actor stopped"
            );
        } else {
            tracing::debug!(
                actor_type = self.actor_type,
                actor_id = self.actor_id,
                reason = ?reason,
                "Actor stopped"
            );
        }
    }
}

/// Wraps an actor's loop in the `actor` span that names the instance, and in
/// the guard that reports why it stopped.
///
/// The span names the actor type, its spawn-order id and the call site it was
/// spawned from, so that instrumentation in actor methods nests under the
/// actor task.
///
/// The span is created before the future is spawned, which parents it to
/// whatever span is current where the handle is created. This is why it is a
/// manual wrapper rather than `#[tracing::instrument]`: on an async fn the
/// attribute creates its span at first poll, inside the spawned task, where
/// the creation context is gone.
/// `run` is the loop itself, which generated code supplies: it takes the
/// receiver and the actor by value, so its future borrows nothing from here
/// and needs neither a trait to name it nor a box to hold it.
#[doc(hidden)]
pub fn serve<T, R, F, Fut>(rx: R, actor: Actor<T>, run: F) -> impl Future<Output = ()> + Send
where
    T: Send + Sync + 'static,
    R: Send + 'static,
    F: FnOnce(R, Actor<T>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    let span = actor.span();
    let guard = actor.exit_guard();
    async move {
        let _guard = guard;
        run(rx, actor).await
    }
    .instrument(span)
}

impl<T: Send + Sync + 'static> Actor<T> {
    /// The span every call on this actor runs inside.
    fn span(&self) -> tracing::Span {
        tracing::info_span!(
            "actor",
            actor_type = type_name::<T>(),
            actor_id = self.id,
            spawned_at = %self.spawned_at,
        )
    }

    /// Reports why the actor stopped, whenever the loop's future is dropped.
    fn exit_guard(&self) -> ExitGuard {
        ExitGuard {
            actor_type: type_name::<T>(),
            actor_id: self.id,
        }
    }
}

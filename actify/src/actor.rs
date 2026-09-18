use futures_channel::oneshot;
use std::any::{Any, type_name};
use std::fmt::{self, Debug};
use std::future::Future;
use std::pin::Pin;
use tracing::Instrument;

use crate::channel::JobReceiver;

/// A boxed future, as returned by an actor method.
pub(crate) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

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

/// How one queued call runs against the actor that owns the state.
///
/// Implemented by [`Job`](crate::Job), and by the message type a caller
/// supplies in its place. It is monomorphised, so dispatching a call is a
/// direct call and not a jump through a vtable.
pub trait Dispatch<T>: Sized + Send + 'static {
    /// Runs this call on the actor, answering its caller before returning.
    fn dispatch(self, actor: &mut Actor<T>) -> impl Future<Output = ()> + Send;
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

/// A single call on an actor, sent from a handle and run once by [`serve`].
///
/// The lifetime is bound with `for<'a>` because the returned future borrows the
/// actor it was handed.
pub(crate) type ActorMethod<T> = Box<
    dyn for<'a> FnOnce(
            &'a mut Actor<T>,
            Box<dyn Any + Send + Sync>,
        ) -> BoxFuture<'a, Box<dyn Any + Send + Sync>>
        + Send
        + Sync,
>;

/// A call carried as a closure, which is how the handle's own closure-taking
/// methods reach the actor.
///
/// Every part is `Sync` as well as `Send`, so that a job is `Sync` and the
/// channel carrying it can be too. A channel's sending half holds the item it
/// is queueing, so a job that is not `Sync` would leave that half `!Sync`, and
/// a handle has to be shareable.
#[doc(hidden)]
pub struct ClosureJob<T> {
    pub(crate) call: ActorMethod<T>,
    pub(crate) args: Box<dyn Any + Send + Sync>,
    pub(crate) respond_to: oneshot::Sender<Box<dyn Any + Send + Sync>>,
}

impl<T> Debug for ClosureJob<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ClosureJob<{}>", type_name::<T>())
    }
}

impl<T: Send + Sync + 'static> Dispatch<T> for ClosureJob<T> {
    async fn dispatch(self, actor: &mut Actor<T>) {
        let res = (self.call)(actor, self.args).await;
        if self.respond_to.send(res).is_err() {
            tracing::debug!(
                actor_type = type_name::<T>(),
                actor_id = actor.id,
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

/// Serves jobs inside an `actor` span that names the instance: the actor
/// type, its spawn-order id and the call site it was spawned from, so that
/// instrumentation in actor methods nests under the actor task.
///
/// The span is created before the future is spawned, which parents it to
/// whatever span is current where the handle is created. This is why it is a
/// manual wrapper rather than `#[tracing::instrument]`: on an async fn the
/// attribute creates its span at first poll, inside the spawned task, where
/// the creation context is gone.
pub(crate) fn serve<T, M, R>(rx: R, actor: Actor<T>) -> impl Future<Output = ()> + Send
where
    T: Send + Sync + 'static,
    M: Dispatch<T>,
    R: JobReceiver<M>,
{
    let span = tracing::info_span!(
        "actor",
        actor_type = type_name::<T>(),
        actor_id = actor.id,
        spawned_at = %actor.spawned_at,
    );
    run(rx, actor).instrument(span)
}

async fn run<T, M, R>(mut rx: R, mut actor: Actor<T>)
where
    T: Send + Sync + 'static,
    M: Dispatch<T>,
    R: JobReceiver<M>,
{
    let _guard = ExitGuard {
        actor_type: type_name::<T>(),
        actor_id: actor.id,
    };
    while let Some(job) = rx.recv().await {
        job.dispatch(&mut actor).await;
    }
}

//! The actor itself: the value, its identity, and the two ways it is served.

use std::any::type_name;
use std::fmt::{self, Debug};
use std::future::{Future, poll_fn};
use std::panic::Location;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};

use futures_channel::oneshot;
use futures_core::Stream;
use tracing::Instrument;

use crate::blocking::Wait;

/// Names actor instances in spawn order, process-wide, which is what the
/// span's `actor_id` carries.
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// The actor's state, as the generated loop holds it.
///
/// Generated code names this type in the loop's signature and reaches
/// [`respond`](Self::respond) through it. Only [`serve`](Self::serve) and
/// [`serve_blocking`](Self::serve_blocking) make one.
#[doc(hidden)]
pub struct Actor<T> {
    /// The value the actor guards. Generated code borrows it to call methods.
    pub inner: T,
    id: u64,
    spawned_at: &'static Location<'static>,
}

impl<T: Debug> Debug for Actor<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Actor").field("inner", &self.inner).finish()
    }
}

/// The half of a call's reply channel that an async actor holds.
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

/// The actor's half of a call's reply channel, whichever backend made it.
///
/// Both backends answer a call the same way and log the same line when nobody
/// is left to hear it, so [`Actor::respond`] is one function over this trait
/// rather than one function per backend. Only the two reply types implement
/// it.
#[doc(hidden)]
pub trait Answerable<R> {
    /// Hands the value to the caller, reporting whether anyone was left.
    fn answer(self, value: R) -> Result<(), ()>;
}

impl<R> Answerable<R> for Reply<R> {
    fn answer(self, value: R) -> Result<(), ()> {
        self.0.send(value).map_err(|_| ())
    }
}

/// Reads the next job from the stream an async actor is served from.
///
/// Generated code calls this in its loop. It is what `StreamExt::next` would
/// be, without a dependency on `futures-util` for one method.
#[doc(hidden)]
pub async fn next<M, R: Stream<Item = M> + Unpin>(rx: &mut R) -> Option<M> {
    poll_fn(|cx| Pin::new(&mut *rx).poll_next(cx)).await
}

/// Why an actor stopped serving jobs. Reported on the actor's own exit event;
/// a caller only learns that the actor is gone, not which of the two it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActorExit {
    /// A method panicked, unwinding the actor task or thread.
    Panicked,
    /// The loop ended without unwinding, because every handle to it was
    /// dropped or the runtime shut down and cancelled it.
    Stopped,
}

/// Reports the exit reason when the actor ends, however it ends.
///
/// `std::thread::panicking()` is true while a panic unwinds, which is what
/// separates a panicking actor method from a runtime shutdown or a cancelled
/// task - both of which drop the loop without unwinding.
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

impl<T> Actor<T> {
    /// The id and the spawn site name the instance, and both go on the actor
    /// span.
    fn new(inner: T, spawned_at: &'static Location<'static>) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            inner,
            id,
            spawned_at,
        }
    }

    /// The span every call on this actor runs inside.
    ///
    /// It names the actor type, its spawn-order id and the call site it was
    /// built from, so that instrumentation in actor methods nests under the
    /// actor.
    fn span(&self) -> tracing::Span {
        tracing::info_span!(
            "actor",
            actor_type = type_name::<T>(),
            actor_id = self.id,
            spawned_at = %self.spawned_at,
        )
    }

    /// Reports why the actor stopped, whenever the loop ends or unwinds.
    fn exit_guard(&self) -> ExitGuard {
        ExitGuard {
            actor_type: type_name::<T>(),
            actor_id: self.id,
        }
    }

    /// Answers one call.
    ///
    /// The caller is gone whenever it dropped the call before the actor got to
    /// it, which is worth a line because it means the work was wasted.
    pub fn respond<R, A: Answerable<R>>(&self, reply: A, value: R) {
        if reply.answer(value).is_err() {
            tracing::debug!(
                actor_type = type_name::<T>(),
                actor_id = self.id,
                "Actor failed to respond as the receiver is dropped"
            );
        }
    }

    /// Serves the actor as a future, inside the `actor` span that names the
    /// instance and the guard that reports why it stopped.
    ///
    /// The span is created before the future is spawned, which parents it to
    /// whatever span is current where the handle is built. This is why it is
    /// a manual wrapper rather than `#[tracing::instrument]`: on an async fn
    /// the attribute creates its span at first poll, inside the spawned task,
    /// where the creation context is gone.
    ///
    /// `run` is the loop itself, which generated code supplies. It takes the
    /// receiver and the actor by value, so its future borrows nothing from
    /// here and needs neither a trait to name it nor a box to hold it.
    pub fn serve<R, F, Fut>(
        inner: T,
        spawned_at: &'static Location<'static>,
        rx: R,
        run: F,
    ) -> impl Future<Output = ()> + Send
    where
        T: Send + Sync + 'static,
        R: Send + 'static,
        F: FnOnce(R, Self) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send,
    {
        let actor = Self::new(inner, spawned_at);
        let span = actor.span();
        let guard = actor.exit_guard();
        async move {
            let _guard = guard;
            run(rx, actor).await
        }
        .instrument(span)
    }

    /// Serves the actor on the calling thread, inside the same span and guard.
    ///
    /// A thread enters the span rather than instrumenting a future with it.
    /// The guard is declared after the span guard so that it drops first,
    /// putting the exit event inside the span it belongs to.
    pub fn serve_blocking<R, F>(
        inner: T,
        spawned_at: &'static Location<'static>,
        rx: R,
        wait: Wait,
        run: F,
    ) where
        T: Send + 'static,
        F: FnOnce(R, Self, Wait),
    {
        let actor = Self::new(inner, spawned_at);
        let span = actor.span();
        let _entered = span.enter();
        let _guard = actor.exit_guard();
        run(rx, actor, wait)
    }
}

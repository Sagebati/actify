//! The messages an actor is served.
//!
//! A handle turns a call into one of these and sends it down the job channel.
//! [`Dispatch`] is what the actor future runs it with, and because that is a
//! plain trait bound rather than a trait object, a call reaches its method
//! through a direct call.

use std::any::type_name;
use std::fmt::{self, Debug};

use crate::actor::{Actor, ClosureJob, Dispatch, Reply};
use crate::handles::ToView;

/// The calls every handle has, whatever methods its actor declares.
///
/// [`Handle::get`](crate::Handle::get) and [`Handle::set`](crate::Handle::set)
/// send these, and a message type carries them by implementing
/// `From<Builtin<T, V>>`, which is the one piece of glue a generated message
/// type owes the library.
pub enum Builtin<T, V> {
    /// Read the actor's view, as [`Handle::get`](crate::Handle::get) does.
    Get(Reply<V>),
    /// Replace the actor's value, as [`Handle::set`](crate::Handle::set) does.
    Set(T, Reply<()>),
}

impl<T, V> Debug for Builtin<T, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Builtin::Get(_) => "Get",
            Builtin::Set(_, _) => "Set",
        };
        write!(f, "Builtin<{}>::{name}", type_name::<T>())
    }
}

impl<T, V> Dispatch<T> for Builtin<T, V>
where
    T: ToView<V> + Send + Sync + 'static,
    V: Send + 'static,
{
    async fn dispatch(self, actor: &mut Actor<T>) {
        match self {
            Builtin::Get(reply) => {
                let view = actor.inner.to_view();
                actor.respond(reply, view);
            }
            Builtin::Set(val, reply) => {
                actor.inner = val;
                actor.respond(reply, ());
            }
        }
    }
}

/// The message a [`Handle`](crate::Handle) sends when the caller names no
/// other.
///
/// Name it to give a channel its item type, as in
/// `flume::unbounded::<Job<Greeter>>()`.
pub enum Job<T, V = T> {
    /// One of the calls every handle has.
    Builtin(Builtin<T, V>),
    /// A closure, as [`Handle::with`](crate::Handle::with) and
    /// [`Handle::with_mut`](crate::Handle::with_mut) send.
    Closure(ClosureJob<T>),
}

impl<T, V> Debug for Job<T, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Job<{}>", type_name::<T>())
    }
}

impl<T, V> From<Builtin<T, V>> for Job<T, V> {
    fn from(builtin: Builtin<T, V>) -> Self {
        Job::Builtin(builtin)
    }
}

impl<T, V> From<ClosureJob<T>> for Job<T, V> {
    fn from(job: ClosureJob<T>) -> Self {
        Job::Closure(job)
    }
}

impl<T, V> Dispatch<T> for Job<T, V>
where
    T: ToView<V> + Send + Sync + 'static,
    V: Send + 'static,
{
    async fn dispatch(self, actor: &mut Actor<T>) {
        match self {
            Job::Builtin(builtin) => builtin.dispatch(actor).await,
            Job::Closure(job) => job.dispatch(actor).await,
        }
    }
}

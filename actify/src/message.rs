//! The messages an actor is served.
//!
//! A handle turns a call into one of these and sends it down the job channel.
//! [`Dispatch`] is what the actor future runs it with, and because that is a
//! plain trait bound rather than a trait object, a call reaches its method
//! through a direct call.

use std::any::type_name;
use std::fmt::{self, Debug};

use crate::actor::{Actor, Dispatch, Reply};
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
/// other: the calls every handle has, and nothing else.
///
/// An actor with an `#[actify]` block has a message type of its own, generated
/// beside its handle, which carries these as one of its variants. Name this one
/// to give a channel its item type for an actor without such a block, as in
/// `flume::unbounded::<Job<i32>>()`.
pub type Job<T, V = T> = Builtin<T, V>;

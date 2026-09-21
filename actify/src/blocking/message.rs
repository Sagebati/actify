//! The messages a blocking actor is served.
//!
//! The async backend's [`Builtin`](crate::Builtin) with its own reply type.
//! The two cannot be one type: the variants carry two different reply channels
//! (`Reply<V>` and `Reply<()>`), which one type parameter cannot stand for.

use std::any::type_name;
use std::fmt::{self, Debug};

use super::reply::Reply;
use crate::actor::Actor;
use crate::handles::ToView;

/// The calls every blocking handle has, whatever methods its actor declares.
///
/// [`Handle::get`](super::Handle::get) and [`Handle::set`](super::Handle::set)
/// send these, and a message type carries them by implementing
/// `From<Builtin<T, V>>`, which is the one piece of glue a generated message
/// type owes the library.
pub enum Builtin<T, V> {
    /// Read the actor's view, as [`Handle::get`](super::Handle::get) does.
    Get(Reply<V>),
    /// Replace the actor's value, as [`Handle::set`](super::Handle::set) does.
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

/// Runs one of the calls every blocking handle has.
#[doc(hidden)]
pub fn run_builtin<T, V>(actor: &mut Actor<T>, call: Builtin<T, V>)
where
    T: ToView<V> + Send + Sync + 'static,
    V: Send + 'static,
{
    match call {
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

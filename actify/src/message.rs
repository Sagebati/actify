//! The messages an actor is served.
//!
//! A handle turns a call into one of these and sends it down the job channel.
//! The loop that runs them is generated beside the actor's handle, so it is a
//! plain `match` and a call reaches its method directly.

use std::any::type_name;
use std::fmt::{self, Debug};

use crate::actor::{Actor, Reply};
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

/// Runs one of the calls every handle has.
///
/// Generated code calls this for its message's built-in variant. It is a plain
/// function rather than a method on a trait because neither call suspends, so
/// the loop that owns the actor can run it without awaiting.
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

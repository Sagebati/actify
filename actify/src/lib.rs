#![warn(missing_docs, missing_debug_implementations, unreachable_pub)]
#![deny(unused_must_use)]
//! An intuitive actor model for Rust with minimal boilerplate, no manual messages and typed arguments.
//!
//! Actify is a pre-1.0 crate used in production. The API may still change between minor versions.
//!
//! Sharing (mutable) state across async tasks in Rust usually means juggling mutexes and channels,
//! and a lot of boilerplate like hand-written message enums. Actify gives you a typed, async
//! actor model built on [Tokio][tokio] for any struct. Just add `#[actify]` to an `impl` block and call your methods
//! through a clonable [`Handle`].
//!
//! By generating the boilerplate code for you, a few key benefits are provided:
//!
//! * Async actor model built on Tokio and channels
//! * Access to actors through clonable [`Handle`]s
//! * Typed arguments on the methods from your actor, exposed through the handle
//! * No need to define message structs or enums!
//! * Built-in [extension traits] for common standard library types
//!
//! [tokio]: https://docs.rs/tokio/latest/tokio/
//! [extension traits]: #extension-traits
//!
//! # Main functionality of actify!
//!
//! Consider the following example, in which you want to turn your custom Greeter into an actor:
//! ```
//! # use actify::actify;
//! # use std::fmt::Debug;
//! # #[derive(Clone, Debug)]
//! # struct Greeter {}
//! #[actify]
//! impl Greeter {
//!     fn say_hi(&self, name: String) -> String {
//!         format!("hi {}", name)
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() {
//!     // An actify handle is created and initialized with the Greeter struct
//!     let handle = GreeterHandle::new(Greeter {});
//!
//!     // The say_hi method is made available on its handle through the actify! macro
//!     let greeting = handle.say_hi("Alfred".to_string()).await;
//!
//!     // The method is executed on the initialized Greeter and returned through the handle
//!     assert_eq!(greeting, "hi Alfred".to_string())
//! }
//! ```
//!
//! This roughly desugars to:
//! ```
//! # use actify::{Builtin, Dispatch, Handle, actify};
//! # use actify::__private::{Actor, Reply, reply};
//! # #[derive(Clone, Debug)]
//! # struct Greeter {}
//! impl Greeter {
//!     fn say_hi(&self, name: String) -> String {
//!         format!("hi {}", name)
//!     }
//! }
//!
//! // The message a call travels as. One variant per method, holding that
//! // call's arguments and the caller's reply channel, so nothing is boxed
//! // and nothing is downcast.
//! pub enum GreeterCall<V = Greeter> {
//!     Builtin(Builtin<Greeter, V>),
//!     SayHi { name: String, reply: Reply<String> },
//! }
//!
//! // How the actor runs one. A plain trait bound rather than a trait object,
//! // so the call below is direct.
//! impl<V: Send + 'static> Dispatch<Greeter> for GreeterCall<V>
//! where
//!     Greeter: actify::ToView<V>,
//! {
//!     async fn dispatch(self, actor: &mut Actor<Greeter>) {
//!         match self {
//!             GreeterCall::Builtin(builtin) => builtin.dispatch(actor).await,
//!             GreeterCall::SayHi { name, reply } => {
//!                 let result: String = Greeter::say_hi(&actor.inner, name);
//!                 actor.respond(reply, result);
//!             }
//!         }
//!     }
//! }
//!
//! // The handle, whose message type is that enum. A method builds its variant
//! // and waits for the reply.
//! pub struct GreeterHandle(Handle<Greeter, Greeter, GreeterCall>);
//!
//! impl GreeterHandle {
//!     pub async fn say_hi(&self, name: String) -> String {
//!         let (reply, get_result) = reply();
//!         self.0.__call(GreeterCall::SayHi { name, reply }, get_result).await
//!     }
//! }
//!
//! # impl GreeterHandle {
//! #     fn new(val: Greeter) -> Self { GreeterHandle(actify::__private::spawn(val)) }
//! # }
//! #[tokio::main]
//! async fn main() {
//!     let handle = GreeterHandle::new(Greeter {});
//!     let greeting = handle.say_hi("Alfred".to_string()).await;
//!     assert_eq!(greeting, "hi Alfred".to_string())
//! }
//! ```
//!
//! ## Async functions in impl blocks
//! Async functions are fully supported, and work as you would expect:
//! ```
//! # use actify::actify;
//! # use std::fmt::Debug;
//! # #[derive(Clone, Debug)]
//! # struct AsyncGreeter {}
//! #[actify]
//! impl AsyncGreeter {
//!     async fn async_hi(&self, name: String) -> String {
//!         format!("hi {}", name)
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() {
//!     let handle = AsyncGreeterHandle::new(AsyncGreeter {});
//!     let greeting = handle.async_hi("Alfred".to_string()).await;
//!     assert_eq!(greeting, "hi Alfred".to_string())
//! }
//! ```
//!
//! ## Generics in the actor type
//! Generics in the actor type are fully supported, as long as they implement Clone, Debug, Send, Sync and 'static:
//! ```
//! # use actify::actify;
//! # use std::fmt::Debug;
//! # #[derive(Clone, Debug)]
//! struct GenericGreeter<T> {
//!     inner: T
//! }
//!
//! #[actify]
//! impl<T> GenericGreeter<T>
//! where
//!     T: Clone + Debug + Send + Sync + 'static,
//! {
//!     async fn generic_hi(&self, name: String) -> String {
//!         format!("hi {} from {:?}", name, self.inner)
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() {
//!     let handle = GenericGreeterHandle::new(GenericGreeter { inner: usize::default() });
//!     let greeting = handle.generic_hi("Alfred".to_string()).await;
//!     assert_eq!(greeting, "hi Alfred from 0".to_string())
//! }
//!```
//!
//! ## Generics on a method itself
//! A method cannot declare generic parameters of its own. A call travels as a
//! variant of the actor's message enum, and that enum has no such parameter to
//! put the arguments in, so the macro rejects one:
//! ```compile_fail
//! # struct Greeter {}
//! #[actify::actify]
//! impl Greeter {
//!     fn apply<F>(&self, value: usize, f: F) -> usize
//!     where
//!         F: Fn(usize) -> usize + Send + Sync + 'static,
//!     {
//!         f(value)
//!     }
//! }
//! ```
//!
//! A concrete function pointer says the same thing for most callers, and a
//! non-capturing closure coerces to one, so `handle.apply(5, |x| x * 2)` still
//! compiles:
//! ```
//! # use actify::actify;
//! # #[derive(Clone, Debug)]
//! # struct Greeter { }
//! #[actify]
//! impl Greeter {
//!     fn apply(&self, value: usize, f: fn(usize) -> usize) -> usize {
//!         f(value)
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() {
//!     let handle = GreeterHandle::new(Greeter {});
//!     let result = handle.apply(5, |x| x * 2).await;
//!     assert_eq!(result, 10);
//! }
//!```
//!
//! A method's own `where` clause is allowed, and the call carries a pointer to
//! the method so that the bound is proved where the call is made:
//! ```
//! # use actify::actify;
//! # #[derive(Clone, Debug)]
//! # struct Sorter { values: Vec<i32> }
//! #[actify]
//! impl Sorter {
//!     fn sorted(&self) -> Vec<i32>
//!     where
//!         i32: Ord,
//!     {
//!         let mut values = self.values.clone();
//!         values.sort();
//!         values
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() {
//!     let handle = SorterHandle::new(Sorter { values: vec![3, 1, 2] });
//!     assert_eq!(handle.sorted().await, vec![1, 2, 3]);
//! }
//!```
//!
//! ## Passing arguments by reference
//! As referenced arguments cannot be sent to the actor, they are forbidden. All
//! arguments must be owned, and `Send + Sync + 'static`: a job travels through
//! a channel whose sending half a handle shares across tasks, so everything in
//! it has to be shareable too. The same holds for return types.
//! ```compile_fail
//! # struct MyActor {}
//! #[actify::actify]
//! impl MyActor {
//!     fn foo(&self, forbidden_reference: &usize) {
//!         println!("Hello foo: {}", forbidden_reference);
//!     }
//! }
//! ```
//!
//! # Spawning and the job channel
//!
//! [`Handle::builder`] returns the handle and the actor's future, so the
//! caller chooses the executor. Nothing runs until that future is polled.
//!
//! ```
//! # use actify::actify;
//! # #[derive(Clone, Debug)]
//! # struct Greeter {}
//! # #[actify]
//! # impl Greeter {
//! #     fn say_hi(&self, name: String) -> String { format!("hi {name}") }
//! # }
//! #[tokio::main]
//! async fn main() {
//!     let (handle, actor) = GreeterHandle::builder(Greeter {}).build();
//!     tokio::spawn(actor);
//!     let handle = GreeterHandle::from_handle(handle);
//!
//!     assert_eq!(handle.say_hi("Alfred".to_string()).await, "hi Alfred");
//! }
//! ```
//!
//! [`HandleBuilder::channel`] takes the channel itself, so the queue's bound
//! and the crate behind it are the caller's too. The halves go in the order
//! every channel constructor returns them, and anything that is a [`Sink`] and
//! a [`Stream`] qualifies, which covers `flume`, `futures-channel` and, via
//! `tokio-util`, a Tokio `mpsc`:
//!
//! ```
//! # use actify::actify;
//! # #[derive(Clone, Debug)]
//! # struct Greeter {}
//! # #[actify]
//! # impl Greeter {
//! #     fn say_hi(&self, name: String) -> String { format!("hi {name}") }
//! # }
//! #[tokio::main]
//! async fn main() {
//!     let (tx, rx) = futures_channel::mpsc::channel(32);
//!
//!     let (handle, actor) = GreeterHandle::builder(Greeter {}).channel((tx, rx)).build();
//!     tokio::spawn(actor);
//!     let handle = GreeterHandle::from_handle(handle);
//!
//!     assert_eq!(handle.say_hi("Alfred".to_string()).await, "hi Alfred");
//! }
//! ```
//!
//! [`JobSender`] and [`JobReceiver`] are what the two halves have to
//! implement, and the blanket implementations cover every [`Sink`] and
//! [`Stream`] that is `Send`, `Sync`, `Unpin` and `'static`. A channel crate
//! that provides neither can implement them directly.
//!
//! [`Handle::new`] is the Tokio spelling of a build followed by a
//! `tokio::spawn`, kept for the common case.
//!
//! [`Sink`]: https://docs.rs/futures-sink/latest/futures_sink/trait.Sink.html
//! [`Stream`]: https://docs.rs/futures-core/latest/futures_core/stream/trait.Stream.html
//!
//! # Standard methods
//!
//! Every [`Handle`] provides a set of built-in methods that work without the macro:
//!
//! - [`Handle::get`]: returns the actor's view, which for a plain `Clone` actor is a
//!   clone of the value itself
//! - [`Handle::set`]: overwrites the actor value
//!
//! Reading part of an actor without cloning all of it, or changing it in
//! place, is what an `#[actify]` method is for. A handle takes no closures: a
//! call travels to the actor as data, and a closure is not data.
//!
//! # Leaving methods off the handle
//!
//! Every method in an `#[actify]` block gets a handle method. For an inherent
//! impl, a second `impl` block keeps a method off the handle and needs nothing
//! from actify.
//!
//! A trait impl cannot be split that way: Rust requires every method of a trait
//! in one impl block. Mark the method
//! [`#[actify::skip]`](macro@crate::skip) instead:
//!
//! ```
//! # use actify::actify;
//! # #[derive(Clone, Debug)]
//! # struct Ledger { entries: Vec<String> }
//! trait Merge {
//!     fn count(&self) -> usize;
//!
//!     fn merge(&mut self, other: &Ledger);
//! }
//!
//! #[actify]
//! impl Merge for Ledger {
//!     fn count(&self) -> usize {
//!         self.entries.len()
//!     }
//!
//!     // Takes a reference, which an actor call cannot
//!     #[actify::skip]
//!     fn merge(&mut self, other: &Ledger) {
//!         self.entries.extend(other.entries.iter().cloned());
//!     }
//! }
//! ```
//!
//! The method stays on the type, unchanged, and is not checked, so it may take
//! or return references.
//!
//! # Multiple impl blocks
//!
//! Each `#[actify]` block generates a handle struct named `{Type}Handle` and a
//! message enum named `{Type}Call`. Both belong to that one block, and an actor
//! owns one state served through one channel, so every method of an actor has
//! to live in a single block:
//!
//! ```
//! # use actify::actify;
//! # #[derive(Clone, Debug)]
//! struct Counter { value: i32 }
//!
//! #[actify]
//! impl Counter {
//!     fn increment(&mut self) { self.value += 1; }
//!
//!     fn value(&self) -> i32 { self.value }
//! }
//!
//! #[tokio::main]
//! async fn main() {
//!     let handle = CounterHandle::new(Counter { value: 0 });
//!     handle.increment().await;
//!     assert_eq!(handle.value().await, 1);
//! }
//! ```
//!
//! A second block on the same type generates the same two names, which the
//! compiler reports as items defined twice. Splitting a type's methods across
//! blocks that a `#[cfg]` makes mutually exclusive still works, because only
//! one of them ever exists.
//!
//! # ReadHandle
//!
//! A [`ReadHandle`] is a read-only view of an actor. It supports
//! [`get`](ReadHandle::get) but cannot mutate the actor. Obtain one via
//! [`Handle::read_handle`].
//!
//! # Handles for standard library types
//!
//! Actify ships handles for common standard library types, generated from an
//! `#[actify]` block the same way your own are:
//!
//! - [`OptionHandle`] for an `Option<T>` actor
//! - [`VecHandle`] for a `Vec<T>` actor
//! - [`HashMapHandle`] for a `HashMap<K, V>` actor
//! - [`HashSetHandle`] for a `HashSet<K>` actor
//! - [`StringHandle`] for a `String` actor
//! - [`VecDequeHandle`] for a `VecDeque<T>` actor
//!
//! # Views and non-Clone types
//!
//! A handle exposes a view of its actor: the type `V` that [`Handle::get`]
//! returns. By default `V = T`, so [`Handle::new`] requires `T: Clone` and the
//! view is a clone of the value.
//!
//! For a non-Clone type, or to expose a summary instead of the whole value,
//! implement [`ToView<V>`] for a Clone-able `V` and name it explicitly:
//! `Handle::<MyType, Summary>::new(val)`. Reads then return the summary, and an
//! `#[actify]` method is what reaches past it to the actor type itself.
//!
//! # Execution model
//!
//! Each actor is one future, which the caller spawns wherever it likes. It
//! runs one job at a time and takes the next only after the current one
//! returns, so calls never interleave and `&mut self` methods need no lock. A
//! slow method delays every other call to that actor.
//!
//! Jobs queue in the channel the actor was built with. The default is
//! unbounded, so queueing a call never waits and the only thing a call awaits
//! is its reply. A bounded channel, passed to
//! [`HandleBuilder::channel`], is how backpressure is asked
//! for instead.
//!
//! Two actors that call each other never return: each waits for a reply the
//! other can only produce once it is free. The same holds for a method calling
//! its own handle.
//!
//! ```no_run
//! # use actify::actify;
//! #[derive(Clone, Debug)]
//! struct Parser {
//!     store: Option<StoreHandle>,
//! }
//!
//! #[derive(Clone, Debug)]
//! struct Store {
//!     parser: Option<ParserHandle>,
//! }
//!
//! #[actify]
//! impl Parser {
//!     async fn parse(&self) {
//!         self.store.as_ref().unwrap().save().await;
//!     }
//!
//!     async fn is_ready(&self) -> bool {
//!         true
//!     }
//! }
//!
//! #[actify]
//! impl Store {
//!     async fn save(&self) {
//!         // Parser is still inside parse, so this call is never served
//!         self.parser.as_ref().unwrap().is_ready().await;
//!     }
//! }
//! ```
//!
//! # Actor lifetime and panics
//!
//! An actor runs until every [`Handle`] to it is dropped. A [`ReadHandle`]
//! holds a handle internally and keeps the actor alive.
//!
//! A panicking method stops the actor permanently. There is no restart, and
//! every later call on any handle to it panics, reporting that the actor is no
//! longer running. Rust prints the original panic with its message and
//! backtrace, and the actor's own exit event records the panic at ERROR level.
//!
//! Calls after the actor's runtime has shut down panic with the same message:
//! a caller learns only that the actor is gone, since the actor task is the
//! one place the two can be told apart.
//!
//! # Instrumentation
//!
//! Diagnostics are emitted through [`tracing`]. Every actor task runs inside
//! an `actor` span at INFO level that names the instance: `actor_type` is
//! the actor type, `actor_id` a process-wide spawn-order number, and
//! `spawned_at` the [`Handle::new`] call site, captured through
//! `#[track_caller]`. Instrumentation in actor methods nests under the actor
//! serving them. The span is created where the handle is created, which
//! parents it to the span that is current there. A code path that reaches
//! [`Handle::new`] through its own helper reports the helper's caller only
//! if that helper is also `#[track_caller]`.
//!
//! Every actor exit is reported with the reason as a field: at ERROR when a
//! method panicked, at DEBUG when the actor stopped because its handles were
//! dropped or its runtime shut down.
//!
//! Nothing is emitted without a tracing subscriber. A dependent reading
//! diagnostics through the `log` crate can enable the `log` feature instead.
//!
//! # Feature flags
//!
//! - `log`: emits the events as `log` records whenever no tracing subscriber
//!   is set, by forwarding to tracing's own `log` feature. Span fields do not
//!   reach those records, which is why the events name the actor type and id
//!   themselves. Cargo features unify across a build, so enabling it switches
//!   every tracing-using crate in the binary the same way.

/// The README examples, compiled and run as part of the test suite.
///
/// Only present under `cfg(doctest)`, so it costs nothing when building docs.
#[cfg(doctest)]
mod readme {
    #![doc = include_str!("../../README.md")]
}

// Generated code refers to ::actify, which has to resolve inside this crate too.
extern crate self as actify;

mod actor;
mod channel;
mod extensions;
mod handles;
mod message;

// Reexport for easier reference
pub use actify_macros::{actify, skip};
pub use actor::Dispatch;
pub use channel::{Closed, JobReceiver, JobSender};
pub use extensions::{
    map::HashMapHandle, option::OptionHandle, set::HashSetHandle, string::StringHandle,
    vec::VecHandle, vecdeque::VecDequeHandle,
};
pub use handles::{
    DefaultChannel, DefaultReceiver, DefaultSender, Handle, HandleBuilder, ReadHandle, ToView,
};
pub use message::{Builtin, Job};

/// The crate's own items that the [`actify`](macro@crate::actify) macro needs in
/// generated code. Standard library types are named by absolute path instead.
///
/// Not part of the public API. Anything here may change in a patch release;
/// only generated code names it.
#[doc(hidden)]
pub mod __private {
    pub use crate::actor::{Actor, Reply, reply};
    pub use crate::handles::builder;
    #[cfg(feature = "tokio")]
    pub use crate::handles::spawn;
}

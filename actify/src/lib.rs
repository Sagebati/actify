#![warn(missing_docs, missing_debug_implementations, unreachable_pub)]
#![deny(unused_must_use)]
//! An intuitive actor model for Rust with minimal boilerplate, no manual messages and typed arguments.
//!
//! A hard fork of [actify](https://github.com/AvalorAI/actify), taken at 0.9.0.
//! Not published to crates.io, and not tracking upstream.
//!
//! Sharing (mutable) state across async tasks in Rust usually means juggling
//! mutexes and channels, and a lot of boilerplate like hand-written message
//! enums. Actify writes that for you: add `#[actify]` to an `impl` block and
//! call your methods through a clonable handle of the actor's own.
//!
//! By generating the boilerplate code for you, a few key benefits are provided:
//!
//! * Async actor model over any channel, on any executor - or a blocking one
//!   on a thread, with `#[actify(blocking)]` and no runtime at all
//! * Access to actors through clonable, generated handles
//! * Typed arguments on the methods from your actor, exposed through the handle
//! * No need to define message enums: the macro writes one, and a call travels
//!   as a variant of it rather than as a boxed closure
//! * Ready-made [handles] for common standard library types
//!
//! [handles]: #handles-for-standard-library-types
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
//!     let mut handle = GreeterHandle::new(Greeter {});
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
//! # use actify::actify;
//! # use actify::__private::{Actor, Reply, next, reply};
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
//! pub enum GreeterCall {
//!     SayHi { name: String, reply: Reply<String> },
//! }
//!
//! // The actor's loop, which the library wraps in the span and the guard that
//! // report the actor's life. A plain match, so the call below is direct.
//! async fn run_greeter<R>(mut rx: R, mut actor: Actor<Greeter>)
//! where
//!     R: actify::Stream<Item = GreeterCall> + Unpin,
//! {
//!     while let Some(call) = next(&mut rx).await {
//!         match call {
//!             GreeterCall::SayHi { name, reply } => {
//!                 let result: String = Greeter::say_hi(&actor.inner, name);
//!                 actor.respond(reply, result);
//!             }
//!         }
//!     }
//! }
//!
//! // The handle, which owns the sending half of the channel. A method builds
//! // its variant and waits for the reply.
//! pub struct GreeterHandle(actify::Handle<Greeter, GreeterCall>);
//!
//! impl GreeterHandle {
//!     pub fn new(val: Greeter) -> Self {
//!         GreeterHandle(actify::__private::spawn(val, run_greeter))
//!     }
//!
//!     pub async fn say_hi(&mut self, name: String) -> String {
//!         let (reply, get_result) = reply();
//!         self.0.__call(GreeterCall::SayHi { name, reply }, get_result).await
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() {
//!     let mut handle = GreeterHandle::new(Greeter {});
//!     let greeting = handle.say_hi("Alfred".to_string()).await;
//!     assert_eq!(greeting, "hi Alfred".to_string())
//! }
//! ```
//!
//! An actor type needs no derives. Nothing reads it whole and nothing prints
//! it, so `Clone` and `Debug` are yours to add or not; the handle is both
//! regardless.
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
//!     let mut handle = AsyncGreeterHandle::new(AsyncGreeter {});
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
//!     let mut handle = GenericGreeterHandle::new(GenericGreeter { inner: usize::default() });
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
//!     let mut handle = GreeterHandle::new(Greeter {});
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
//!     let mut handle = SorterHandle::new(Sorter { values: vec![3, 1, 2] });
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
//! A generated handle's `builder` returns the handle and the actor's future, so the
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
//!     let (mut handle, actor) = GreeterHandle::builder(Greeter {}).build();
//!     tokio::spawn(actor);
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
//!     let (mut handle, actor) = GreeterHandle::builder(Greeter {}).channel((tx, rx)).build();
//!     tokio::spawn(actor);
//!
//!     assert_eq!(handle.say_hi("Alfred".to_string()).await, "hi Alfred");
//! }
//! ```
//!
//! The sending half is any [`Sink`] over the message type and the receiving
//! half any [`Stream`] of it, which actify re-exports so that naming them
//! needs no dependency on the futures crates. There is no trait of actify's
//! own in between.
//!
//! A generated handle's `new` is the Tokio spelling of a build followed by a
//! `tokio::spawn`, kept for the common case.
//!
//! [`Sink`]: https://docs.rs/futures-sink/latest/futures_sink/trait.Sink.html
//! [`Stream`]: https://docs.rs/futures-core/latest/futures_core/stream/trait.Stream.html
//!
//! # Actors without a runtime
//!
//! `#[actify(blocking)]` generates the same three things with `async` taken out
//! of all of them: the actor's loop is a plain `fn` that a [`std::thread`] runs,
//! and the handle's methods are ordinary calls that return when the actor has
//! answered. There is no executor anywhere, which is what makes it usable from
//! a plain `fn main`, a real-time thread, or a binary that has no runtime to
//! spare.
//!
//! ```
//! use actify::actify;
//!
//! #[derive(Clone, Debug, PartialEq)]
//! struct Counter(i32);
//!
//! #[actify(blocking)]
//! impl Counter {
//!     fn add(&mut self, value: i32) -> i32 {
//!         self.0 += value;
//!         self.0
//!     }
//! }
//!
//! let mut handle = CounterHandle::new(Counter(0));
//! assert_eq!(handle.add(2), 2);
//! assert_eq!(handle.add(3), 5);
//! ```
//!
//! Everything else is as it is above: the same generated message enum, the same
//! handle and builder, the same
//! `#[skip]`, the same two allocations per call, and the same rule that the
//! actor stops when its last handle is dropped. What differs is that `build`
//! hands back a closure rather than a future, that the channel is a blocking
//! one, and that [`Wait`](blocking::Wait) chooses whether the two threads park
//! or busy-wait. An `async fn` in such a block is a compile error, since there
//! is no runtime to drive it.
//!
//! See the [`blocking`] module for the details, and
//! `examples/no_runtime_at_all.rs` for a program that uses nothing else.
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
//!     let mut handle = CounterHandle::new(Counter { value: 0 });
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
//! # Execution model
//!
//! Each actor is one future, which the caller spawns wherever it likes. It
//! runs one job at a time and takes the next only after the current one
//! returns, so calls never interleave and `&mut self` methods need no lock. A
//! slow method delays every other call to that actor.
//!
//! Jobs queue in the channel the actor was built with. The default is
//! unbounded, so queueing a call never waits and the only thing a call awaits
//! is its reply. A bounded channel, passed to [`HandleBuilder::channel`], is
//! how backpressure is asked for instead: a caller then waits for a slot
//! rather than queueing without limit. How much it holds is the channel's own
//! rule - `futures_channel` holds `buffer + one slot per sender` - and a
//! handle is a sender, so a program that clones a handle per task raises its
//! own ceiling.
//!
//! # Handles are `&mut` to call
//!
//! A handle sends through its own sending half, by `&mut`, so one handle
//! carries one call at a time. Cloning it is how two callers proceed at once,
//! and it costs one sending half - a reference count, not a queue.
//!
//! The usual shape is unaffected: a handle cloned into a task is owned there,
//! and an owned handle gives `&mut` for free. What changes is a handle kept in
//! a struct, which a `&self` method can no longer call through:
//!
//! ```
//! # use actify::actify;
//! # #[derive(Clone, Debug)]
//! # struct Counter(i32);
//! # #[actify]
//! # impl Counter {
//! #     fn add(&mut self, value: i32) -> i32 { self.0 += value; self.0 }
//! # }
//! struct Service {
//!     counter: CounterHandle,
//! }
//!
//! impl Service {
//!     // Takes `&mut self`, and calls through the handle it owns.
//!     async fn bump(&mut self) -> i32 {
//!         self.counter.add(1).await
//!     }
//!
//!     // Or keeps `&self`, and clones the handle to get a sending half of
//!     // its own.
//!     async fn peek(&self) -> i32 {
//!         self.counter.clone().add(0).await
//!     }
//! }
//! ```
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
//!         self.store.clone().unwrap().save().await;
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
//!         self.parser.clone().unwrap().is_ready().await;
//!     }
//! }
//! ```
//!
//! # Actor lifetime and panics
//!
//! An actor runs until every handle to it is dropped. Cloning a handle is
//! what keeps it alive from somewhere else; the last clone to go takes the
//! last sending half with it, the channel closes, and the loop returns.
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
//! `spawned_at` the `new` call site, captured through
//! `#[track_caller]`. Instrumentation in actor methods nests under the actor
//! serving them. The span is created where the handle is created, which
//! parents it to the span that is current there. A code path that reaches
//! `new` through its own helper reports the helper's caller only
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
pub mod blocking;
mod extensions;
mod handle;

pub use actify_macros::{actify, skip};
pub use extensions::{
    HashMapHandle, HashSetHandle, OptionHandle, StringHandle, VecDequeHandle, VecHandle,
};
/// The trait an async actor's job channel reads through, re-exported so that
/// naming it needs no dependency on the futures crates.
pub use futures_core::Stream;
/// The trait an async actor's job channel is sent through, re-exported for the
/// same reason.
pub use futures_sink::Sink;
pub use handle::{DefaultChannel, DefaultReceiver, DefaultSender, Handle, HandleBuilder};

/// The crate's own items that the [`actify`](macro@crate::actify) macro needs in
/// generated code. Standard library types are named by absolute path instead.
///
/// Not part of the public API. Anything here may change in a patch release;
/// only generated code names it.
#[doc(hidden)]
pub mod __private {
    pub use crate::actor::{Actor, Reply, next, reply};
    pub use crate::handle::builder;
    #[cfg(feature = "tokio")]
    pub use crate::handle::spawn;
}

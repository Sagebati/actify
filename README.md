# Actify

Actify is a pre-1.0 crate used in production. The API may still change between minor versions.

Sharing (mutable) state across async tasks in Rust usually means juggling mutexes and channels, and a lot of boilerplate like hand-written message enums. Actify gives you a typed, async [actor model](https://en.wikipedia.org/wiki/Actor_model) for any struct. Just add `#[actify]` to an `impl` block and call your methods through a clonable handle of the actor's own.

Actify is runtime-agnostic: an actor is a future the caller spawns, on whatever executor it likes, reading from whatever channel it supplies. A call travels to it as a variant of a generated message enum, carrying its arguments and the caller's reply channel, so nothing on the way is boxed. With `#[actify(blocking)]` there is no runtime at all: the actor is a loop on a `std::thread` and the handle's methods are ordinary blocking calls.

[![Crates.io][crates-badge]][crates-url]
[![License][mit-badge]][mit-url]
[![Docs][docs-badge]][docs-url]

[crates-url]: https://crates.io/crates/actify
[crates-badge]: https://img.shields.io/crates/v/actify.svg
[mit-badge]: https://img.shields.io/badge/license-MIT-blue.svg
[mit-url]: https://github.com/AvalorAI/actify/blob/main/LICENSE
[docs-badge]: https://docs.rs/actify/badge.svg
[docs-url]: https://docs.rs/actify/latest/actify/

## Installation

```sh
cargo add actify
```

## Benefits

By generating the boilerplate code for you, a few key benefits are provided:

- Async actor model that can keep arbitrary owned data types, on any executor.
- The caller spawns the actor and supplies its channel, so neither is the crate's choice.
- [Atomic](https://www.codingem.com/atomic-meaning-in-programming/) access and mutation of underlying data through clonable handles.
- Typed arguments and return values on the methods from your actor, exposed through each handle.
- No need to manually define message enums: the macro writes one, and a call is a variant of it.
- Methods that cannot be actified can stay in the impl block with `#[actify::skip]`.
- Generic type parameters supported on the actor type.
- Ready-made handles for common types: `Vec`, `VecDeque`, `String`, `Option`, `HashMap`, `HashSet`.
- A blocking backend, `#[actify(blocking)]`, for code with no executor to spare.

## Example

Consider the following example, in which you want to turn your custom Greeter into an actor:

```rust
use actify::actify;

#[derive(Clone, std::fmt::Debug)]
struct Greeter {}

#[actify]
impl Greeter {
    fn say_hi(&self, name: String) -> String {
        format!("hi {}", name)
    }
}

#[tokio::main]
async fn main() {
    // The handle is initialized with the Greeter struct, and `actor` is the
    // future that serves it. Spawn it on whatever executor you use.
    let (mut handle, actor) = GreeterHandle::builder(Greeter {}).build();
    tokio::spawn(actor);

    // The say_hi method is made available on its handle through the actify! macro
    let greeting = handle.say_hi("Alfred".to_string()).await;

    // The method is executed remotely on the initialized Greeter and returned through the handle
    assert_eq!(greeting, "hi Alfred".to_string())
}
```

`GreeterHandle::new` is the same three lines, spawning on Tokio, behind the
default `tokio` feature.

## Bring your own channel

The actor reads its calls from a channel the caller owns, so the queue's bound
and the crate behind it are yours to pick. Anything that is a `Sink` and a
`Stream` works, which covers `flume`, `futures-channel` and, through
`tokio-util`, a Tokio `mpsc`:

```rust,ignore
let (tx, rx) = flume::unbounded();

let (mut handle, actor) = GreeterHandle::builder(Greeter {})
    .channel((tx.into_sink(), rx.into_stream()))
    .build();

tokio::spawn(actor);
```

Without a channel the default is an unbounded `futures-channel` queue, so
queueing a call never waits and the only thing a call awaits is its reply. A
bounded channel is how backpressure is asked for instead: a caller waits for a
slot rather than queueing without limit. How much it holds is the channel's own
rule — `futures-channel` holds `buffer + one slot per sender` — and a handle is
a sender.

A handle sends through its own sending half, by `&mut`, so one handle carries
one call at a time; clone it to call from two places at once. An owned handle
gives `&mut` for free, so the usual "clone into each task" shape is unaffected;
a handle kept in a struct needs either a `&mut self` method or a `.clone()`.

See `examples/spawn_it_yourself.rs` for an actor served without Tokio anywhere
in the graph.

## No runtime at all

`#[actify(blocking)]` generates the same message enum, handle and builder with
`async` taken out of all three. The actor's loop is a plain `fn` that a
`std::thread` runs, and the handle's methods block until it answers:

```rust
use actify::actify;

#[derive(Clone, std::fmt::Debug, PartialEq)]
struct Counter(i32);

#[actify(blocking)]
impl Counter {
    fn add(&mut self, value: i32) -> i32 {
        self.0 += value;
        self.0
    }
}

let mut handle = CounterHandle::new(Counter(0));
assert_eq!(handle.add(2), 2);
assert_eq!(handle.add(3), 5);
```

The channel is a blocking one (`std::sync::mpsc` by default, or anything that
implements the two public traits), and `Wait` chooses whether the actor and its
callers park or busy-wait. A call costs one allocation for the reply plus
whatever the channel charges per message, which for a `sync_channel` is nothing
at all. An `async fn` in such a block is a compile error, since nothing is there
to drive it.

See `examples/no_runtime_at_all.rs` for a program that uses nothing else.

For full API documentation, see [docs.rs](https://docs.rs/actify/latest/actify/).

The [changelog](CHANGELOG.md) records every change.

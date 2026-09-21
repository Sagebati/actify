# Changelog

This crate is a hard fork of [actify](https://github.com/AvalorAI/actify),
taken at 0.9.0. Its history before the fork is actify's and lives in that
repository; this file starts at the fork.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

Forked from actify 0.9.0. What is different, and why:

### Changed

- A call travels to its actor as data. `#[actify]` generates a message enum
  with one variant per method, carrying that call's arguments and the
  caller's reply channel. Nothing on the way to the actor is boxed and
  nothing is downcast: a call costs the reply channel and the queue slot,
  two allocations, and a counting allocator pins that exactly.

- The handle, the builder and the actor's loop are generated for each actor.
  There is no generic handle, no handle trait and no dispatch trait: the loop
  is a plain `match` over the actor's own enum, so a call reaches its method
  directly. `#[actify]` is the only way to make an actor.

- A handle is exactly the methods its actor declares. There is no `get`, no
  `set` and no read handle, and an actor type needs no `Clone` or `Debug`.
  The ready-made handles for std types read themselves out through
  `to_vec` and `to_string`.

- A handle's methods take `&mut self`. A handle owns its sending half and is
  cloned by cloning it; concurrent callers hold a handle each. That is what
  lets a bounded channel refuse a caller, and what makes a lost wakeup on a
  `futures_channel` sender unwriteable.

- An async actor's job channel is any `Sink` and `Stream` over its message
  type, both re-exported from the crate root. The caller supplies the
  channel and spawns the actor's future on any executor; `new` is the Tokio
  spelling of that, behind the default `tokio` feature.

- Handles take no closures. A closure is not data, and a call is.

### Added

- A blocking backend, `#[actify(blocking)]`. The actor runs on a
  `std::thread` and the handle's methods are ordinary synchronous calls.
  `Wait` chooses whether the actor and its callers park or busy-wait. The
  reply channel is the `oneshot` crate's; the job channel is `std::sync::mpsc`
  by default, or anything implementing `blocking::JobSender` and
  `blocking::JobReceiver`.

### Removed

Everything that could only be built on one runtime's channels and timers:
throttling, caching, broadcasting, the profiler, and the closure-taking
handle methods.

//! Blocking actors that allocate nothing.
//!
//! `#[actum(blocking, no_alloc)]` is `#[actum(blocking)]` with both of a call's
//! channels moved off the heap. The job queue is a [`Queue`]: a ring buffer
//! the program declares in a `static`, sized in the type. The reply travels
//! through a slot on the caller's own stack. With those two, a call touches
//! the allocator not at all, and a counting allocator can pin that at zero.
//!
//! ```
//! use actum::actum;
//! use actum::blocking::no_alloc::Queue;
//!
//! struct Counter(i32);
//!
//! #[actum(blocking, no_alloc)]
//! impl Counter {
//!     fn add(&mut self, value: i32) -> i32 {
//!         self.0 += value;
//!         self.0
//!     }
//! }
//!
//! // The queue is a static, so the message enum is named. Eight slots; the
//! // size is a power of two, at most 128.
//! static JOBS: Queue<CounterCall, 8> = Queue::new();
//!
//! // There is no default channel: `build` exists only once one is given.
//! let (handle, actor) = CounterHandle::builder(Counter(0))
//!     .channel(JOBS.split())
//!     .build();
//! let running = std::thread::spawn(actor);
//!
//! assert_eq!(handle.add(2), 2);
//! assert_eq!(handle.add(3), 5);
//!
//! drop(handle);
//! running.join().unwrap();
//! ```
//!
//! # Why blocking only
//!
//! The reply slot is a local of the call, and the message carries a pointer
//! to it. That is sound only if the call cannot return before the actor has
//! written through the pointer, and a blocking call cannot: it has nothing to
//! do but wait. An async call is a future, and a future can be dropped or
//! leaked while its message is still in the queue, so it needs the heap for
//! its reply. This is the one place the library uses `unsafe`, in
//! `Slot::split`, and the argument is written beside it.
//!
//! # The queue
//!
//! A [`Queue`] holds `N` jobs and applies backpressure at `N`: a caller whose
//! send finds it full waits for room, by spinning and yielding rather than
//! parking, so size it for the load. It has one receiving half, taken by
//! [`Queue::split`], and as many sending halves as handles are cloned. The
//! actor stops when the last sending half is dropped, and a call finding the
//! receiving half gone reports the actor as gone, both as with any other
//! channel.
//!
//! # What a call costs
//!
//! Nothing on the heap. The message is three words larger than a plain
//! blocking actor's, since its reply half is a reference to the slot, the
//! caller's [`Thread`](std::thread::Thread) to unpark, and a flag, where the
//! heap reply is one word. The slot on the stack is two copies of the return
//! type and two words of bookkeeping.

mod queue;
mod reply;

pub use queue::{Queue, Receiver, Sender};

// What generated code names, and nothing a caller does.
#[doc(hidden)]
pub use reply::{Answer, Reply, Slot};

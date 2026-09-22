//! An actor served on embassy, the embedded async runtime.
//!
//! Embassy is the executor the "any executor" claim is least obviously true
//! for: one thread, tasks allocated from static pools, no `JoinHandle`, and a
//! `main` that never returns. The actor runs there unchanged, and the channel
//! between it and its callers is embassy's own, a static array, so a call's
//! queue slot allocates nothing at all.
//!
//! Run it with `cargo run --example on_embassy`.

use actum::actum;
use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::{Channel, Sender};

const CLIENTS: usize = 2;
const ADDS_EACH: usize = 100;
const QUEUE: usize = 8;

struct Counter(usize);

#[actum]
impl Counter {
    fn add(&mut self, value: usize) -> usize {
        self.0 += value;
        self.0
    }
}

// The job channel, in a static, as embassy channels are. It is bounded: once
// eight calls are queued a client waits for a slot. `CounterCall` is the
// message enum `#[actum]` generates for `Counter`, one variant per method.
static CALLS: Channel<CriticalSectionRawMutex, CounterCall, QUEUE> = Channel::new();

/// The generated handle over embassy's sender rather than the default one.
///
/// A task argument has to be a nameable `'static` type, and this is one.
type Handle = CounterHandle<Sender<'static, CriticalSectionRawMutex, CounterCall, QUEUE>>;

#[embassy_executor::task(pool_size = CLIENTS)]
async fn client(mut handle: Handle, id: usize) {
    for _ in 0..ADDS_EACH {
        // Everything is on one thread, so this `.await` is where the actor
        // gets to run. The reply channel is the one allocation a call makes.
        let total = handle.add(1).await;

        // The actor is the one thing the clients share, so it is also how the
        // last of them knows it is last. Embassy's `main` never returns, so on
        // a host the program ends here.
        if total == CLIENTS * ADDS_EACH {
            println!("client {id} made the last call; total {total}");
            std::process::exit(0);
        }
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let (handle, actor) = CounterHandle::builder(Counter(0))
        .channel((CALLS.sender(), CALLS.receiver()))
        .build();

    // One handle per client. A handle sends through `&mut self`, so it carries
    // one call at a time; concurrency is spelled by cloning.
    for id in 0..CLIENTS {
        spawner.spawn(client(handle.clone(), id).unwrap());
    }

    // The actor's future is opaque, so it cannot be a task argument, but it
    // can be awaited right here: this task is the actor. A static channel has
    // no last sender to drop, so it serves for as long as the executor runs.
    actor.await;
}

//! An actor served on an executor that is not Tokio.
//!
//! The generated handle's `builder` hands back the actor's future instead of
//! spawning it, so the executor is the caller's choice. Here it is
//! `futures_executor`, running the actor on a thread of its own, and the
//! channel between the two is `futures_channel`'s.
//!
//! Run it with `cargo run --example spawn_it_yourself`.

use actify::actify;

#[derive(Clone, Debug)]
struct Greeter {
    greeting: String,
}

#[actify]
impl Greeter {
    fn say_hi(&self, name: String) -> String {
        format!("{} {name}", self.greeting)
    }

    fn shout(&mut self) {
        self.greeting = self.greeting.to_uppercase();
    }
}

fn main() {
    // A bounded channel, so callers wait rather than queue without limit. The
    // default is unbounded; either half can come from any channel crate.
    let channel = futures_channel::mpsc::channel(32);

    let (handle, actor) = GreeterHandle::builder(Greeter {
        greeting: "hi".to_string(),
    })
    .channel(channel)
    .build();

    // Nothing runs until the future is polled, and nothing here is Tokio.
    let served = std::thread::spawn(move || futures_executor::block_on(actor));

    futures_executor::block_on(async {
        println!("{}", handle.say_hi("Alfred".to_string()).await);

        handle.shout().await;
        println!("{}", handle.say_hi("Alfred".to_string()).await);
    });

    // The actor stops once the last handle is gone, which ends its future and
    // with it the thread serving it.
    drop(handle);
    served.join().unwrap();
}
